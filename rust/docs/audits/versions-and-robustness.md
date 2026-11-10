# Version ownership and bounded robustness regressions

C10-07/C11-02/C11-03, 10 November 2026.

## Current versions without changing frozen data

| Owner | Current constants / consumers |
| --- | --- |
| `goa2_core::versions` | `ORACLE_REVISION`, `STATE_SCHEMA=1`, `RULES=9`, `NATIVE_RESUME=1`, `CHECKPOINT=1`, `CONTENT=1`, `WORKER_SCHEMA=1`; embedded admitted oracle metadata via `oracle_identity()` |
| Core codec/live/setup/session export | Validate/read/write the shared engine constants. Codec's existing public oracle constant remains a reexport for existing callers. Source-hash/header/rules/resume disagreement checks stay independent and rejecting. |
| Worker | Shared worker schema for dispatch and output headers, including streaming paths. Future-schema examples still reject before execution. |
| Game service | Engine rules identity uses the core constant. Private service `versions` owns `REPLAY_SCHEMA=1` and `DATABASE_SCHEMA=3`; replay/migration consumers use those aliases. |

Historical migration steps, fixture JSON, generated catalog/template recipes and
Python's frozen exporter/provenance remain literal, independently admitted data.
They are not rewritten to the current constants or used as a reason to repin an
oracle. Package versions remain Cargo-owned. Source SHA/runtime identity still
comes from the admitted oracle manifest, and content raw-byte identity still
comes from its own manifest. These are distinct identities, not a single version
number. The identity regression compares active constants against embedded rules/
oracle metadata; existing negative/future/mismatch tests remain in place.

## New deterministic mutation/property coverage

`tests/robustness.rs` uses bundled content and deterministic inputs; no new fuzz
runtime/dependency or external corpus is needed.

- Malformed physical-reference indices/types, missing definitions, invalid/duplicate/
  wrong-kind object paths and future native resume versions reject without panics.
- A 65×65 coordinate grid roundtrips; overflowing/fractional/wrong-sum/extra-field
  coordinates reject. Existing Serde coordinate-sequence acceptance is preserved.
- Malformed JSON and required catalog shape/type mutations reject at the read
  boundary, without attempting native execution of untrusted template bodies.
- Six seeds including zero and maximum u64, three legal commitments each, restore
  the exact checkpoint/reference/binding representation. Candidate mutation leaves
  roots and sibling candidates unchanged; opponents see hidden commitments, stale
  revisions/foreign transitions reject, and cancellation/deadline/zero-step-budget
  preparation leaves the authoritative root unchanged.

Replay mutation tests start with a real synthetic native checkpoint and reject
wrong archive kind/schema/ranges/checkpoint hash/bot roster plus invalid or
inconsistent rewind targets, then restore the untouched original. Worker tests
cover every operation's required fields and future version, plus 128 malformed/
future request cases against a retained board. The standalone worker-process
check verifies continued successful requests after 69 rejected lines.

These cases target concrete serialization/reference/transaction/privacy risks;
they do not duplicate expected gameplay values from the implementation. They are
a bounded property/mutation set, not a claim of coverage-guided fuzzing, full
frozen parity, exhaustive malformed-input coverage or a full performance baseline.
