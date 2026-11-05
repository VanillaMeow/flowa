# Building

Run `just` to list commands.

## Local build

Requires Rust 1.99+, Just, a C compiler, and pkg-config. Linux video builds need
FFmpeg 9+, libplacebo 7.360+, SDL3, and Vulkan headers. AVIF and HEIC support need
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

Cargo compiles the C video shim automatically. Local builds use system media
libraries. For CPU-specific optimization:

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

## Portable Linux build

Requires Just and Podman or Docker. The container supplies Rust and the native SDK;
Ubuntu 24.04's system FFmpeg is too old for a local video build.

```sh
just portable
just test-portable
```

Output: `target/x86_64-unknown-linux-gnu/portable/flowa`

This is one x86_64 executable with statically linked media libraries. It requires
glibc 2.39+, a Vulkan loader, GPU drivers, and a host audio backend. Video decoding
requires compatible hardware; software video decoding is disabled.

Sources are checksum-verified and cached in `.cache/portable-deps`. Builds use two
jobs by default; override with `CARGO_BUILD_JOBS`. Portable builds always use the
generic x86-64 CPU baseline. `CONTAINER_ENGINE=docker` selects Docker explicitly.

`just portable-native` runs the same build on an already prepared Linux host.
The required SDK packages are listed in `tools/portable/Containerfile`.

## Development

| Command | Action |
| --- | --- |
| `just fmt` | Format Rust and C |
| `just check` | Check formatting and run Rust and C analysis |
| `just test` | Run ordinary tests |
| `just configure-c` | Generate clangd's compilation database |
| `just test-portable` | Check executable dependencies and run a clean-runtime test |

C tools: Uncrustify, clang-tidy, and clangd. On Arch, install `uncrustify clang`;
on Debian/Ubuntu, install `uncrustify clang clang-tidy clangd`.
`UNCRUSTIFY`, `CLANG`, and `CLANG_TIDY` can select alternate executables.

C formatting follows `uncrustify.cfg`. For editor formatting, pass the buffer to
`uncrustify -q -c /path/to/flowa/uncrustify.cfg -l C`. Clangd's built-in formatter
is disabled so it cannot undo the vertical header layout.

See [video validation](native/video/README.md#validation) for hardware tests.

## Install on Linux

`just install` builds and installs under `~/.local`, including the desktop entry
and icon. `just uninstall` removes the installation and preserves preferences.
