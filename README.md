# flowa

A native image viewer built with Rust, egui, and wgpu. Linux is the primary platform. The core viewer also builds for Windows.

Flowa opens directly onto the image, with a minimal toolbar, an optional thumbnail strip, and optional image details. Your files stay untouched until you explicitly rename, export, or move one to the system trash.

## Run

Rust 1.99 or later is required.

```sh
cargo run --release
cargo run --release -- /path/to/image.png
cargo run --release -- /path/to/folder
```

On Arch Linux, install the build and codec dependencies:

```sh
sudo pacman -S --needed rust pkgconf clang cmake wayland libxkbcommon dav1d libheif
```

On Debian/Ubuntu, use the distribution equivalents, including `build-essential`, `pkg-config`, `libclang-dev`, `libwayland-dev`, `libxkbcommon-dev`, `libdav1d-dev`, and `libheif-dev`. HEIF support requires libheif 1.17 or newer. Little CMS is built from bundled source. No GTK or Qt libraries are linked into Flowa. File dialogs use the desktop portal; the desktop supplies its own dialog implementation.

The default build includes AVIF through dav1d and HEIC/HEIF through libheif. The latter needs an installed HEVC decoder plugin, usually libde265, for HEIC images. To build the portable core without these native codec dependencies:

```sh
cargo build --release --no-default-features
```

On Windows, install the Rust MSVC toolchain and Visual Studio C++ build tools, then use that core build command. The UI, JPEG XL, SVG, common image formats, file dialogs, clipboard, and trash support remain available. Enabling AVIF/HEIF also requires their native libraries and pkg-config metadata. A Windows core build is included in CI; a Windows executable has not been tested on this Linux development machine.

## Daily use

- Open an image or folder from the File menu, command line, or drag and drop. Opening one image also loads its supported siblings in natural filename order.
- Scroll to zoom around the pointer. Drag to pan. Double-click or press `0` to fit. Small images are not enlarged by default. Press `1` for one image pixel per physical screen pixel.
- Press Left/Right to navigate, Home/End to jump, `T` for thumbnails, and `I` for metadata. Folder sorting is in the image context menu.
- Press F5 to refresh the folder and reload the current image.
- Press `R` to rotate, Shift+R to rotate backward, and `H`/`V` to flip. These are view transforms. Export a copy to save them.
- Copy the displayed frame with Ctrl+C, copy its path with Ctrl+Shift+C, and export with Ctrl+Shift+S. Exports support PNG, JPEG, WebP, BMP, and TIFF. Exports are 8-bit SDR copies and do not preserve source metadata. JPEG drops transparency.
- Rename with F2. Delete opens a confirmation before moving the file to the system trash. There is no permanent-delete action.
- Press Space to pause an animation or start/stop a slideshow for a still image. Slideshow timing is configurable.
- Press F or F11 for fullscreen. Controls stay hidden until the pointer reaches the top edge. Escape leaves fullscreen or closes the active dialog.
- Ctrl+, opens Preferences. All shortcuts are listed there.

## Appearance

The default System / Qt appearance reads KDE's `kdeglobals` color roles, including the selection accent. It checks for changes every two seconds. This follows the Qt color palette, not the Qt widget style engine. On other desktops, Flowa follows the OS light/dark preference.

Twelve appearances are available: System / Qt, Charcoal, Midnight, Nord, Catppuccin, Dracula, Forest, OLED, Paper, Sand, Latte, and High contrast. Twelve accent choices can be combined with those themes. The canvas can follow the theme or use a separate neutral/custom color. A configurable checkerboard is available behind image transparency and is disabled by default.

Preferences and recent paths are local. On Linux they live in `$XDG_CONFIG_HOME/flowa/settings.json`, normally `~/.config/flowa/settings.json`. Flowa makes no network requests and does not index your home directory. It scans only the folder you open.

Control icons use bundled [Lucide](https://lucide.dev/) SVGs and follow the selected theme. They render locally and work offline. Icon license notices are available in Preferences → About → Icon licenses and in [assets/icons/lucide/LICENSE](assets/icons/lucide/LICENSE).

## Image support

The core build reads JPEG, PNG/APNG, GIF, WebP, BMP, ICO, TIFF, TGA, DDS, QOI, PNM, farbfeld, OpenEXR, Radiance HDR, SVG/SVGZ, and JPEG XL. Default builds add AVIF and HEIC/HEIF.

GIF, APNG, and animated WebP play with their frame delays. Animation buffering has a configurable memory budget and a 2000-frame limit. JPEG XL and HEIF containers currently show their primary/first image. SVGs rasterize at intrinsic dimensions, capped at 16384 pixels; scripts and external image references are not executed or loaded.

EXIF orientation is applied. Supported photographic EXIF fields appear in the details panel. Embedded RGB ICC profiles are converted to sRGB. The current rendering pipeline displays 8-bit SDR pixels, including for higher-bit-depth inputs. It does not implement monitor-specific ICC transforms, HDR output, camera RAW development, or layered PSD decoding.

## Large images

Decoding and downsampling happen on background workers. A 2048-pixel preview appears before detailed tiles upload. The viewport chooses resolution levels according to zoom and screen scale and requests only visible 1024-pixel tiles, prioritizing the center. Tile uploads are spread across frames, and the GPU cache is bounded to approximately 192 MB. Tiles overlap at the edges to avoid filtering seams. The thumbnail strip is virtualized and keeps at most 128 thumbnails.

The full decoded image is still held in CPU memory. This is not an out-of-core decoder for images larger than RAM. The default decoded-image budget is 1024 MiB and is adjustable from 128 to 4096 MiB. Decoder scratch buffers, orientation/color conversion, export copies, and cached tiles add overhead. A 16000 × 10000 RGBA image alone uses about 610 MiB. Images beyond the budget report an error instead of attempting an unlimited allocation. During animation playback the viewer uses previews; paused frames can resolve into detailed tiles.

## Build and install locally

```sh
cargo build --release
./packaging/linux/install-local.sh
```

The installer copies the executable to `~/.local/bin`, adds a desktop launcher and SVG icon, and leaves default file associations unchanged. Native codec libraries still need to be installed. `./packaging/linux/uninstall-local.sh` removes those installed files and preserves preferences.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo check --no-default-features
```

Generate local test images, including a 160-megapixel image, with:

```sh
cargo run --example make_fixtures -- artifacts/fixtures --huge
```

A native screenshot smoke test uses the actual wgpu window. It writes only the requested screenshot and does not persist preferences:

```sh
cargo run -- --smoke-test --screenshot artifacts/viewer.png --filmstrip artifacts/fixtures/01-dunes.png
cargo run -- --smoke-test --preferences --theme nord --screenshot artifacts/preferences.png
cargo run --release -- --smoke-test --zoom 100 --screenshot artifacts/huge.png artifacts/fixtures/07-huge-160mp.png
```

The `--theme` override is session-only. `--smoke-test` and `--screenshot` are development switches. See [ARCHITECTURE.md](ARCHITECTURE.md) for the rendering and ownership model and [VALIDATION.md](VALIDATION.md) for recorded checks.
