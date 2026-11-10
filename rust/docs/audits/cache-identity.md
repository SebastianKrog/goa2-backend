# Cache identity and invalidation audit

C03-10 covers the cache/admission contract using the present checkout, bounded
smoke inputs and synthetic corruption/identity cases. It does not establish full
parity for unavailable payloads (C03-01–04/C04 remain open).

| Boundary | Identity/admission | Invalidation or failure |
| --- | --- | --- |
| Runtime catalogs | `content/manifest.json` raw hashes, catalog schema and admitted source hash | Changed bytes/schema/source reject quick data verification; no move-time regeneration |
| Smoke content and input set | Schema-1 manifests, exact oracle revision/source hash, raw content/case hashes and semantic case hashes | Manifest source/schema drift, changed membership, changed bytes or ordered case data fail |
| Full trace index | Registry raw index hash/count where admitted, schema 2, exact oracle/source identity and self-digest | Resigning a changed index does not bypass the registry; wrong revision/source/count or empty/duplicate selection fails |
| Indexed cases | Content-addressed logical path, case name, ordered canonical JSON SHA256 | Payload edits, changed order, null/value/reference mutations or renamed cases fail; full preflight never substitutes smoke |
| Monolithic families | Admitted raw SHA256 in registry/cache identities | Different plain/gzip decompressed bytes fail full verification |
| Frozen exporter runtime | Source-file hashes plus CPython/Pydantic/core/jsonpatch identity in oracle manifest | `verify_sources` rejects source/runtime drift before reference generation |
| Bridge/generator version | Recorded source commit/hash in smoke provenance, benchmark inventory and gate catalog | Changed bridge behavior needs review; identical oracle source is not proof that a changed generator emits the old cases |
| Physical state and protocol | Ordered JSON, separate physical instance entries and repeated reference indices retained byte-for-byte | IO never deduplicates equal objects, sorts dictionaries, rewrites nulls or normalizes arbitrary IDs |

Plain authoring files are intentionally tried before `.gz` and before an external
cache; corruption or admission mismatch must fail rather than fall through to a
convenient alternate. Byte IO itself does not certify gameplay. The selected
verification tier admits metadata and payload identities before running native or
reference comparisons. Gzip uses a zero timestamp and atomic publication; readers
decompress one case at a time. No whole-corpus extraction/cache was introduced.

The strengthened verifier checks the smoke revision/source/schema and the full
registry's index bytes/count as well as the existing self-digest. Regression tests
exercise self-consistently re-signed wrong metadata, reordered maps, nulls, two
equal-but-distinct instances, repeated references and maximum-width seed values.
Existing cache tests cover deterministic compression, corruption, interrupted
publication and original-byte retention. The generator-mismatch investigation
remains C03-03: no new generated output was admitted as a replacement.
