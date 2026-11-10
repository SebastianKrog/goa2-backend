# Fixture ownership

Production catalogs live in `../../content/catalogs/`, not here.
`registry.json` classifies retained bootstrap examples, parity indexes and reference
metadata. `smoke/manifest.json` admits a bounded native input set. Full corpus
payloads belong in `GOA2_FIXTURE_CACHE`; they are not implicitly shipped or skipped.

Run `python3 tools/fixtures.py verify` from the Rust workspace for quick data
checks, or add `--full` for counted full-corpus verification. See
[testing](../../docs/testing.md), [content](../../docs/content.md), and the preserved
[historical fixture guide](../../docs/archive/fixture-history.md).
