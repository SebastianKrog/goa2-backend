# Native workspace guidance

Work in the existing checkout. Read `README.md`, `docs/compatibility.md`,
`docs/testing.md`, and `docs/readiness.md` before changing boundaries. Paths in
this file are relative to the Rust workspace. Do not change Python gameplay,
frontend layout, oracle pins or content versions as a shortcut for cleanup.

Use the pinned compiler and locked dependencies. Run format, strict Clippy,
`python3 tools/fixtures.py verify`, quick native tests and rustdoc. Choose service,
full differential, restore and performance tiers according to `test-tiers.json`.
If full artifacts are missing, report expected/missing counts; never call a
reduced run full parity. Do not replace unavailable frozen cases with regenerated
cases whose hashes differ. Preserve unique coverage and bounded decompression.

Catalogs in `content/catalogs/` are production data. Preserve bytes during moves;
`content/manifest.json` gates raw hashes. Keep mutable continuations, physical
card aliases, dictionary/list order and event order intact. Scope every client
view; checkpoints, tokens and private replay projections are private data.

Keep formatting-only work separate from rules changes. The intended Rust facade
is `goa2_core::api`; broad existing modules remain experimental and still have
reference/search consumers. Do not remove dense execution or worker diagnostics
without migrating their callers and preserving unique regressions.

Do not publish, rewrite original history, delete fixture families, or claim rights
approval. Extraction rehearsals belong in disposable directories and must use
`extraction-manifest.json`. Release blockers belong in `docs/readiness.md`.
