# Building

Run `just` to list commands.

## Local build

Requires Rust 1.99+, Just, a C compiler, and pkg-config. Video binding generation
also requires Clang/libclang. Linux video builds need FFmpeg 9+, libplacebo 7.360+,
SDL3, and Vulkan headers. AVIF and HEIC support need
dav1d and libheif 1.17+ with an HEVC decoder.

On Arch Linux:

```sh
sudo pacman -S --needed rust just base-devel pkgconf clang cmake wayland \
    libxkbcommon dav1d libheif ffmpeg libplacebo sdl3 vulkan-headers
```

Build and run:

```sh
just build
./target/release/flowa /path/to/file
```

Cargo generates FFmpeg bindings from the installed SDK and compiles the C GPU
shim automatically. Local builds use system media libraries. For CPU-specific optimization:

```sh
RUSTFLAGS="-C target-cpu=native" just build
```

`just run /path/to/file` builds and runs a debug version. Plain Cargo commands work
as well. To build without video, AVIF, or HEIF:

```sh
cargo build --release --no-default-features
```

On Windows, use that command with Rust MSVC and the Visual Studio C++ build tools.

Linux file dialogs and trash operations need a working desktop portal and D-Bus.

## Windows Vulkan build preparation

The video source path is enabled for Windows, with Vulkan as the primary GUI and
video renderer. Native Windows compilation, packaging, and GPU playback have not
yet been validated. This is not a completed Windows release milestone.

Use an x64 MSVC developer environment with Rust, Clang/libclang, pkg-config, and
an SDK containing FFmpeg 9+, libplacebo 7.360+, SDL3 3.2+, and Vulkan headers.
The SDK must provide matching headers, MSVC-compatible import libraries, runtime
DLLs, and `.pc` files with valid include and library paths. Configure
`PKG_CONFIG_PATH` for that SDK and `LIBCLANG_PATH` if libclang is outside its normal
search path. Put runtime DLLs on `PATH` or alongside the executable.

Build the video path without requiring the optional AVIF/HEIF SDKs:

```sh
cargo build --locked --release --no-default-features --features video
```

FFmpeg must be built with the intended hardware paths. NVDEC needs the compatible
NVIDIA driver and CUDA/Vulkan interoperability; Vulkan Video needs the matching
driver and codec capabilities. VA-API is skipped on Windows. Run the native tests
from the [video guide](native/video/README.md#validation) on Windows hardware before
declaring any GPU/codec combination supported.

Auto tries Vulkan first and retries DX12 for GUI startup if necessary. Video on a
DX12 renderer remains planned work. `static-video` currently supports Linux only;
Windows DLL assembly and deployment remain part of native validation and packaging.

## Portable Linux build

Requires Just and Podman or Docker. The container supplies Rust and the native SDK;
Ubuntu 24.04's system FFmpeg is too old for a local video build.

```sh
just portable
```

Output: `target/x86_64-unknown-linux-gnu/portable/flowa`

This is one x86_64 executable with statically linked media libraries. It requires
glibc 2.39+, a Vulkan loader, GPU drivers, and a host audio backend. Video decoding
prefers compatible hardware and falls back to CPU decoding. Rendering still
requires Vulkan, including when decoding on the CPU.

Sources are checksum-verified and cached in `.cache/portable-deps`. Builds use two
jobs by default; override with `CARGO_BUILD_JOBS`. Portable builds always use the
generic x86-64 CPU baseline. `CONTAINER_ENGINE=docker` selects Docker explicitly.
The build checks the executable's shared dependencies, RPATH, and glibc baseline.

`just portable-native` runs the same build on an already prepared Linux host.
The required SDK packages are listed in `tools/portable/Containerfile`.

## Development

| Command | Action |
| --- | --- |
| `just fmt` | Format Rust and C |
| `just check` | Check Rust formatting, run Clippy, and check the minimal feature build |
| `just test` | Run ordinary tests |

Install Uncrustify to format C with `just fmt`. `UNCRUSTIFY` can select an alternate
executable. Cargo compiles the C shim during video builds.

C formatting follows `uncrustify.cfg`. For editor formatting, pass the buffer to
`uncrustify -q -c /path/to/flowa/uncrustify.cfg -l C`. Clangd's built-in formatter
is disabled so it cannot undo the vertical header layout.

See [video validation](native/video/README.md#validation) for hardware tests.

## Install on Linux

`just install` builds and installs under `~/.local`, including the desktop entry
and icon. `just uninstall` removes the installation and preserves preferences.
