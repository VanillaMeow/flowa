# Building

## Build

You need the following tools installed:

- [Git](https://git-scm.com/install/)
- [Rust and Cargo](https://rustup.rs/) 1.99 or later

On Arch Linux:

```sh
sudo pacman -S --needed rust pkgconf clang cmake wayland libxkbcommon dav1d libheif
```

On Debian/Ubuntu:

```sh
sudo apt install build-essential pkg-config cmake libclang-dev libwayland-dev libxkbcommon-dev libdav1d-dev libheif-dev
```

HEIF support needs libheif 1.17 or newer and an HEVC decoder plugin such as libde265 for HEIC files.

## Build and run

```sh
git clone https://github.com/VanillaMeow/flowa.git
cd flowa
cargo build --release
./target/release/flowa
```

Pass an image or folder path as the first argument.

## Video playback on Linux

Build Flowa's Vulkan-enabled libmpv before opening videos:

```sh
./packaging/linux/build-video.sh
cargo build --release
./target/release/flowa /path/to/video.mp4
```

See [the video backend guide](native/mpv/README.md) for native dependencies,
installation, and hardware playback tests. Flowa keeps its wgpu renderer and
shares video textures with mpv directly on the GPU.

To omit native AVIF and HEIF support:

```sh
cargo build --release --no-default-features
```

On Windows, use this variant with the Rust MSVC toolchain and Visual Studio C++ build tools. The executable is `target/release/flowa.exe`.

On Linux, file dialogs and trash operations need `xdg-desktop-portal`, a backend for your desktop, and a user D-Bus session.

## Install on Linux

After a release build:

```sh
./packaging/linux/install-local.sh
```

This installs the executable, icon, and desktop entry under `~/.local` without changing file associations.
Uninstall with `./packaging/linux/uninstall-local.sh`.

## Portable Linux build

The portable build produces one executable with statically linked AVIF and HEIC
codecs, Flowa's Vulkan libmpv backend, FFmpeg 9.0, SDL 3.4.18, and libplacebo.
It targets x86_64 Linux with glibc 2.39 or newer. Copy the executable wherever you
want to run it. There is no launcher, extraction step, or companion media library.

A working desktop, Vulkan loader, and GPU drivers are still required. SDL3 loads
the host's PipeWire, PulseAudio, or ALSA client library for audio. VA-API and CUDA
also use the host's GPU drivers. Software decoding remains available when the
source codec has no hardware decoder.

With Podman and podman-compose installed, run from the repository root:

```sh
podman-compose run --rm --build portable
```

This builds in Ubuntu 24.04 and checks the glibc 2.39 limit.
Output is `target/x86_64-unknown-linux-gnu/portable/flowa`.
Build caches persist on the host. Builds use two jobs by default; set
`CARGO_BUILD_JOBS` to change this. The first build compiles the media stack and
will take longer than a regular Rust build.

For a native x86_64 GNU/Linux build, install Rust and the packages listed in
[the builder container](packaging/linux/portable.Containerfile), then run:

```sh
./packaging/linux/build-portable.sh
./packaging/linux/check-portable.sh target/x86_64-unknown-linux-gnu/portable/flowa 2.39
```

The script downloads checksum-verified sources and caches them under
`.cache/portable-deps`. FFmpeg and SDL are built from the pinned sources rather
than the distribution's versions. It preserves the generic x86-64 CPU baseline
even when local builds use `RUSTFLAGS="-C target-cpu=native"`.
Building on a newer distribution may exceed the glibc limit.
`cargo build --profile portable` alone does not build or link the media stack.

The [portable workflow](.github/workflows/portable.yml) uses the same build and
compatibility checks. It copies only the executable into a clean Ubuntu runtime
without system FFmpeg, SDL, or mpv, renders a video through software Vulkan, and
checks the screenshot for decoded pixels. The uploaded artifact is the `flowa`
executable. After downloading it, run `chmod +x flowa`.
