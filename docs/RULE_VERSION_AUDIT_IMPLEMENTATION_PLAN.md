# Rule versioning and single-replay audit implementation plan

Status: proposed implementation; the referenced audit facilities are not claimed
to exist. Written 11 November 2026. This dedicated plan implements the substantial
support required by the [rule procedure](RULE_ENGINE_UPDATE_PROCEDURE.md) and
[later NN procedure](NN_RULE_CHANGE_UPDATE_PROCEDURE.md). Small identity and
recording integration points belong in the shared foundation; the machinery
below is not hidden inside that plan.

## 1. Corrected replay requirement

A game may be replayed to check it. **Each target engine replays it once and
compares reproducible decision-space hashes recorded during original play.**
The original engine does not need to replay it a second time for every check.

Do not require complete before/after snapshots at every decision or independent
per-transition cross-engine state import. Retain the existing replay start,
ordered operations, time/chance provenance and ordinary anchors. Add compact
original decision/effect evidence to that stream.

Old games without recorded evidence may need an optional one-time baseline
backfill. Preserve its reconstructed provenance and missing/unsupported counts;
do not pretend those hashes were recorded at play or require that recovery
again for every target run.

Support different engine implementations through adapters and capabilities,
not a promise that an arbitrary future engine can load an unknown archive.
Unsupported contracts/operations/hash schemas are explicit uncheckable results.

## 2. Version registry and adapters

Implement structured Python rule version plus Rust revision and variant digest,
separate from build/source/content/codec/model identities. Follow Python's
lifetime game-version gates. A change of build is not inherently a rules change;
a Rust semantic difference/fix gets its own revision record.

Map current saved data to the exact baseline contract through its admitted
source/content identity. Do not default every unknown old input to revision 0.
New games bind the selected admitted target; active games restore their pinned
contract. Dispatch may use a limited in-process implementation or separately
pinned runtime worker. Unknown/unsafe/unavailable runtime is actionable failure,
not “run latest anyway.” No broad obsolete public-API compatibility is required.

An audit adapter declares:

- engine/source/runtime artifact, supported rule/variant/content versions;
- replay start/operation/time/chance support;
- canonical decision/state/event/result schema/profile support;
- request/physical-object reference mapping and known limitations;
- supported original-contract/candidate-contract comparison modes.

First validate the Rust adapter against its own recorded trace. Validate Python
under the same Python rule contract as the gold standard, except exact scoped
known-error entries. Pin Python dependencies/source and retain artifact access.
Another engine implements the same adapter contract and conformance suite;
it does not need the same internal interpreter layout.

Reference exception records are versioned, witnessed, narrowly matched and
visible in reports. An unexpected mismatch is unresolved until classified;
neither Rust's presumed correctness nor “Python is gold” suppresses evidence.

## 3. Canonical decision-space hash

### Exact meaning

At every recorded accepted gameplay decision, capture the **complete issued
legal command space at that actual point**, before applying the command.
This is not a capped search candidate list, a sampled opponent policy,
a list of hypothetical whole-game plans or a UI-label hash.

The semantic descriptor includes decision kind/context, phase/round/turn,
addressed/eligible engine seats, optional skip/pass/finish/confirmation semantics,
and every typed command/selection permitted at that boundary. Preserve current
planning/second-slot, deployment, upgrade and team/simultaneous semantics.

Use a canonical set of legal commands with a specified stable encoding/order
for hashing. Preserve order *inside* commands/sequences when meaningful.
Do not merge different physical objects because their printed name/label matches.
Offered presentation order/duplicates, when part of the wire contract, have
separate ordering evidence; do not confuse a harmless enumeration-order
difference with a different mathematical decision space.

An explicitly specified exact structured domain is permissible for large
spaces only if independent adapters produce the same canonical representation
and tests prove equivalence to enumeration on bounded cases. Never hash a
function pointer, Python predicate source, undocumented constraint or the first
N choices as a substitute for completeness. If a domain cannot be represented
exactly, record unsupported coverage.

### Stable encoding and identities

Specify domain-tagged canonical bytes and test vectors, for example:

```text
H_decision = SHA256("goa2.decision-space.v1" || canonical_decision_bytes)
```

The exact canonical schema is a versioned public specification. It covers type
tags, signed/unsigned integer precision, null/absent values, booleans, strings,
ordered structures and unknown/invalid inputs. Reject duplicate keys and
non-finite/unsupported values. Do not round integers through JavaScript Number,
case-fold IDs, normalize strings or sort order-sensitive containers casually.

Normalize opaque request IDs through a verified logical prompt binding. Map
physical references from stable game/entity/instance origins and bindings,
not memory addresses, arena slots or allocation order. An ambiguous reference
mapping is an adapter failure, not permission to collapse instances.

Keep engine build, rule/version/content identity, game ID and trace ordinal in
the signed/trusted record envelope. Do **not** unconditionally put target
engine/version strings in H_decision itself: otherwise every new build/rule
version would differ even when its semantic decision space is identical.
Semantic context/action meaning remains in the canonical descriptor;
identity bindings prevent using a hash for the wrong game or rule contract.
If content meaning changes while choices stay the same, other checks below
and the explicit contract change carry that difference.

Hash preimages/full legal spaces may contain private hand/upgrade information.
Store full audit descriptors only where needed in restricted diagnostics;
normal trace storage may keep their digests. Even small-space hashes can leak
by guessing, so they are private audit data, not extra public player fields.
Client observations retain the foundation's separate scoped digest.

### Completeness beyond decision spaces

A matching legal-space hash cannot detect every damage, state, event or victory
error. During the same play/replay stream, also support:

- canonical semantic state digest, with physical bindings/order/knowledge and
  relevant continuation state represented by the declared profile;
- ordered semantic event/effect digest;
- selected-command identity/membership and terminal-result digest/invariants;
- raw checkpoint/archive hashes for source integrity, explicitly separate from
  portable semantic comparison.

Cross-engine state/effect profiles normalize only documented nonsemantic fields.
Keep rule/build/content identity metadata in the bound envelope, not as an
unconditional semantic-state difference: changing a version string alone must
not make every state fail. Raw checkpoint hashes still protect exact originals.
Include genuine changed game values, effects, definition meaning, clock resources
and physical bindings; exclude only specified transport handles/implementation
bookkeeping. Do not demand byte equality of different engines' private stacks,
or delete a semantic field to obtain equality. Record unsupported profile components instead
of certifying full equivalence. Identity/format versioning alone is not a rule
correctness proof.

## 4. Trace capture and single-pass comparison

Extend the accepted-operation record with trace schema, original contract/
artifacts, operation/decision ordinal, canonical context/alignment ID,
H_decision, selected-action digest, supported state/event/result digests,
coverage flags and time/chance references.

Capture on the same immutable source revision used for prepare. Commit the
accepted trace, checkpoint/journal and receipt atomically. Retries do not create
another decision. Invalid commands are not recorded as accepted legal actions.
Represent timers, pause/consensus, rewinds and administrative operations explicitly
rather than inventing a player decision for each one. Controller heartbeats
remain outside gameplay revision.

Recording must not advance the game, consume RNG, mutate shared probes, change
latency-based clock semantics or reorder output. Compute on isolated/nonmutating
views and measure projection/hash overhead. Retain a trace-integrity chain or
equivalent manifest binding to original game/revision/source authority; a digest
alone does not prove who recorded it. Reject client-submitted baseline hashes
as authoritative original evidence.

### Target start and audit range

Strict source/runtime checks remain in force. An old checkpoint cannot simply
have its rule/oracle header edited so a new engine accepts it.

For as-played/original-contract checks, load through the adapter for that
declared original contract. For candidate-contract checks, use an explicit,
versioned target-start recipe: reconstruct an admitted target setup/state from
the recorded start and target definitions, preserving supported original physical
bindings/deal/chance context. Record source and target start digests, changed
definition/field mapping, validation and intended differences. Keep the original
unchanged. A rebuild from the same seed is sufficient only when its start/RNG
equivalence has actually been proved.

If the source start cannot be mapped without inventing state or choices, report
unsupported target start. Do not force the old snapshot into a new decoder,
silently generate a different starting game or report target illegality as an
original rule violation. Test a no-change contract, supported real rule change,
definition change, invalid start and unsupported RNG/context mapping.

Default whole-game checks start at the original initial setup via that recipe.
Starting from an ordinary mid-game anchor only verifies a suffix conditional on
that anchor. It cannot certify the whole game under new rules unless a matching
target-prefix proof already exists. Bind the audited range/prefix proof to
coverage and cache identity.

For each target job:

1. Verify input integrity, original trace provenance and target/adapter/profile.
2. Validate the target start/range as above, restore it once and process recorded
   operations in order exactly once.
3. At each applicable pre-command boundary, compute target H_decision and compare
   the stored original H_decision; test selected command membership.
4. Execute the recorded operation once, with explicit time/chance semantics;
   compare supported state/events/result and record target-derived evidence.
5. Continue only with meaningful alignment. A changed mandatory prompt, illegal
   recorded action, missing chance input or unreconstructable state can make the
   suffix uncheckable. Never force an action or choose a replacement silently.
6. Publish the complete report and optional staged new-contract dataset outputs
   after validating their coverage/admission. Audit output does not mutate games.

Logical request remapping may replace opaque target request tokens only after
proving the same semantic prompt/action binding. A new rule's extra meaningful
decision cannot be skipped by pretending it is only a token change.
If earlier state diverges but recorded actions stay legal, retain first-divergence
causality and do not count every downstream mismatch as an independent root bug.

Record original random/chance evidence sufficient for supported replay:
exact generator/version/state where applicable, realized semantic chance outcomes
or a tested cross-engine tape mapping. The same seed with different RNG algorithms
or draw order is not equivalent. Merely realizing a still-possible outcome does
not prove an unchanged probability law. Bind/check the chance contract where
supported and report changed laws/sampling separately, even when the observed
path matches. New-rule chance/schema changes can be an expected
non-replayable path; classify them instead of forcing old draws into new semantics.

Cache by source trace/archive digest, target contract/artifact, adapter/profile/
hash schema, target-start recipe/digest, audited range/prefix proof and
exception-registry digest. A target change invalidates its audit
cache, but does not require recomputing immutable original hashes.

## 5. Historical coverage, findings and downstream consumers

Run all retained games in a declared cohort where feasible. Otherwise freeze
stratified/targeted sampling with counts, seed, exposure predicate, population
and omitted fraction. Exposures inferred from recorded card names alone are not
proof that nonlocal rule effects cannot occur elsewhere.

Track expected games/operations/decision points, executed/comparable matches,
divergences, observed illegal selections/incorrect transitions, expected new-rule
changes, known-reference errors, missing/unsupported and unreachable suffixes.
Categories must have documented disjoint/overlap semantics so totals are honest.
Partial checks are not whole-game certification.

Store restricted findings as immutable versions linked to game, target, rule,
comparison mode, first witness, coverage and review status. Original-contract
violation, decision-space exposure and candidate-only behavior change are
different flags. Add safe scoped game annotations only through the existing
data/result workflow. No automatic winner/Elo rewrite, player accusation or
external messaging.

The same target replay may stream features/targets into staging for the NN
procedure. Keep two admissions separate: reuse of original labels needs proved
original/target equivalence; a completely executed target trajectory may instead
be admitted as a new scripted replay-derived record with its own target state,
outcome, lineage and recipe. Missing original hashes block equivalence claims,
not necessarily this second admission. Neither is new human/competitive play.
Incomplete/illegal/unmapped target paths provide no terminal value labels.
All admissions need the declared result/split/usage checks and must not reuse
old search-policy telemetry as target search evidence.

Older games with missing evidence receive explicit coverage gaps. Optional
one-time original backfill uses pinned artifacts, isolated resources and its own
report. Mark it reconstructed; if unavailable, still run supported target
legality/invariant checks but do not manufacture original hashes.

## 6. Storage, performance and integration

Integrate with the database plan's journal/outbox/archive catalogs using trace
schema/digest references and private access classes. No full snapshot per
decision is mandatory. Existing start/ordinary anchors plus compact digests
suffice for the single-replay design; descriptors/witnesses are materialized
only as bounded restricted diagnostic outputs.

Archive older traces immutably; index metadata for contract/content/purpose,
rule exposure, game/result, trace presence and scan status. Stream games and
records with bounded decompression, worker pools, memory and job time.
Keep audits/training away from live database scans and human Elo cohorts.
Cancellation/resume acknowledges completed cases and remains idempotent.

Small foundation extensions: declare structured Python/Rust rule identity in
F01; expose nonmutating decision/transition evidence hooks and trace binding at
the existing projection/prepare/journal boundaries. All adapter/hash specification,
history scanning, version dispatch and model-impact machinery belongs here.

Current-format import/backfill preserves IDs/revisions/credentials and existing
raw digests. Migration cannot falsely state that a trace existed at original
play. Artifact retention serves active games and declared audit coverage,
without requiring arbitrary historical executable formats in the live service.

## 7. Implementation packages and acceptance

All packages are open; no new audit CLI is assumed to exist.

| ID | Deliverable | Depends on | Complete when |
| --- | --- | --- | --- |
| R01 | Extend F01 identity/current binding with multi-revision variant/known-error registries and dispatch design | Foundation identity | Version gates/current saved games and exact scoped exception tests pass |
| R02 | Canonical decision/action/reference spec and golden hash vectors | R01, foundation decisions | Rust/Python independent canonicalization agrees; completeness and order/precision cases pass |
| R03 | Nonmutating trace capture, state/effect profiles and atomic journal/archive integration | R02, current store | Recording on/off preserves gameplay/RNG; retry/failure/recovery tests preserve evidence once |
| R04 | Rust/Python adapters, validated target-start recipes and bounded single-replay runner | R01–R03 | One target replay compares stored hashes/effects; original source is unchanged and no repeated original runtime is invoked |
| R05 | Historical inventory, optional one-time baseline backfill, cohorts/resume and honest reporting | R03–R04 | Recorded/reconstructed/missing coverage is distinguishable; no prefix is falsely certified |
| R06 | Findings registry/scoped annotations, fixture/artifact impact and NN/data export integration | R04–R05 | Witnesses/expected changes/known errors are classified without mutating source results |
| R07 | Version-pinned live routing/current transition, client/content/model compatibility and rollback drills | R01, R03 | Existing games keep correct rules; new games select target; no lost accepted operations |
| R08 | Procedures/skills conformance, performance/retention and end-to-end release evidence | R01–R07 | Acceptance below passes with identities/counts and declared limitations |

Acceptance must cover:

- [ ] Independent Rust/Python hashes for the same logical legal space, including
      shuffled enumeration, typed numeric/skip values, physical twins, opaque
      tokens, planning slots, team/simultaneous prompts and exact large domains.
- [ ] A version/build change alone does not force H_decision inequality, while
      a genuinely different action/target/controller entitlement does.
- [ ] A chosen illegal action is flagged; an unchosen legal-space change is
      not falsely labelled an actual illegal move.
- [ ] An injected damage/winner error with unchanged decision space is detected
      by same-pass state/effect/result checks.
- [ ] Instrumented runner shows one replay per target, with zero repeated
      original-engine runs when a recorded baseline exists.
- [ ] Cross-version start adaptation never bypasses source guards; suffix-only
      checks cannot certify an unaudited prefix or a whole target game.
- [ ] Rule/build metadata changes alone do not force semantic-state inequality;
      genuine changed effects/state still compare under the selected profile.
- [ ] Missing original hashes, unsupported engine/contract, broken alignment,
      chance mapping, known Python error and unreachable suffix are not passes.
- [ ] Record/no-record, replay, private scope, retry/fault/rewind/clock and
      current-game-version tests preserve the exact intended behavior.
- [ ] Sample/all coverage, cancelled/resumed jobs, duplicate delivery, source
      integrity and cache invalidation are reproducible with truthful counts.
- [ ] Findings affect only audited annotations/admissions until an explicit
      result/eligibility correction; no automatic production or Elo mutation.
- [ ] NN target/data reuse distinguishes complete state/result evidence from
      decision-space equality alone.
- [ ] Real current-content/required correctness suites and bounded live-hash/
      replay/audit performance pass the declared baseline budgets.

Reports bind source/target artifacts, contracts, hash/adapter/profile schemas,
exceptions, commands, actual counts, raw findings, resource measurements and
publication/rollback decisions. Documentation and skill files can be committed
now; implementation gates remain open.
