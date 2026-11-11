# CI selection and release-candidate enforcement

C04-07, 10 November 2026: `.github/workflows/rust.yml` uses the maintained
`rust/tools/ci_scope.py` selector rather than an untested embedded snippet.
Changes anywhere in engine/search/service crates, codec/arena, fixture/test support,
content/identity/lock/toolchain inputs, acquisition/gate tools and verification
bridges select **both full differential and exhaustive restore**. This deliberately
conservative selection handles indirect consumers and avoids a narrow source-path
list missing a restore-sensitive change. Docs-only changes may skip reference
jobs. Missing/zero base history, schedules and manual runs select both gates.

Version tags (`v*`) and an explicitly selected manual `release_candidate` run
require the complete candidate matrix regardless of file diff: native quality,
tests/docs/example, tooling/service contracts, full differential, restore, and
all seven remaining recipes (setup/view/timed-bot, simulation, strength, local
hosted adapter and read-only staging). `release-acceptance` needs all prerequisite
jobs to succeed and its argv loop fails on any missing input/failed command. It
does not provision, publish or deploy anything. The selector also handles release
events fail-closed if another workflow later calls it with such a payload.

`test_ci_scope.py` covers engine, worker, codec/arena, service, fixtures, content,
acquisition/workflow changes, docs-only changes, unknown history, periodic runs,
tags/release events and manual candidate inputs. The workflow runs those tests in
the tooling job. The exact gate commands enforce admitted identities and nonzero
Cargo filters as documented in [verification gates](../verification-gates.md).

Required artifact/input locations must be configured on the candidate runner;
Python/source/content/cache paths are explicit variables, not an invented
acquisition service. Release binaries are built locally in the job. Staging
credentials use repository secrets and are only passed to the explicitly selected
read-only verifier. Missing artifact distribution still prevents full jobs from
passing (C04-01); local selector checks do not establish a remote workflow result
(C04-09), cold acquisition (C04-08), hosted compatibility or release readiness.

The full and restore jobs now call the
[reference archive installer](../fixture-archives.md) with explicit artifact URL
and SHA256 variables before running their tier. Download integrity, isolated
staging admission and nonzero full counts are enforced. The distribution
location, real archive acceptance and remote run still require evidence;
C04-01 and C04-09 remain open until those are available.
