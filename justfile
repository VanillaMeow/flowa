set positional-arguments

formatter := env("UNCRUSTIFY", "uncrustify")
portable_binary := "target/x86_64-unknown-linux-gnu/portable/flowa"

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
    tools/portable/container.sh build

# Build portable on a prepared host, including the builder container
[private]
portable-native:
    tools/portable/build.sh

# Format Rust and C sources
fmt:
    just --fmt
    cargo fmt
    {{ quote(formatter) }} -q -c uncrustify.cfg --replace --no-backup native/video/*.c native/video/*.h

# Check Rust and C formatting, including the C layout fixture
[private]
check-format:
    just --fmt --check
    cargo fmt --check
    {{ quote(formatter) }} -q -c uncrustify.cfg --check native/video/*.c native/video/*.h tools/tests/c-format.c

# Run formatting checks and Rust and C static analysis
check: check-format check-rust check-c

# Run Rust analysis, optionally selecting Cargo features
[private]
check-rust *args:
    cargo clippy --locked --all-targets "$@" -- -D warnings
    cargo check --locked --no-default-features

# Run C syntax checks and static analysis
[private]
check-c:
    tools/checks/c.sh

# Generate C editor flags without running static analysis
configure-c:
    tools/checks/c.sh --configure

# Run ordinary tests, optionally passing Cargo test arguments
test *args:
    cargo test --locked "$@"

# Verify portability and software video playback in a clean Linux runtime
test-portable binary=portable_binary:
    tools/portable/container.sh test "$1"

# Build and install locally, including desktop integration
install: build
    tools/install.sh install

# Remove the local installation, preserving preferences
uninstall:
    tools/install.sh uninstall
