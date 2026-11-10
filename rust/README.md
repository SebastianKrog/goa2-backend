# GoA2 native engine

A deterministic Guards of Atlantis II rules engine, bounded bot search, and a
persistent REST/WebSocket game service. Native execution does not call Python.
This workspace is being prepared for independent source hosting; outstanding
release gates are recorded in [readiness](docs/readiness.md).

| Crate | Responsibility |
| --- | --- |
| `goa2-core` | Rule execution, physical object identity, game setup, scoped views and search |
| `goa2-game-server` | HTTP/WebSocket auth, game orchestration, SQLite, receipts, clocks, replay and shares |
| `goa2-search-cli` | JSON-lines search automation and reference diagnostics |

## Build and check

Use Linux, **Rust 1.99.1**, rustfmt, Clippy, and a C compiler/linker for bundled
SQLite. No lower compiler minimum, macOS or Windows support is claimed. Cargo
reads `rust-toolchain.toml`. Keep `Cargo.lock`; all builds use `--locked`.
From this directory:

```sh
cargo build --locked --workspace
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
python3 tools/fixtures.py verify
cargo test --locked --workspace
RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --no-deps
cargo run --locked -p goa2-core --example local_game
```

`sh cargo-dev.sh ...` provides bounded parallelism without requiring WSL or a
personal cache path. `cargo-wsl.sh` remains for existing callers. The `iterate`
profile is for local optimized checks; release is for final performance results.

The default native suite uses bundled, hash-verified inputs and needs no Python
engine, source history, sibling checkout or downloaded corpus. Tests marked
`full-parity` and environment-dependent ignored gates are explicitly excluded;
a passing quick suite **does not establish full parity**. See [testing](docs/testing.md).

## Run a local game

```sh
export GOA2_GAME_CONTENT="$PWD/content/smoke-game.json"
export GOA2_GAME_DATABASE="$PWD/target/local-games.sqlite"
export GOA2_GAME_BIND=127.0.0.1:3288
cargo run --locked -p goa2-game-server
```

The bundled synthetic test pack has seven heroes and Forgotten Island. It has
no credentials or real games and supports the example and service contract
suites. It is not a full-roster release. Restart with the same database to resume
games. Never commit that database or tokens. Full content acquisition and exact
oracle regeneration are in [content](docs/content.md).

## Contributor guides

- [Architecture and API](docs/architecture.md), [compatibility](docs/compatibility.md)
- [Testing and fixtures](docs/testing.md), [content and reference source](docs/content.md)
- [Frontend protocol](docs/frontend-protocol.md), [operations and persistence](docs/operations.md)
- [Search and benchmarking](docs/search.md), [contribution workflow](docs/contributing.md)
- [Extraction manifest](extraction-manifest.json), [release readiness](docs/readiness.md)

Historical implementation and benchmark narratives are preserved in
[development history](docs/archive/development-history.md) and
[fixture history](docs/archive/fixture-history.md). Their counts are historical.
Crates.io publishing remains disabled. Declared MIT metadata does not resolve
upstream/content redistribution rights; see [license review](docs/license-review.md).
