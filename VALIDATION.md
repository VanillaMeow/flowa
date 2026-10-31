# Validation

Recorded on 2026-10-31 on Arch Linux with Rust 1.99.1. Native rendering used an NVIDIA GeForce GTX 1050 Ti through Vulkan.

## UI copy cleanup

Removed decorative slogans, duplicate instructions, and implementation commentary from the viewer and preferences. The status bar is hidden until an image has decoded. `cargo fmt --check`, `cargo clippy -j 3 --all-targets -- -D warnings`, `cargo test -j 3`, and the debug build passed; all 30 tests passed. Native Vulkan captures verified the empty screen, failed-image screen, loaded-image status bar, inspector, preferences, and trash cancellation. Checks used fixtures and isolated settings under `/tmp/flowa-copy-ivx3av8v`.

## Lucide control icons

The Lucide replacement on 2026-10-31 passed `cargo fmt --check`, `cargo clippy -j 3 --all-targets -- -D warnings`, `cargo test -j 3`, `cargo check -j 3 --no-default-features`, and `cargo build -j 3`. All 30 tests passed. The three new tests cover rasterization of all 18 embedded SVGs at multiple sizes, premultiplied-alpha masks suitable for tinting, texture reuse across frames, separate entries for different display sizes, the 64-entry cache bound, and eviction after disuse.

Native Vulkan captures were inspected in Nord and Paper, including disabled controls, selected accent swatches, thumbnail placeholders, and 150% UI scale. X11 interaction checks passed for keyboard and toolbar navigation, filmstrip selection, wheel zoom, drag panning, rotation, rename, trash cancellation and confirmation, clearing the last image, preferences, fullscreen, context menus, and clean exit. A separate check verified animation playback, pause, resume, and isolated settings persistence.

Assets and the full upstream license are embedded in the executable. No dependencies were added. Native checks used copied fixtures and isolated configuration/trash directories under `/tmp/flowa-lucide-pv8d_2mp`. This pass rebuilt the debug executable; release measurements below remain historical.

## Readability and UI nesting

The readability pass on 2026-10-31 split deeply nested filmstrip, canvas, dialog, inspector, toolbar, and preferences code into focused helpers. Layout scopes and widget order were retained. Application updates, UI composition, and maintenance now have named phases. Semantic blank lines separate setup, guards, calculations, worker requests, and results throughout application, service, and viewer code. `AGENTS.md` records these conventions for future edits.

`cargo fmt --check`, `cargo clippy -j 3 --all-targets -- -D warnings`, `cargo test -j 3`, and `cargo check -j 3 --no-default-features` passed. All 27 existing tests passed, including rename submission with Enter, modal shortcut suppression, selection resets, and stale worker-result rejection.

The debug build passed. Native viewer, Preferences, and full-resolution transparent-tile screenshots matched their pre-cleanup captures pixel for pixel. X11 checks passed for wheel zoom, drag panning, keyboard and toolbar navigation, filmstrip selection, rotation, rename with Enter, trash cancellation/confirmation, clearing the last image, preferences, fullscreen, and context menus. Animation playback/pause/resume and settings persistence also passed. File operations used copied fixtures and an isolated trash/configuration directory under `/tmp/flowa-readability-1mkgxflp`.

## Source layout and shared utilities

The source layout reorganization on 2026-10-31 passed `cargo fmt --check`, `cargo clippy -j 3 --all-targets -- -D warnings`, `cargo test -j 3`, `cargo check -j 3 --no-default-features`, and `cargo build -j 3`. All 27 existing tests passed after moving alongside their owning modules. Filename validation is now tested under `utils::filename`, folder scanning under `services::folder_scan`, and settings round trips under `settings::store`.

Native Vulkan rendering checks passed after the moves. Preferences matched the baseline pixel for pixel. Viewer differences were confined to asynchronously completed thumbnails; the canvas, controls, inspector, and status matched the baseline. X11 checks passed for navigation, toolbar and filmstrip clicks, rotation, rename with Enter, trash cancellation and confirmation, clearing the last image, preferences, fullscreen, the context menu, and clean exit.

A separate native check verified GIF playback, pause, and resume by comparing image-region captures. Settings persistence on exit passed with an isolated configuration directory. All file-operation checks used copied fixtures and an isolated trash directory under `/tmp/flowa-layout-jzn70kfp`. No personal files or saved preferences were changed. This pass rebuilt the debug executable; release measurements below remain historical.

## UI architecture refactor

The UI ownership refactor on 2026-10-31 passed `cargo fmt --check`, `cargo clippy -j 3 --all-targets -- -D warnings`, `cargo test -j 3`, `cargo check -j 3 --no-default-features`, and a debug build. All 27 tests pass, including ten new tests for stale folder/decode results, clearing an image, selection/view reset, selection-preserving sorting, rename during loading, modal/preference shortcut suppression, and rename submission with Enter.

Native Vulkan windows were checked under Wayland for rendering and under X11 for interactions. Preferences matched the pre-refactor screenshot pixel for pixel. The viewer matched a repeated pre-refactor capture pixel for pixel; the initial comparison differed only in the inspector's animated scrollbar visibility. A separate 4096 by 3072 RGBA fixture at 100% zoom loaded two detailed tiles and also matched the pre-refactor capture pixel for pixel.

X11 interaction checks passed for keyboard navigation, thumbnail/details toggles, rotation, rename with Enter, trash cancellation, preferences, fullscreen/Escape, the canvas context menu, toolbar navigation, filmstrip selection, confirmed trash, clearing the final image, and clean exit. File operations used copied fixtures under `/tmp/flowa-ui-refactor`, with an isolated configuration and trash directory. A separate native run verified that a thumbnail-strip toggle persisted on exit using an isolated settings directory. No personal images or saved preferences were changed.

This pass rebuilt the debug executable. The release measurements below are historical and do not describe a newly rebuilt release executable.

## Earlier automated checks

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test`: 17 passed. These cover natural folder ordering, navigation boundaries, portable filename validation, rename collision protection, settings round trips and defaults, KDE palette parsing, zoom anchoring, inverse rotation/flip transforms, PNG decode/export, transparency, animation delays, corrupt input, decoded memory limits, and preview sizing without enlargement or aspect distortion.
- `cargo check --no-default-features`: passed.
- `cargo build --release`: passed. The Linux executable is 27,775,192 bytes, about 27.8 MB, plus shared system codec libraries.
- `desktop-file-validate packaging/linux/flowa.desktop`: passed.
- `sh -n packaging/linux/install-local.sh packaging/linux/uninstall-local.sh`: passed.

## Earlier native desktop checks

Actual wgpu windows rendered under Wayland and X11. Screenshots were captured from the application's renderer or its own X11 window, not from an HTML mockup.

Generated fixtures opened successfully as PNG, JPEG, GIF animation, SVG, HEIC, JPEG XL, and AVIF. The corrupt PNG produced a recoverable error screen. Preferences rendered with the system Sweet palette and with Nord. The normal viewer, filmstrip, metadata panel, animation controls, and theme picker were visually inspected.

Keyboard-driven X11 checks passed for:

- Right-arrow navigation between sibling files.
- Thumbnail-strip and metadata toggles.
- Rotation commands.
- F2 rename, editing the name, and submitting with Enter.
- Cancelling the trash confirmation without changing the file.
- Confirming trash and verifying that a generated file moved into an isolated trash directory.
- F11 fullscreen, hidden controls, and Escape to return.
- Opening and closing Preferences.
- Ctrl+Q and successful process exit.

A separate run with an isolated configuration directory verified that preferences persist on exit, including the thumbnail-strip choice, System theme, and checkerboard disabled by default. All file-management checks used generated files under `artifacts`; no personal image collection was used.

## Large-image check

A generated 16000 × 10000 PNG, 160 megapixels, opened at 100% zoom. The rendered screenshot showed full-resolution tile detail rather than the enlarged preview. A pixel transition at the fixture's hard edge was checked in the screenshot. The source image is larger than the GPU's single-texture requirements handled by this renderer because it uploads visible tiles separately.

The final release run, including native window startup and screenshot encoding, completed in 3.37 seconds and peaked at approximately 1175 MiB RSS. Four detailed tiles were resident for the captured viewport. This is a single synthetic image with a small compressed file and a warm filesystem cache; it is not a general image-decoding or frame-rate benchmark. The full decoded image remains resident in CPU memory.

## Limits of this validation

Windows source support and a Windows core CI job are included, but no Windows executable was run on this Linux machine. Camera RAW, layered PSD, monitor-specific ICC transforms, and HDR output are outside the implemented format/display scope. AVIF and HEIF depend on the native codec build and installed HEIF plugins. The local installer was syntax checked; it was not run, and no default file associations were changed.

Generated images, native screenshots, and smoke-test logs are in the ignored `artifacts` directory. They can be regenerated using the example and development switches documented in README.md.
