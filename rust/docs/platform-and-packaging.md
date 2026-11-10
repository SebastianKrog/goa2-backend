# Supported platform and packaging decision

Decision for C02-05 and C03-11, 10 November 2026: retain **Linux x86_64 with
Rust 1.99.1** as the tested baseline. All three crates inherit that compiler
requirement. Bundled SQLite requires a C compiler and linker. WSL is a Linux
guest workflow; keeping `cargo-wsl.sh` is not a native Windows support claim.
No macOS, native Windows, other architecture or lower MSRV is claimed. Broader
support requires its own build, test and service/persistence evidence.

Retain the `rust/` workspace prefix in standalone snapshots. Production JSON
catalogs stay at `rust/content/catalogs/`, the content manifest stays beside
them, and `oracle-manifest.json` remains at the workspace root. This preserves
the verified `include_str!` paths and Python `ROOT` conventions. The dependency
audit checks every literal embed and external test-support module against the
extraction selection, including the generated server catalogues and SQL migration.
Runtime catalogs, bundled baselines, parity metadata and benchmark recipes have
separate ownership even when a bounded test seed is deliberately shared.

**Public source hosting is the current packaging target; crates.io stays disabled.**
The workspace has `publish = false`, inherited by all three crates. A future
crates.io proposal must select names, stabilize its API policy, put all required
data within packaged-crate boundaries, add accurate metadata and run package
verification. Do not enable publication just to make the current external paths
appear packaged. Source hosting also remains subject to the independent rights,
attribution and release-owner decisions in [readiness](readiness.md).

This closes the platform/placement decisions only. It grants no redistribution
rights and does not select a public repository, maintainer identity or host.
