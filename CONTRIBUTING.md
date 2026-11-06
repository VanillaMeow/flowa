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

Open the fixtures for manual UI testing:

```sh
just run --filmstrip artifacts/fixtures/01-dunes.png
```

Use disposable fixtures and isolated configuration/trash directories for rename and delete tests.
