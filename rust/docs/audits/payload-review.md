# Preliminary standalone payload review

Reviewed on 11 November 2026 after cleanup checkpoint 29c3ee4e.
The [structured evidence](payload-review.json) records the selection size,
resolved locations and the audit tool's scope.

Personal Linux checkout, Windows runtime, WSL share and worker-cache paths in
the historical development guide now use generic user examples. Its original
Git version and byte identity remain available, and the archive banner plus
[documentation provenance](documentation.json) identify this limited edit.

The repeatable audit covers every file selected by the extraction manifest,
including historical guides and compressed text. It checks personal paths,
GitHub credential forms, cloud access-key forms and private-key headers.
It reports file/category/line only. Binary or oversized selected files require
review. The current scan has zero findings.

~~~sh
python3 rust/tools/payload_audit.py
python3 rust/tools/documentation_audit.py --output /absolute/path/documentation-audit.json
~~~

CI runs both commands in the tooling job. Regression cases cover compressed
history, generic examples, bounded reads and reports that never echo matched
credential values.

These checks support the later public-tree review. Final source/content rights,
the approved payload, history strategy, named ownership and public-release
acceptance remain separate open decisions.
