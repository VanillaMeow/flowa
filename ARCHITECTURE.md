# Architecture

Flowa is one Rust crate. `src/main.rs` handles the native entry point, codec registration, and command-line arguments. The source folders distinguish application control, viewer state, GUI code, external work, settings, and general helpers. Each folder has a small `mod.rs` declaring its modules and exposing its entry types.

## Source layout

| Folder | Responsibility | Main files |
| --- | --- | --- |
| `app/` | Application construction and coordination | `application.rs`, `command.rs`, `command_handler.rs`, `worker_results.rs`, `launch.rs`, `capture.rs` |
| `viewer/` | Viewing state and rules | `browser.rs`, `catalog.rs`, `image_session.rs`, `playback.rs`, `transform.rs` |
| `ui/` | Egui components, styling, input, and drawing | `viewer_ui.rs`, `canvas.rs`, `filmstrip.rs`, `toolbar.rs`, `inspector.rs`, `dialogs.rs`, `preferences.rs`, `keyboard.rs`, `theme.rs`, `icons.rs` |
| `ui/rendering/` | GPU textures and drawing geometry | `image_renderer.rs`, `icon_cache.rs`, `geometry.rs` |
| `services/` | Filesystem access, codecs, desktop integration, and background workers | `folder_scan.rs`, `image_decode.rs`, `image_workers.rs`, `file_actions.rs` |
| `settings/` | Configuration values and persistence | `model.rs`, `store.rs` |
| `utils/` | General helpers with no application, viewer, or UI dependencies | `format.rs`, `filename.rs` |

`app/application.rs` defines `Flowa`, constructs its owned components, coordinates selection changes, and implements the eframe lifecycle. `app/command.rs` defines commands; `app/command_handler.rs` executes them. `app/worker_results.rs` coordinates background results that affect several owners. Development screenshot and smoke-run state lives in `app/capture.rs`.

`viewer/browser.rs` owns catalog entries, selection, sorting, and scan generations. It receives catalog data from the application; filesystem inspection and background scans run in `services/folder_scan.rs`. `viewer/image_session.rs` defines decoded image data and owns load generations, loading/errors, and the current image. `viewer/playback.rs` owns animation and slideshow clocks. Playback returns repaint delays to the application and does not schedule UI work itself. `viewer/transform.rs` contains reversible image-to-screen transforms.

`settings/model.rs` defines settings, appearance choices, defaults, and validation. `settings/store.rs` handles configuration paths, loading, saving, and change tracking. Color mapping and egui styling live in `ui/theme.rs`.

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

Toolbar, status, context-menu, and inspector rendering remain functions with explicit inputs. They have no persistent application-owned state of their own. `ui/keyboard.rs` translates key events into commands.

Components collect commands in a local vector; `Flowa` applies them after each panel releases its borrows, before drawing dependent panels. Navigation, sorting, view commands, file operations, and window toggles use the same handlers regardless of their input source. Continuous canvas dragging and pointer zoom update the canvas's own view directly.

Rename/trash confirmations carry the path captured when the dialog opened. The command handler dispatches native work through the `services::file_actions::Task` channel. UI commands are synchronous and do not use a separate message bus.

## Control icons

`ui/icons.rs` maps semantic control names to bundled [Lucide SVGs](assets/icons/lucide/README.md), retaining the button, tooltip, and accessibility helpers. The Flowa flower logo remains its own vector artwork. `ui/rendering/icon_cache.rs` rasterizes SVGs with the existing resvg dependency and stores the resulting textures in egui's context-owned cache.

Icon textures are keyed by glyph and physical pixel size. Monochrome SVGs become white premultiplied-alpha masks; the painter applies the current text, accent, or disabled color without rerasterizing. Bitmap rectangles align to device pixels. Cache lookup/insertion happens under egui's memory lock, while rasterization and texture upload happen after that lock is released.

The cache holds at most 64 entries and releases icons unused for 120 frames. Failed rasterizations are cached too. Assets are embedded at compile time and require no runtime file or network access. The pinned upstream revision and full ISC/MIT notices are stored under `assets/icons/lucide`; the license text is also available in Preferences → About → Icon licenses.

## Readability conventions

Separate logical steps with blank lines. Egui closures establish layout scopes and call focused helpers for panel contents, item painting, and input handling. Preserve the existing scope structure and widget order when extracting helpers, because egui uses them for IDs, focus, and layout.

The filmstrip separates viewport calculation, item interaction, thumbnail painting, captions, outlines, and worker requests. The canvas separates pointer gestures, image fitting, rendering, and its empty states. Dialog forms and notification rows have their own helpers. The application frame handler calls named update, drawing, and maintenance phases in execution order.

## Loading and ownership

Folder scanning runs outside the event loop and returns generation-tagged catalogs. The main decode worker serializes full-image decoding, drains queued requests to the latest requested image, and drops stale results. This avoids decoding every intermediate image during rapid navigation. The UI rejects any stale generation.

Decoded RGBA frames are shared through `Arc`. They are not copied just to send them between workers. The tile worker crops and downsamples visible regions, and a separate thumbnail worker loads visible filmstrip entries. Both have bounded request and result queues. They check the active generation before and after expensive work.

GPU resources are created and evicted on the UI thread. The image renderer retains one preview texture and a bounded set of detailed tile textures. The filmstrip owns its separate bounded thumbnail cache. Each component uploads at most four completed results per UI pass. Native file dialogs, exports, trash, renames, and clipboard transfers run on a separate action worker. That worker retains clipboard ownership across requests for X11.

## Coordinate system

The image is centered in the canvas. Horizontal/vertical flips apply in source coordinates, followed by clockwise quarter-turn rotation, zoom, and pan. Source-to-screen and screen-to-source transforms are inverses. Pointer-centered zoom adjusts pan so the image point underneath the pointer stays fixed. The actual-pixels action accounts for native display scale.

Each tile has a source rectangle and a texture. The renderer calculates visible source bounds, picks a power-of-two resolution level, and paints transformed quads. A one-sample overlap provides neighboring pixels for bilinear filtering. Painting a tile first restores the canvas/checkerboard in its core region so semitransparent pixels do not blend twice over the underlying preview.

## Persistence and file operations

Settings use a version-tolerant serde model with field defaults and validated ranges. Saves write a temporary sibling before replacing the preferences file. The app persists changed settings on a two-second interval and on exit.

Rotation and flips never modify the original file. Exports encode a temporary sibling and replace the chosen destination after success. The source path cannot be selected as the export destination. Renames reject path traversal, Windows reserved names, and existing destinations. Deletion always requires an in-app confirmation and uses the system trash.

## Scope

The large-image renderer reduces GPU memory pressure and upload latency, but decoding still requires CPU memory for the complete image. Adding true region decoders should preserve the `TileKey`/source-rectangle model while replacing the `Arc<RgbaImage>` source. Full HDR or monitor-specific color management would require a higher-precision texture path and explicit display transforms.
