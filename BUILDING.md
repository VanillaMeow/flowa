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

The portable executable bundles AVIF and HEIC codecs. It targets x86_64 Linux with glibc 2.39 or newer and still needs a working desktop and GPU drivers.

With Podman and podman-compose installed, run from the repository root:

```sh
podman-compose run --rm --build portable
```

This builds in Ubuntu 24.04 and checks the glibc 2.39 limit.
Output is `target/x86_64-unknown-linux-gnu/portable/flowa`.
Build caches persist on the host. Builds use two jobs by default; set `CARGO_BUILD_JOBS` to change this.

For a native x86_64 GNU/Linux build, install Rust, a C/C++ toolchain, CMake, Meson, Ninja, NASM, pkg-config, curl, tar, xz, and binutils, then run:

```sh
./packaging/linux/build-portable.sh
./packaging/linux/check-portable.sh target/x86_64-unknown-linux-gnu/portable/flowa 2.39
```

The script downloads verified codec sources and caches them under `.cache/portable-deps`. System codec development packages are unnecessary. Building on a newer distribution may exceed the glibc limit. `cargo build --profile portable` alone does not bundle codecs.

The [portable workflow](.github/workflows/portable.yml) uses the same build and compatibility checks. After downloading its `flowa` artifact, run `chmod +x flowa`.
