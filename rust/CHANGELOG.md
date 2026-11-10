# Changelog

## Unreleased — standalone preparation

- Separate production catalogs from differential fixtures without changing bytes.
- Add a native quick tier, verified bundled inputs, full-parity preflight and Rust CI.
- Support frozen oracle source archives independently of original Git history.
- Add a small synthetic game-content pack, embedding facade and compiled example.
- Introduce transactional numbered SQLite migrations for existing schema evolution.
- Make service suites independently runnable with reusable process helpers.
- Replace chronological onboarding with contributor guides and extraction tooling.

REST/WebSocket messages, rules-v9 identity, checkpoint/replay v1 and search policy
semantics remain unchanged. SQLite now records schema version 3. Public release
is blocked on the remaining gates in `docs/readiness.md`.
