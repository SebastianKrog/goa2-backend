# Synthetic baseline games

C01-02 defines four portable, disposable service drivers covering manual 2D
protocol play, bots, timed play, rollback, upgrades, restart and private replay.
The generator inputs, fixed seed (29), seven-hero test-pack hash, oracle/source
identity, driver hashes and coverage are recorded in `manifest.json`.

From the repository root, with the separately locked tooling environment:

```sh
python3 rust/tools/baseline_games.py --binary /absolute/path/goa2-game-server --output /absolute/path/new-observations.json
```

The runner creates temporary SQLite databases and starts local processes only.
The drivers assert expected commands, scoped views, ordered events, revisions,
retry/restart/rollback/replay results, bot behavior and upgrade choices. Additional
coverage checks require the upgrade phase and acquired level, running clocks,
planning/resolution and the declared routes. Four drivers passed and yielded 473
observations in the admitted `observations.json` record. Its checksum identifies
that run, not a promise of identical polling or clock timings on every machine.

Observations use allowlisted fields: command kind/printed card/selection class,
expected status, revision, phase/round/turn, scoped hero/card visibility summary,
clock status and ordered event types. Random game IDs become stable logical IDs.
Credentials, request tokens, private checkpoints and private replay bodies are
never copied. The regression tests include a private-value sentinel and ordered
event check. Driver source changes require reviewing the recorded identity.

The recorded frontend consumer is `a3c5e0a`. This baseline replays its native
scoped protocol; actual browser 2D rendering/reconnect/play is separately recorded
in `rust/docs/cleanup-evidence.json`. No frontend redesign or browser claim is
introduced by running the service drivers.

Inputs are synthetic. Their redistribution grant remains the independent
C01-03/C07 decision; this operational baseline does not claim rights clearance.
They are separate from frozen full parity corpora and may not replace them.
