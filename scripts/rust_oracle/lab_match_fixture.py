"""Package a Python-verified, rules-ended arena game for the Lab browser test."""

import argparse
import json
import subprocess
from pathlib import Path

from arena_reports import load_report
from arena_verify import ReplayUnsupported, restore_python_start, verified_oracle
from port_lab_situations import convert


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--store", type=Path, required=True)
    parser.add_argument("--report", required=True)
    parser.add_argument("--server", type=lambda value: Path(value).resolve(), required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()

    try:
        oracle = verified_oracle()
    except ReplayUnsupported as error:
        parser.error(str(error))
    report, envelope = load_report(args.store, args.report)
    if any(envelope.get(key) != oracle[key] for key in ("oracle_revision", "source_sha256", "rules_version")):
        parser.error("The report uses a different Python rules oracle")
    verification = json.loads(
        (args.store / "verification" / (args.report + ".json")).read_text(encoding="utf-8")
    )
    if (verification["status"], verification["report_id"], verification["decisions"]) != (
        "pass", args.report, len(report["trace"]),
    ):
        parser.error("Choose a report with a matching Python verification pass")
    start = envelope["state"]
    endpoint = report["endpoint"]
    if (start["phase"], start["round"], start["turn"]) != ("PLANNING", 1, 1):
        parser.error("Choose a match that starts in round one, turn one")
    if start["pending_inputs"] or start.get("clock") is not None:
        parser.error("Choose an unplayed, untimed match")
    if endpoint["phase"] != "GAME_OVER" or endpoint["winner_team"] not in ("RED", "BLUE"):
        parser.error("Choose a match that ended under the team victory rules")
    with subprocess.Popen(
        [str(args.server), "--port-lab-situations"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, encoding="utf-8",
    ) as worker:
        checkpoint = convert(worker, envelope, {}, None)["checkpoint"]
        worker.stdin.close()
        if worker.wait(timeout=10):
            raise RuntimeError("Rust Situation port worker failed")

    from goa2.bootstrap import register_all_effects

    register_all_effects()
    python_start, _ = restore_python_start(envelope)
    fixture = {
        # Both engines accept unsigned 64-bit seeds. The browser harness
        # must forward the checkpoint text without rounding it through JS.
        "pythonCheckpointJSON": json.dumps({"state": python_start.model_dump(mode="json")}),
        "reportId": args.report,
        "map": start["board"]["map_id"],
        "checkpoint": checkpoint,
        "endpoint": endpoint,
        "trace": report["trace"],
        "verified": verification["progress"],
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    with args.out.open("x", encoding="utf-8") as output:
        json.dump(fixture, output)
    print("Packaged", fixture["map"], "with", len(fixture["trace"]), "verified decisions")


if __name__ == "__main__":
    main()
