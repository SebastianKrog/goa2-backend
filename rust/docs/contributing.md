# Contributing

Start with the quick checks in [README](../README.md) and the [API map](architecture.md).
No Python engine, training dependency or private frontend asset is needed for
ordinary native work. Python reference tools use their separate hashed lock.

Before changing a rule/card, identify its compiled instruction and catalog
builder. Add a concrete regression, preserve mandatory/optional action semantics,
identity and event order, and use the pinned oracle for differential evidence.
Generated data changes must include generator, source/runtime identity and semantic
comparison. Do not format JSON and silently change an admitted content identity.

Run the relevant tiers from `test-tiers.json`. Explain what changed, why, the
boundary affected and exact checks/counts. A missing artifact is a blocker for
the selected full tier. Mechanical moves need byte/identity evidence; behavior
fixes need a failing scenario. Keep formatting, rules fixes, oracle repins and
schema migrations separately reviewable.

For a bug, provide build/content identity, steps and an artificial reproduction.
Do not attach player tokens, private checkpoints, databases or real private replay
archives to a public issue. Protocol/persistence proposals must name producers,
consumers, supported versions and migration/rollback behavior.

Ownership is by role until maintainers publish a named map: engine maintainers
review rules/catalogs/restore; service maintainers review auth/privacy/receipts and
migrations; search maintainers review policy/budget/performance; release maintainers
own content provenance, rights, packaging and hosted verification. No P0 release
gate is waived by a quick-suite pass. See [readiness](readiness.md).
