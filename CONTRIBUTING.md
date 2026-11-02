# Contributing

See [BUILDING.md](BUILDING.md) for setup, building, and installation.
Follow [AGENTS.md](AGENTS.md) and `rustfmt.toml` for code style.
See [ARCHITECTURE.md](ARCHITECTURE.md) for the source layout and ownership model.

## Checks

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo check --locked --no-default-features
```

## UI testing

Generate test images. Add `--huge` for a 160-megapixel image, which needs extra RAM.

```sh
cargo run --example make_fixtures -- artifacts/fixtures
```

Capture the viewer or preferences with a release build:

```sh
./target/release/flowa --smoke-test --screenshot artifacts/viewer.png --filmstrip artifacts/fixtures/01-dunes.png
./target/release/flowa --smoke-test --preferences --theme nord --screenshot artifacts/preferences.png
```

Smoke tests capture the native window and exit without loading or saving preferences.
Use disposable fixtures and isolated configuration/trash directories for rename and delete tests.
