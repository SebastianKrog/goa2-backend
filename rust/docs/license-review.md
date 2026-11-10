# License, attribution and public payload review

The workspace currently declares MIT in Cargo metadata and disables publishing.
That declaration is preserved. It is not evidence that every upstream source,
board-game name/text/map, generated catalog or fixture can be redistributed.
No public release or original-history rewrite is performed by extraction tools.

| Material | Provenance | Required decision |
| --- | --- | --- |
| Existing native code | Backend `f0fb93e1`; original commit mapping in extraction source map | Confirm intended MIT grant, holders and upstream notices; add full license text with accurate attribution |
| Rules catalogs and content packs | Exact oracle revision/file/runtime hashes in `oracle-manifest.json` | Confirm rights for generated rules, hero/card text, names and maps; separate restricted material if necessary |
| Smoke and full parity cases | Source commit and per-case hashes in manifests/indexes | Confirm public fixture/artifact rights and absence of private match data |
| Bundled dependencies | `Cargo.lock`, generated dependency license inventory | Review licenses/advisories with a maintained scanner; document any exceptions |
| Historical records | Archived milestone/source documents | Review personal paths, attribution and references before public distribution |

The fixture/content sets in this change contain synthetic inputs and static
content rather than real game credentials. This does not substitute for a full
public-tree/history rights and privacy review. Prefer a deliberate clean snapshot
or filtered extraction; never rewrite the original repository as cleanup.
Destination repository, named maintainers and private security reporting must be
selected before publication. Crates.io remains disabled until packaging/data paths
and API policy are independently reviewed.
