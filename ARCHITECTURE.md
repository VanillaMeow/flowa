# Architecture

Flowa is one Rust crate. `src/main.rs` handles the native entry point, codec registration, and settings. `src/cli.rs` defines command-line options with clap, validates theme and zoom values, and converts them into application launch options. The source folders distinguish application control, viewer state, GUI code, external work, settings, and general helpers. Each folder has a small `mod.rs` declaring its modules and exposing its entry types.

## Source layout

| Folder | Responsibility | Main files |
| --- | --- | --- |
| `app/` | Application construction and coordination | `application.rs`, `native.rs`, `command.rs`, `command_handler.rs`, `worker_results.rs`, `launch.rs` |
| `viewer/` | Viewing state and rules | `browser.rs`, `catalog.rs`, `image_session.rs`, `playback.rs`, `transform.rs`, `vector.rs` |
| `ui/` | Egui components, styling, input, and drawing | `viewer_ui.rs`, `canvas.rs`, `filmstrip.rs`, `toolbar.rs`, `inspector.rs`, `dialogs.rs`, `preferences.rs`, `keyboard.rs`, `theme.rs`, `style.rs`, `widgets.rs`, `icons.rs` |
| `ui/rendering/` | GPU textures and drawing geometry | `image_renderer.rs`, `icon_cache.rs`, `geometry.rs` |
| `services/` | Filesystem access, codecs, desktop integration, and background workers | `folder_scan.rs`, `image_decode/`, `image_loader/mod.rs`, `worker.rs`, `tiles.rs`, `thumbnails.rs`, `file_actions.rs` |
| `video/` | FFmpeg media ownership, playback workers, and native GPU presentation | `media/`, `player/`, `gpu/`, `native_bindings.rs`, `thumbnail.rs` |
| `settings/` | Configuration values and persistence | `model.rs`, `store.rs` |
| `utils/` | General helpers with no application, viewer, or UI dependencies | `format.rs`, `filename.rs`, `size.rs` |

`app/application.rs` defines `Flowa`, constructs its owned components, coordinates selection changes, and implements the eframe lifecycle. `app/command.rs` defines commands; `app/command_handler.rs` executes them. `app/worker_results.rs` coordinates background results that affect several owners.

`app/native.rs` creates the native window and configures graphics. The saved renderer preference applies at startup. Auto initializes Vulkan alone on Linux and Windows. If graphics initialization fails before the app is created, it retries OpenGL on Linux or DX12 on Windows. Other platforms use eframe's default selection in Auto. A specific renderer uses only that backend, and an explicit `WGPU_BACKEND` overrides the saved preference. Preferences → Performance offers platform-appropriate choices and shows the active renderer separately. Changes take effect after restarting the app.

Launch options remain owned by startup until the graphics context is ready, so a failed attempt cannot start image workers or consume those options. Errors after app creation do not trigger a restart.

`viewer/browser.rs` owns catalog entries, selection, sorting, and scan generations. It receives catalog data from the application; filesystem inspection and background scans run in `services/folder_scan.rs`. `viewer/image_session.rs` defines decoded image data and owns load generations, loading/errors, and the current image. `viewer/playback.rs` owns animation and slideshow clocks. Playback returns repaint delays to the application and does not schedule UI work itself. `viewer/transform.rs` contains reversible image-to-screen transforms.

`settings/model.rs` defines settings, appearance choices, defaults, and validation. `settings/store.rs` handles configuration paths, loading, saving, and change tracking. Color mapping and egui styling live in `ui/theme.rs`. `ui/style.rs` names the shared spacing, margins, radii, sizes, text sizes, and animation timing, and `ui/widgets.rs` holds the section cards, captions, hints, segmented controls, and fade-out helper built from them. Components use these instead of literal numbers.

`utils/format.rs` formats file sizes for the toolbar and inspector. `utils/filename.rs` validates portable filenames for both the rename dialog and file-action service. Image transforms stay in the viewer, and texture-quad drawing stays in `ui/rendering/geometry.rs`.

## UI composition and commands

`ViewerUi` in `ui/viewer_ui.rs` constructs the persistent UI components. Each component owns its state and receives temporary borrows of the data needed to draw a pass. Components do not store references to `Flowa` or to one another.

| Owner | Responsibility |
| --- | --- |
| `ImageCanvas` in `ui/canvas.rs` | View transforms, pointer interaction, canvas bounds, and welcome/loading/error displays |
| `ImageRenderer` in `ui/rendering/image_renderer.rs` | Preview texture, detailed tile cache, tile worker, upload limits, and painting |
| `Filmstrip` in `ui/filmstrip.rs` | Thumbnail worker, bounded thumbnail cache, virtualized layout, and selection scrolling |
| `PreferencesWindow` in `ui/preferences.rs` | Open state, selected tab, and editing settings |
| `Dialogs` in `ui/dialogs.rs` | Rename/trash confirmations and temporary notices |

Toolbar, status, context-menu, and inspector rendering remain functions with explicit inputs. They have no persistent application-owned state of their own. Whether the inspector is open lives in `ViewerUi` and is not saved, so each session starts with it closed unless `--info` is passed. `ui/keyboard.rs` translates key events into commands.

Components collect commands in a local vector; `Flowa` applies them after each panel releases its borrows, before drawing dependent panels. `Command` groups them by area into `FileCommand`, `BrowseCommand`, `ViewCommand`, `PlaybackCommand`, and `WindowCommand`, and `app/command_handler.rs` has one handler per area. Each `Action` in `ui/actions.rs` pairs a command with its label and shortcuts, and its `button`, `menu`, and `consume` helpers queue that command, so toolbar buttons, menus, and keys never restate it. Every input source uses the same handlers. Continuous canvas dragging and pointer zoom update the canvas's own view directly.

Rename/trash confirmations carry the path captured when the dialog opened. The command handler dispatches native work through the `services::file_actions::Task` channel; each task sets the busy status that its result clears. Opening the containing folder launches the file manager directly without waiting. UI commands are synchronous and do not use a separate message bus.

## Control icons

`ui/icons.rs` maps semantic control names to bundled [Lucide SVGs](assets/icons/lucide/README.md), retaining the button, tooltip, and accessibility helpers. The Flowa flower logo remains its own vector artwork: `assets/flowa.svg` is the desktop icon, and `assets/flowa-mark.svg` is the same flower without its backdrop, rendered through the icon cache so its center stays transparent. `ui/rendering/icon_cache.rs` rasterizes SVGs with the existing resvg dependency and stores the resulting textures in egui's context-owned cache.

Icon textures are keyed by glyph and physical pixel size. Monochrome SVGs become white premultiplied-alpha masks; the painter applies the current text, accent, or disabled color without rerasterizing. Bitmap rectangles align to device pixels. Cache lookup/insertion happens under egui's memory lock, while rasterization and texture upload happen after that lock is released.

The cache holds at most 64 entries and releases icons unused for 120 frames. Failed rasterizations are cached too. Assets are embedded at compile time and require no runtime file or network access. The pinned upstream revision and full ISC/MIT notices are stored under `assets/icons/lucide`; the license text is also available in Preferences → About → Icon licenses.

## Readability conventions

Separate logical steps with blank lines. Egui closures establish layout scopes and call focused helpers for panel contents, item painting, and input handling. Preserve the existing scope structure and widget order when extracting helpers, because egui uses them for IDs, focus, and layout.

The filmstrip separates viewport calculation, item interaction, thumbnail painting, captions, outlines, and worker requests. The canvas separates pointer gestures, image fitting, rendering, and its empty states. Dialog forms and notification rows have their own helpers. The application frame handler calls named update, drawing, and maintenance phases in execution order.

Modules declare how far they reach: `pub(crate) mod`, or a private `mod` with `pub(crate) use` re-exports. Items inside use plain `pub`, or `pub(super)` and `pub(in …)` to stay within a subsystem, and never repeat `pub(crate)`.

## Loading and ownership

Folder scanning runs outside the event loop and returns generation-tagged catalogs. The main decode worker serializes full-image decoding, drains queued requests to the latest requested image, and drops stale results. This avoids decoding every intermediate image during rapid navigation. The UI rejects any stale generation.

`services/image_loader/mod.rs` also preloads the next and previous images after a selection finishes loading. Hovering a thumbnail or navigation button puts that image ahead of queued neighbors. Selected loads have priority, and a click on an image being preloaded adopts the running decode. A codec call already in progress finishes before the worker handles another image.

The loader retains at most four decoded images in CPU memory, within a quarter of the configured image limit and at most 512 MiB. Header checks restrict speculative decoding to static JPEG, PNG, WebP, BMP, and QOI images. Each candidate needs two estimates: the pixels, preview, and thumbnail it will keep must fit the cache, and its peak decoding memory must fit the image limit. The worker evicts cached images to make room for the retained size before decoding. Codec internals can use additional temporary memory, so this is not a process-wide RAM limit. File size and modification time validate cache hits. Changing the selection or folder keeps cached and in-flight preloads; refreshes, renames, and deletions invalidate them.

## Startup

`app/native.rs` starts the loader before creating the graphics device and preloads the file being opened, so a large image decodes while Vulkan initializes. The application adopts that decode when it selects the file. Previews, thumbnails, and zoom tiles are downscaled with SIMD box filtering in `services/resize.rs`; thumbnails come from the preview rather than the full image.

`video/gpu/shared_device.rs` creates the shared Vulkan device with only the queues flowa uses and imports it into libplacebo, because libplacebo creates every queue of every family, which costs over 100 ms on some NVIDIA drivers. If wgpu requests a feature structure the importer does not recognize, it falls back to libplacebo's own device creation. Hardware decoder detection creates CUDA and VA-API devices, so it runs on a background thread the first time Preferences needs the list.

During navigation, the canvas retains only the previous preview texture and its view transform. This frozen preview is separate from the selected image and cannot be copied or exported. The loading indicator waits 150 ms and fades in over 100 ms. The new image appears as soon as it is available, with no minimum indicator duration or image fade.

Decoded RGBA frames are shared through `Arc`. They are not copied just to send them between workers. The tile worker in `services/tiles.rs` crops and downsamples visible regions, and a separate thumbnail worker in `services/thumbnails.rs` loads visible filmstrip entries.

`services/worker.rs` holds what background threads share. `spawn` names every thread `flowa-…`. `Worker` handles requests in order on one thread with bounded queues, wakes the UI with each result, and refuses requests instead of blocking when full. Owners keep a `Generation` they advance when they replace their image or folder; `Worker::spawn_tagged` skips requests from other generations before and after the work. The tile, thumbnail, and file action workers use `Worker`. The image loader keeps its own loop for its priority queue, and folder scans run one thread each so a slow folder never delays a newer one, but both use the same `Generation` and thread naming. A request to a stopped worker is logged; a result nobody receives is dropped.

GPU resources are created and evicted on the UI thread. The image renderer retains one preview texture and a bounded set of detailed tile textures. The filmstrip owns its separate bounded thumbnail cache. Each component uploads at most four completed results per UI pass. Native file dialogs, exports, trash, renames, and clipboard transfers run on a separate action worker. That worker retains clipboard ownership across requests for X11.

## Video ownership

`video/media/` owns FFmpeg demuxing, stream snapshots, packets, codecs, audio
resampling, decoded frames, and bounded poster extraction. It calls bindings
generated from the SDK headers that the build links.
`video/native_bindings.rs` holds bindings for libplacebo and SDL3 audio, generated from
`native/video/bindings.h`, so struct layouts come from the C headers rather than
hand-written mirrors. `video/vulkan.rs` gives both sets of bindings ash's Vulkan
types.

`build.rs` sets the `video_playback` cfg when the `video` feature targets Linux or
Windows. Without it, `video/unsupported.rs` supplies an uninhabited player, so
`VideoSession` has one implementation that reports playback as unavailable.
A video with an audio track still plays, silently and timed by the wall clock, when
no audio output device can be opened.
Input owners retain stable interruption callbacks through close. Codec owners
retain stable hardware-format callback storage through codec-worker shutdown.

CPU frames have a checked transferable type that rejects hardware pixel formats.
The software worker owns its codec and transfers CPU frame ownership without a
rendering-device dependency. Audio blocks use Rust-owned sample buffers. Immutable
stream snapshots own copied parameters and can outlive the demuxer.

`video/player/decoder.rs` defines the `Decoder` trait: packets in, frames out,
without blocking playback. `HardwareDecoder` in `hardware_decoder.rs` decodes on the
rendering GPU, and `SoftwareDecoder` in `software_decoder.rs` on a CPU thread.
`DecoderSelection` opens the first backend that works and replaces it with the
next one when it fails. `DecoderDetection` lists the hardware decoders for
Preferences.

`HardwareDecoder` and the frames it returns keep the Vulkan device owner in
`video/gpu/mod.rs` alive. That owner holds the device, which never changes after
creation, and a locked renderer used by one playback owner.
`video/gpu/frame_renderer.rs` drives libplacebo and keeps in-flight frame
references until their timeline value completes.
`video/gpu/frame_transfer.rs` copies NVDEC frames and Vulkan decode images into
sampleable planes when direct import is unavailable, without CPU readback. It
retires copy command buffers after GPU completion.
`native/video/placebo.c` is the only C: frame import, which libplacebo implements
in a header, and the libplacebo macros and exported data Rust cannot link.
The player preserves the existing queues, clocks, seek generations, and fallback
behavior. Its current presenter still requires Vulkan even for CPU decoding.

The shared Vulkan path is enabled in source for Linux and Windows; only Linux
has native runtime validation. Metal presentation and additional renderer paths
remain in the [backend roadmap](native/video/BACKENDS.md).

## Coordinate system

The image is centered in the canvas. Horizontal/vertical flips apply in source coordinates, followed by clockwise quarter-turn rotation, zoom, and pan. Source-to-screen and screen-to-source transforms are inverses. Pointer-centered zoom adjusts pan so the image point underneath the pointer stays fixed. The actual-pixels action accounts for native display scale.

Each tile has a source rectangle and a texture. The renderer calculates visible source bounds, picks a power-of-two resolution level, and paints transformed quads. A one-sample overlap provides neighboring pixels for bilinear filtering. Painting a tile first restores the canvas/checkerboard in its core region so semitransparent pixels do not blend twice over the underlying preview.

## Decoding

`services/image_decode/` exposes `decode`, `thumbnail`, and `preload_estimate`. An `ImageFile` picks the decoder by format: `RasterFile` in `raster.rs` or `VectorFile` in `vector.rs`, each wrapping one path and its memory `Budget`. `color.rs` converts embedded ICC profiles to sRGB, and `export.rs` writes transformed copies. Services depend on viewer types such as `DecodedImage` and `View`, never the reverse.

Pixel sizes are `[u32; 2]` outside the UI, which uses egui's `Vec2`. `utils/size.rs` fits sizes into boxes, either shrinking only or scaling both ways, and `services/resize.rs` resamples pixels.

`VectorFile` parses SVGs into a `viewer/vector.rs` `VectorImage`, which renders any region at any scale. A decoded SVG keeps it beside pixels at the intrinsic size, which copying and exporting use. Its preview and thumbnail are rendered to fill their full size, and fitting always enlarges it. A `TileSource` is either raster pixels, which stop adding detail at one sample per pixel, or a `VectorImage`, which renders tiles at finer levels past 100% zoom so SVGs stay sharp at any magnification.

## Persistence and file operations

Settings use a version-tolerant serde model with field defaults and validated ranges. The ranges are constants in `settings/model.rs` shared with the preference controls. Saves write a temporary sibling before replacing the preferences file. The app persists changed settings on a two-second interval and on exit.

The recent list holds files and folders opened explicitly, through dialogs, drag and drop, the command line, or the welcome screen. Browsing within a folder does not change it. The file on screen at exit is saved separately so "Reopen the last image" returns to it.

Rotation and flips never modify the original file. Exports encode a temporary sibling and replace the chosen destination after success. The source path cannot be selected as the export destination. Renames reject path traversal, Windows reserved names, and existing destinations. Deletion always requires an in-app confirmation and uses the system trash.

## Scope

The large-image renderer reduces GPU memory pressure and upload latency, but decoding still requires CPU memory for the complete image. Adding true region decoders should preserve the `TileKey`/source-rectangle model while replacing the `Arc<RgbaImage>` source. Full HDR or monitor-specific color management would require a higher-precision texture path and explicit display transforms.
