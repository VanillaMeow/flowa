# Contributing

## Build

You need the following tools installed:

- [Git](https://git-scm.com/install/)
- [Rust and Cargo](https://rustup.rs/) 1.99 or later

On Arch Linux, install the build dependencies:

```sh
sudo pacman -S --needed rust pkgconf clang cmake wayland libxkbcommon dav1d libheif
```

On Debian/Ubuntu, install `build-essential`, `pkg-config`, `cmake`, `libclang-dev`, `libwayland-dev`, `libxkbcommon-dev`, `libdav1d-dev`, and `libheif-dev`. HEIF support requires libheif 1.17 or newer and an HEVC decoder plugin such as libde265 for HEIC files.

Clone the repository and build:

```sh
git clone https://github.com/VanillaMeow/flowa.git
cd flowa
cargo build --release
```

To build without native AVIF and HEIF support, use this build command instead:

```sh
cargo build --release --no-default-features
```

On Windows, use this variant with the Rust MSVC toolchain and Visual Studio C++ build tools. The executable will have a `.exe` suffix.

## Portable Linux build

For a single executable with AVIF and HEIC support included, run:

```sh
./packaging/linux/build-portable.sh
```

This uses the `portable` Cargo profile, downloads pinned source archives with SHA-256 verification, and statically links dav1d, libheif, libde265, and the C++ runtime. Little CMS is already built statically. Libheif's decoders are built in, so no codec plugins need to be installed. Other image formats continue to use the existing Rust dependencies.

The script currently supports native x86_64 GNU/Linux builds. Install a C/C++ toolchain, Cargo, CMake, Meson, Ninja, NASM, pkg-config, curl, tar, xz, and binutils. It does not need system dav1d, libheif, Wayland, or xkbcommon development packages. Sources and native build outputs are cached under `target/portable-deps`. Set `CARGO_BUILD_JOBS` to limit build parallelism. Running `cargo build --profile portable` alone applies the release optimization settings but does not download or bundle native libraries.

The executable is written to `target/x86_64-unknown-linux-gnu/portable/flowa`. CI builds natively on an Ubuntu 24.04 GitHub Actions runner. To build locally on Ubuntu 24.04, install the build tools and run:

```sh
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  build-essential ca-certificates curl pkg-config cmake meson ninja-build nasm \
  xz-utils git binutils
./packaging/linux/build-portable.sh
./packaging/linux/check-portable.sh target/x86_64-unknown-linux-gnu/portable/flowa 2.39
```

The Portable Linux workflow uploads the executable as a workflow artifact. It rejects unexpected shared dependencies and glibc requirements above 2.39. The build script also checks that the executable starts with `--version`. Building on a newer distribution can raise its glibc requirement.

For a local build on another distribution, install Podman and podman-compose, then run from the repository root:

```sh
podman-compose run --rm --build portable
```

The [Compose service](compose.yaml) builds an Ubuntu 24.04 environment from `packaging/linux/portable.Containerfile`, builds the executable, checks the glibc 2.39 ceiling, and removes the container when finished. The executable and build caches stay under `target` on the host. Builds use two jobs by default; set `CARGO_BUILD_JOBS` to change that. Later runs reuse cached image layers and build outputs.

The container uses the [official Ubuntu image's minimal root filesystem](https://ubuntu.com/docs/oci-registries/oci-reference/oci-image-configuration/), installs build tools without recommended packages, removes APT package lists, and uses Rust's minimal toolchain profile. CI builds directly on its Ubuntu runner.

The [Podman user namespace](https://docs.podman.io/en/latest/markdown/podman-run.1.html#userns-mode) keeps generated files owned by your host user. The `:Z` mount option supplies the container's SELinux label on hosts that use SELinux.

After extracting a downloaded workflow artifact, run `chmod +x flowa` to restore its executable permission.

The binary uses the host's glibc and dynamic loader. It also needs a working X11 or Wayland desktop and compatible GPU drivers, which it loads at runtime. File dialogs use the desktop's XDG portal, and SVG text uses system fonts. These desktop services and drivers are not bundled. The supported release baseline is x86_64 Linux with glibc 2.39 or newer. A musl build would still need to load the host's graphics stack, so it is not used for this release path.

## Run

```sh
./target/release/flowa
```

You can pass an image or folder path as the first argument.

## Install on Linux

After building, install the app locally:

```sh
./packaging/linux/install-local.sh
```

This installs the executable, icon, and desktop entry under `~/.local` without changing default file associations. To uninstall, run `./packaging/linux/uninstall-local.sh`.

## Checks

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo check --locked --no-default-features
```

Follow [AGENTS.md](AGENTS.md) and `rustfmt.toml` for code style. See [ARCHITECTURE.md](ARCHITECTURE.md) for the source layout and ownership model.

## UI testing

Generate test images:

```sh
cargo run --example make_fixtures -- artifacts/fixtures
```

Append `--huge` to generate a 160-megapixel image too. It needs enough RAM for the full decoded image and working buffers. Use disposable fixtures and isolated configuration/trash directories for rename and delete tests.

Capture the viewer or preferences using the release build:

```sh
./target/release/flowa --smoke-test --screenshot artifacts/viewer.png --filmstrip artifacts/fixtures/01-dunes.png
./target/release/flowa --smoke-test --preferences --theme nord --screenshot artifacts/preferences.png
```

Smoke tests capture the actual native window and exit after saving the screenshot. They do not load or save preferences.
