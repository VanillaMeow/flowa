# Building

Run `just` to list every command.

## Requirements

Rust 1.99+, Just, a C compiler, pkg-config, and Clang. For video, AVIF, and HEIC
you also need FFmpeg 9+, libplacebo 7.360+, SDL3, Vulkan headers, dav1d, and
libheif 1.17+ with an HEVC decoder.

On Arch Linux:

```sh
sudo pacman -S --needed rust just base-devel pkgconf clang cmake wayland \
    libxkbcommon dav1d libheif ffmpeg libplacebo sdl3 vulkan-headers
```

## Build and install

```sh
just install    # Build for this computer's CPU and install under ~/.local
just uninstall  # Remove it again, keeping your settings
```

| Build | Install | Output | Use it for |
| --- | --- | --- | --- |
| `just build` | `just install` | `target/release` | This computer |
| `just build-generic` | `just install-generic` | `target/generic` | Other computers with the same libraries |
| `just portable` | `just install-portable` | `target/x86_64-unknown-linux-gnu/portable` | Any x86_64 Linux |
| `just portable-native` | `just install-portable-native` | `target/x86_64-unknown-linux-gnu/portable-native` | This computer, without installing system libraries |

Each build type keeps its own output folder, so switching between them doesn't
trigger a rebuild. `just run /path/to/file` builds and opens a debug build.

To build without video, AVIF, or HEIC, which needs none of the media libraries:

```sh
cargo build --release --no-default-features
```

## Portable builds

Portable builds need only Just and Podman (or Docker, with
`CONTAINER_ENGINE=docker`). They produce a single executable with the media
libraries built in. It runs on glibc 2.39+ and needs a Vulkan driver and an
audio backend.

`just portable-native` is tuned for the CPU it's built on, so it may not run on
other machines. It suits image-based distributions such as Aurora, where adding
system packages is awkward.

Builds use every CPU core. Set `CARGO_BUILD_JOBS` to use fewer if the machine runs
out of memory. Downloaded sources are cached in `.cache/portable-deps`.

The portable build patches FFmpeg to fix Vulkan Video decoding. See the
[video backend notes](native/video/README.md) for details.

## Windows

Windows support is untested. With Rust MSVC, Clang, and an SDK that provides the
video libraries through pkg-config, try:

```sh
cargo build --locked --release --no-default-features --features video
```

## Development

| Command | Action |
| --- | --- |
| `just fmt` | Format Rust and C (C needs Uncrustify) |
| `just check` | Check formatting, run Clippy, and check the build without optional features |
| `just test` | Run tests |

C code follows `uncrustify.cfg`. Hardware playback tests are described in the
[video backend notes](native/video/README.md#validation).
