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
