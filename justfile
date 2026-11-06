set positional-arguments

formatter := env("UNCRUSTIFY", "uncrustify")

# List project commands
[private]
default:
    @just --list

# Build a local release with video and installed media libraries
build *args:
    cargo build --locked --release "$@"

# Build and run a development version, optionally opening a file
run *args:
    cargo run --locked -- "$@"

# Build the single executable in the pinned Linux container
portable:
    tools/portable/container.sh

# Build portable on a prepared host, including the builder container
[private]
portable-native:
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

# Build and install locally, including desktop integration
install: build
    tools/install.sh install

# Remove the local installation, preserving preferences
uninstall:
    tools/install.sh uninstall
