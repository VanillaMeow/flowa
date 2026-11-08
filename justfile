set positional-arguments

formatter := env("UNCRUSTIFY", "uncrustify")
target_dir := env("CARGO_TARGET_DIR", "target")

# Each build flavor has its own Cargo profile, so switching flavors never rebuilds another
native_flags := "-C target-cpu=native"

# List project commands
[private]
default:
    @just --list

# Build a release for this computer's CPU with video and installed media libraries
build *args:
    RUSTFLAGS="${RUSTFLAGS:-} {{ native_flags }}" cargo build --locked --release "$@"

# Build a release without CPU-specific instructions, for machines with the same libraries
build-generic *args:
    cargo build --locked --profile generic "$@"

# Build and run a development version, optionally opening a file
run *args:
    cargo run --locked -- "$@"

# Build the single executable in the pinned Linux container
portable:
    tools/portable/container.sh

# Build the single executable for this computer's CPU in the pinned Linux container
portable-native:
    FLOWA_PORTABLE_CPU=native tools/portable/container.sh

# Build portable on a prepared host without the container; set FLOWA_PORTABLE_CPU=native to tune
[private]
portable-host:
    tools/portable/build.sh

# Format Rust and C sources
fmt:
    just --fmt
    cargo fmt
    {{ quote(formatter) }} -q -c uncrustify.cfg --replace --no-backup native/video/*.c native/video/*.h

# Check Rust formatting and analysis, optionally selecting Cargo features
check *args:
    cargo fmt --check
    cargo clippy --locked --all-targets "$@" -- -D warnings
    cargo check --locked --no-default-features

# Run ordinary tests, optionally passing Cargo test arguments
test *args:
    cargo test --locked "$@"

# Build for this computer and install locally, including desktop integration
install: build
    tools/install.sh install {{ quote(target_dir / "release/flowa") }}

# Build without CPU-specific instructions and install locally
install-generic: build-generic
    tools/install.sh install {{ quote(target_dir / "generic/flowa") }}

# Build the portable executable and install it locally
install-portable: portable
    tools/install.sh install {{ quote(target_dir / "x86_64-unknown-linux-gnu/portable/flowa") }}

# Build the portable executable for this computer's CPU and install it locally
install-portable-native: portable-native
    tools/install.sh install {{ quote(target_dir / "x86_64-unknown-linux-gnu/portable-native/flowa") }}

# Remove the local installation, preserving preferences
uninstall:
    tools/install.sh uninstall
