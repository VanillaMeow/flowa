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
shim automatically. Local builds use system media libraries and are optimized for
this computer's CPU. To share a build with other machines that have the same
system libraries, build without CPU-specific instructions instead:

```sh
just build-generic
./target/generic/flowa /path/to/file
```

Each flavor uses its own Cargo profile and output folder, so switching between
them does not rebuild the other.

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

Sources are checksum-verified and cached in `.cache/portable-deps`. Builds use every
CPU core; set `CARGO_BUILD_JOBS` to limit them on machines with little memory. `just portable` uses the generic
x86-64 CPU baseline. `CONTAINER_ENGINE=docker` selects Docker explicitly.

`just portable-native` builds the same self-contained executable tuned for the
computer running the build, into `target/x86_64-unknown-linux-gnu/portable-native/flowa`.
Containers see the host's CPU, so the tuning matches the machine. The result may
not run on other CPUs, so build it on the computer that will use it. This suits
image-based distributions such as Aurora, where adding system packages is awkward:
only Podman and Just are needed, and `just install-portable-native` builds and
installs it in one step. Media libraries stay generic and already choose
SIMD instructions at runtime, so both flavors share one dependency cache.
The build checks the executable's shared dependencies, RPATH, and glibc baseline.

FFmpeg remains pinned to 9.0, validated with NVIDIA 580 and its CUDA 13.0 driver
API. Revalidate NVDEC before changing the FFmpeg or NVIDIA codec-header versions.
The local FFmpeg 9.0.2-100 package has a separate CUDA 13.0 compatibility patch;
its playback results do not establish compatibility for unpatched 9.0.2 builds.

The portable SDK applies `tools/portable/ffmpeg-vulkan-frames.patch` before building
FFmpeg. It restricts Vulkan decode-image flags to those supported by the video
profile and fixes access-mask bookkeeping and decoder synchronization. Native
system-library builds need equivalent fixes for affected drivers. CUDA compatibility
patches alone do not resolve this Vulkan allocation issue.

`just portable-host` runs the same build on an already prepared Linux host, and
`FLOWA_PORTABLE_CPU=native just portable-host` tunes it for that host.
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

`just install` builds for this computer and installs under `~/.local`, including
the desktop entry and icon. `just install-generic` installs the generic build
instead, and `just install-portable` or `just install-portable-native` builds and
installs the matching portable executable. Builds are cached, so installing again
without changes does not recompile. `just uninstall` removes the installation and preserves
preferences.
