# Validation

Recorded on 2026-10-31 on Arch Linux with Rust 1.99.1. Native rendering used an NVIDIA GeForce GTX 1050 Ti through Vulkan.

## Automated checks

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test`: 17 passed. These cover natural folder ordering, navigation boundaries, portable filename validation, rename collision protection, settings round trips and defaults, KDE palette parsing, zoom anchoring, inverse rotation/flip transforms, PNG decode/export, transparency, animation delays, corrupt input, decoded memory limits, and preview sizing without enlargement or aspect distortion.
- `cargo check --no-default-features`: passed.
- `cargo build --release`: passed. The Linux executable is 27,775,192 bytes, about 27.8 MB, plus shared system codec libraries.
- `desktop-file-validate packaging/linux/flowa.desktop`: passed.
- `sh -n packaging/linux/install-local.sh packaging/linux/uninstall-local.sh`: passed.

## Native desktop checks

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
