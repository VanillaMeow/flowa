# Architecture

`main.rs` handles the native entry point, codec registration, and command-line arguments. `app.rs` owns application state and the window lifecycle. Its `app/` modules contain the canvas, toolbar, filmstrip, inspector, keyboard input, dialogs, screenshots, and background-result handling. `preferences.rs` defines one function per preferences tab and the shortcut reference. `theme.rs` maps semantic UI colors to built-in or KDE palettes. `view.rs` contains reversible image-to-screen transforms.

`loader.rs` handles decoding, color conversion, previews, and export. `loader/workers.rs` owns the channels and scheduling for decoding, tiles, and thumbnails.

## Loading and ownership

Folder scanning runs outside the event loop and returns generation-tagged catalogs. The main decode worker serializes full-image decoding, drains queued requests to the latest requested image, and drops stale results. This avoids decoding every intermediate image during rapid navigation. The UI rejects any stale generation.

Decoded RGBA frames are shared through `Arc`. They are not copied just to send them between workers. The tile worker crops and downsamples visible regions, and a separate thumbnail worker loads visible filmstrip entries. Both have bounded request and result queues. They check the active generation before and after expensive work.

GPU resources are created and evicted on the UI thread. The renderer retains one preview texture, a bounded set of detailed tile textures, and a bounded thumbnail cache. It uploads at most four completed tiles per UI pass. Native file dialogs, exports, trash, renames, and clipboard transfers run on a separate action worker. That worker retains clipboard ownership across requests for X11.

## Coordinate system

The image is centered in the canvas. Horizontal/vertical flips apply in source coordinates, followed by clockwise quarter-turn rotation, zoom, and pan. Source-to-screen and screen-to-source transforms are inverses. Pointer-centered zoom adjusts pan so the image point underneath the pointer stays fixed. The actual-pixels action accounts for native display scale.

Each tile has a source rectangle and a texture. The renderer calculates visible source bounds, picks a power-of-two resolution level, and paints transformed quads. A one-sample overlap provides neighboring pixels for bilinear filtering. Painting a tile first restores the canvas/checkerboard in its core region so semitransparent pixels do not blend twice over the underlying preview.

## Persistence and file operations

Settings use a version-tolerant serde model with field defaults and validated ranges. Saves write a temporary sibling before replacing the preferences file. The app persists changed settings on a two-second interval and on exit.

Rotation and flips never modify the original file. Exports encode a temporary sibling and replace the chosen destination after success. The source path cannot be selected as the export destination. Renames reject path traversal, Windows reserved names, and existing destinations. Deletion always requires an in-app confirmation and uses the system trash.

## Scope

The large-image renderer reduces GPU memory pressure and upload latency, but decoding still requires CPU memory for the complete image. Adding true region decoders should preserve the `TileKey`/source-rectangle model while replacing the `Arc<RgbaImage>` source. Full HDR or monitor-specific color management would require a higher-precision texture path and explicit display transforms.
