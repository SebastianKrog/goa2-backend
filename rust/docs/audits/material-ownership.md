# Validation, scratch and guard material decisions

C03-08/C03-09 are recorded by
`python3 rust/tools/material_inventory.py --output REPORT.json`.
The [inventory](material-inventory.json) covers all 29 tracked milestone JSON
records and all files in the repository-owned `rust/artifacts` tree, with raw
hashes, sizes, last-change Git provenance, original scope/compiler fields where
recorded, reference consumers and structural regression-input candidates.
Missing historical compiler fields remain `not_recorded`; the current compiler
is not assigned retroactively. Historical success counts are not current gates.

**Decision: retain every original artifact and diagnostic; delete nothing.**
Exclude the original tree from the standalone runtime/public extraction until its
rights/provenance review. Retain its metadata inventory in the extracted docs.
The structural scan found no embedded envelope/operation/query input objects in
the tracked reports under the inspected payload keys. This is not proof that a
report has no unique information: preserving all bytes avoids that assumption.
Any future retirement needs reference review and promotion of unique scenarios
before deletion. The inventory lists byte-identical groups if any; similarity
or a different filename alone is not a deduplication rule.

Ignored/uncommitted cache and report bodies are not parsed or modified. Private
player data, developer directories and `/workspace/work` intermediates are not
within this deletion audit. No scratch work is removed to satisfy a space target.

[Error/guard classification](error-guards.json) separates intentional boundary and
resource rejection, native porting/representation limits, the recorded recursive
Python reference defect, and mine reference/order/coercion defects. Each category
names source consumers, regression tests and its retention/removal condition.
Existing error metadata is retained byte-for-byte. A porting limitation is not a
permanent gameplay rule, and a reference exception is not silently normalized into
a successful native result. Full-feature test names are coverage obligations,
not claims of execution with missing payloads.
