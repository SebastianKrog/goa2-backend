# Third ten locally actionable cleanup tasks

Completed on `bots-upstream` after `c001c6a9`, 10 November 2026. The
[structured evidence](third-ten.json) records the actual checks and limits.
The checklist now has **30 completed and 54 open tasks**.

| Task | Result / maintained proof |
| --- | --- |
| C08-05 | [Concurrency/lock review](concurrency-and-panics.md): moved contended bot/deadline snapshots and socket vote registration/notice/cleanup to the blocking pool. A current-thread runtime regression proves timer progress during a held snapshot mutex; all process scenarios pass. |
| C09-01 | [Export/caller inventory](boundaries.json) and [review](api-boundaries.md): every public root module, declarations/reexports, conservative Rust imports, Python requests/streams/dynamic adapters and restore/search consumers are retained and classified before visibility changes. |
| C09-02 | Service imports migrated to `api`, supported `recommend_bot` wrapper consumed by bots, worker imports migrated to explicit `api::experimental_search` reexports. Old aliases and raw internal visibility stay until full migration evidence; no new crate is justified. |
| C09-03 | All 39 worker operations have status/consumer/owner/retirement metadata, required-field examples and schema/error tests; request and streaming consumers are recorded. Diagnostics remain separately dispatched; no operation is removed based on an absent direct caller. |
| C09-04 | Typed internal categories and bounded boundary context preserve HTTP status/detail/Display and worker strings/packets. Native category/privacy and actual Axum response-body regressions pass; generic internal-storage detail remains private. |
| C10-05 | [Dense/object consumer decisions](api-boundaries.md#dense-versus-object-aware-execution-c10-05) retain both implementations and their unique consumers. State/bindings determine representation; no branch, alias, diagnostic or corpus is retired. |
| C10-07 | [Active version ownership](versions-and-robustness.md): core and service constants replace duplicated active-reader/writer literals while independent identity/future/header checks and frozen/historical data stay unchanged. |
| C11-01 | [Panic/trust review](concurrency-and-panics.md#panic-assumptions-and-trust-distinctions) and source inventory distinguish guarded external rejects, trusted embedded/compiled invariants and test assertions, retaining size/depth/entity/step/coordinate limits. |
| C11-02 | [Deterministic mutation/property tests](versions-and-robustness.md#new-deterministic-mutationproperty-coverage) cover malformed reference tables/bindings, hex coordinates, catalog JSON/types, replay ranges/hashes/rewinds and worker request schemas. |
| C11-03 | Six seeds × three commitments check exact ordered checkpoint/reference/binding restore, root/sibling isolation, recipient privacy, stale revision/foreign transition rejection and cancelled/deadline/step-limited preparation nonmutation. |

## Validation

The pinned Rust 1.99.1/Linux iterate suite passes **400 native tests**, with 64
explicit ignores and no failures. Eleven native tests are added in this batch;
the stronger ordered-serialization property check also passes its targeted rerun.
All **15 real-process service scenarios** pass across six suites after the executor
fix. The persistent worker handles **71 packets: 69 rejects and two successes**.
All 39 protocol examples pass required-field and future-version tests without
executing their synthetic empty envelopes as gameplay.

Quick admitted-data verification passes **572/572**, Python tooling **38 tests**,
plus strict formatting, all-target/all-feature Clippy, rustdoc and the compiled
embedding example. Disposable extraction checks cover tooling, admitted data,
local documentation links, all nine gate definitions, the worker process contract
and the WAL backup scenario against the same built native binaries. This is an
extraction/tooling rehearsal, not the final approved public candidate/C12 gate.

Full preflight continues to reject **23,955 missing indexed cases and 12 missing
monolithic payloads**. Catalog/content/index/oracle and original report bytes are
unchanged. No rules correction, generator repin, dense-path removal, publication,
deployment or original-history rewrite occurs. The concurrency fix changes which
thread waits for existing locks; game advancement, candidate persistence, ordered
events and accepted wire fields remain the same.

## Selection and limits

These source audits, adapter/identity refinements and bounded regressions use
available source, synthetic content and disposable native processes. Acquisition,
historical corpus/binary matrices, rights/repository-owner decisions and remote
workflow results remain open. Interpreter/search restructuring and its full
fixed-work/differential acceptance still require the missing admitted corpora;
measured performance/footprint work remains a distinct C11 follow-up.

The conservative caller/panic inventories are source-review evidence, not proof
that unknown external Rust embedders do not exist or that arbitrary corrupted
trusted internal state cannot panic. No item is made private/retired based on the
inventory. Internal diagnostic codes do not silently migrate the existing public
worker/HTTP schema. Periodic/cold/load and hosted release acceptance are not claimed.
