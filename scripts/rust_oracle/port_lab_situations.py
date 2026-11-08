"""Stage Rust ports of Layout Lab Situations after exact frontend parity checks."""

from __future__ import annotations

import argparse
import copy
from contextlib import ExitStack
import json
import os
from pathlib import Path
import subprocess
import sys

from shared_objects import export_state_with_shared_objects
from state_codec import verified_instance_exporter


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def difference(expected, actual, path="$"):
    if isinstance(expected, dict) and isinstance(actual, dict):
        missing = sorted(expected.keys() - actual.keys())
        extra = sorted(actual.keys() - expected.keys())
        if missing or extra:
            return f"{path}: missing keys {missing}, extra keys {extra}"
        for key in sorted(expected):
            mismatch = difference(expected[key], actual[key], f"{path}.{key}")
            if mismatch:
                return mismatch
    elif isinstance(expected, list) and isinstance(actual, list):
        if len(expected) != len(actual):
            return f"{path}: Python has {len(expected)} entries, Rust has {len(actual)}"
        for index, (left, right) in enumerate(zip(expected, actual, strict=True)):
            mismatch = difference(left, right, f"{path}[{index}]")
            if mismatch:
                return mismatch
    elif expected != actual or isinstance(expected, bool) != isinstance(actual, bool):
        return f"{path}: Python={repr(expected)[:200]}, Rust={repr(actual)[:200]}"
    return None


def normalize(value):
    value = copy.deepcopy(value)
    if request := value["input_request"]:
        request.pop("request_id", None)
        if request.get("can_rollback") is False:
            request.pop("can_rollback")
    return value


def python_views(saved, *, rewind=False):
    from backend import FROZEN_MS, restore
    from goa2.domain.views import build_view
    from goa2.server.visibility import awaiting_input_hero_ids, input_request_for_viewer

    session, result = restore(saved)
    if rewind:
        result = session.rollback()
    state = session.state
    request = result.input_request if result else None
    roster = [hero.id for team in state.teams.values() for hero in team.heroes]
    names = saved.get("hero_names", {})
    return {
        viewer or "spectator": {
            "view": build_view(state, viewer, now_ms=FROZEN_MS),
            "input_request": input_request_for_viewer(request, state, viewer),
            "awaiting_input": awaiting_input_hero_ids(request, state),
            "winner": state.individual_winner_id or state.winner,
            "hero_names": names,
        }
        for viewer in [None, *roster]
    }



def python_upgrade_preview(saved, hero, cards):
    from backend import FROZEN_MS, restore
    from goa2.domain.views import build_view

    session, _ = restore(saved)
    step = session.state.execution_stack[-1]
    preview = step.preview_choices(session.state, hero, cards)
    return {
        "card_ids": cards,
        "remaining": preview.pending_upgrades.get(hero, 0),
        "options": step._get_upgrade_options(preview, hero) if hero in preview.pending_upgrades else [],
        "view": build_view(preview, hero, now_ms=FROZEN_MS),
    }


def convert(process, envelope, hero_names, rollback):
    packet = {"envelope": envelope, "hero_names": hero_names, "rollback": rollback}
    process.stdin.write(json.dumps(packet) + "\n")
    process.stdin.flush()
    line = process.stdout.readline()
    if not line:
        raise SystemExit("Rust port worker exited before returning a response")
    response = json.loads(line)
    if "error" in response:
        raise ValueError(response["error"])
    return response["result"]


def project(lab):
    # The Lab's Python checkout owns the frontend contract. Run it in its
    # own interpreter so the pinned exporter retains its model classes.
    sys.path.insert(0, str(lab / "src"))
    from goa2.bootstrap import register_all_effects

    register_all_effects()
    for line in sys.stdin:
        packet = json.loads(line)
        try:
            if "preview" in packet:
                hero, cards = packet["preview"]
                result = python_upgrade_preview(packet["saved"], hero, cards)
            else:
                result = python_views(packet["saved"], rewind=packet["rewind"])
            print(json.dumps({"result": result}), flush=True)
        except (ValueError, KeyError, TypeError, RuntimeError) as error:
            print(json.dumps({"error": str(error)}), flush=True)


def audit(lab, binary, out, frontend_python_path=None):
    sys.path.insert(0, str(lab / "src"))
    from backend import FROZEN_MS
    from goa2.bootstrap import register_all_effects
    from goa2.domain.rules_version import CURRENT_RULES_VERSION, THIS_TURN_OUTLASTS_END_OF_TURN
    from goa2.domain.state import GameState
    from goa2.engine import step_types  # noqa: F401 - installs typed stack validation
    from state_model import decision_state

    register_all_effects()
    export = verified_instance_exporter()
    library = lab / "library"
    entries = []
    with ExitStack() as workers:
        process = workers.enter_context(subprocess.Popen(
            [str(binary), "--port-lab-situations"],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, encoding="utf-8",
        ))
        frontend = None
        if frontend_python_path is not None:
            frontend = workers.enter_context(subprocess.Popen(
                [sys.executable, str(Path(__file__).resolve()), "--project", str(lab)],
                env={**os.environ, "PYTHONPATH": str(frontend_python_path)},
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, encoding="utf-8",
            ))

        def projection(saved, **request):
            if frontend is None:
                if "preview" in request:
                    hero, cards = request["preview"]
                    return python_upgrade_preview(saved, hero, cards)
                return python_views(saved, rewind=request.get("rewind", False))
            packet = {"saved": saved, "rewind": False, **request}
            frontend.stdin.write(json.dumps(packet) + "\n")
            frontend.stdin.flush()
            line = frontend.stdout.readline()
            if not line:
                raise RuntimeError("Lab frontend projection worker exited without returning a response")
            response = json.loads(line)
            if "error" in response:
                raise ValueError(response["error"])
            return response["result"]

        def compare_views(expected, actual):
            expected = {seat: normalize(view) for seat, view in expected.items()}
            actual = {seat: normalize(view) for seat, view in actual.items()}
            if frontend is None:
                # The frozen rules oracle predates these frontend fields.
                for seat, view in actual.items():
                    facts = (view["input_request"] or {}).get("decision_context") or {}
                    if facts.get("purpose") == "defense":
                        old = (expected.get(seat, {}).get("input_request") or {}).get("decision_context") or {}
                        for key in ("combat", "source_card_id"):
                            if key not in old:
                                facts.pop(key, None)
            return difference(expected, actual)

        for path in sorted((library / "situations").rglob("*.json")):
            situation = json.loads(path.read_text(encoding="utf-8"))
            source_id = situation["decisionStateId"]
            source = json.loads((library / "decision-states" / (source_id + ".json")).read_text(encoding="utf-8"))
            saved = source["checkpoint"]
            if decision_state(saved)["id"] != source_id:
                raise ValueError(f"Corrupt source Decision State: {source_id}")
            row = {"id": situation["id"], "sourceDecisionStateId": source_id, "name": situation["name"]}
            entries.append(row)
            if saved.get("engine") == "rust":
                row.update(status="native")
                continue
            state = saved["state"]
            rules = state.get("rules_version")
            if rules not in (CURRENT_RULES_VERSION, THIS_TURN_OUTLASTS_END_OF_TURN):
                row.update(status="deferred", reason=f"Requires a rules-v{rules} port")
                continue
            row.update(sourceRulesVersion=rules, targetRulesVersion=CURRENT_RULES_VERSION)
            try:
                expected = projection(saved)
                current = {**state, "rules_version": CURRENT_RULES_VERSION}
                if state.get("clock") is not None:
                    clock = {**state["clock"], "last_settled_at_ms": FROZEN_MS}
                    if clock.get("paused_at_ms") is not None:
                        clock["paused_at_ms"] = FROZEN_MS
                    current["clock"] = clock
                envelope, _ = export_state_with_shared_objects(
                    GameState.model_validate(current), instance_exporter=export,
                )
                rollback = None
                if any((view["input_request"] or {}).get("can_rollback") for view in expected.values()):
                    anchor = {
                        **saved["rollback_snapshot"],
                        "board": state["board"],
                        "rules_version": CURRENT_RULES_VERSION,
                    }
                    rollback, _ = export_state_with_shared_objects(
                        GameState.model_validate(anchor), instance_exporter=export,
                    )
                result = convert(process, envelope, saved.get("hero_names", {}), rollback)
                actual = result["projections"]
                mismatch = compare_views(expected, actual)
                if mismatch:
                    raise ValueError(mismatch)
                if rollback is not None:
                    expected_rollback = projection(saved, rewind=True)
                    actual_rollback = result["rollback_projections"]
                    if mismatch := compare_views(expected_rollback, actual_rollback):
                        raise ValueError("Rollback changes the restored stopping point: " + mismatch)
                    row["rollbackProjections"] = len(actual_rollback)
                if rules != CURRENT_RULES_VERSION:
                    updated = {**saved, "state": current}
                    current_views = projection(updated)
                    if mismatch := compare_views(expected, current_views):
                        raise ValueError("Rules upgrade changes the stopping point: " + mismatch)
                for item in result["upgrade_previews"]:
                    preview = item["preview"]
                    expected_preview = projection(saved, preview=(item["actor"], preview["card_ids"]))
                    actual_preview = {key: value for key, value in preview.items() if key != "request_id"}
                    if mismatch := difference(expected_preview, actual_preview):
                        raise ValueError("Upgrade preview differs: " + mismatch)
                if result["upgrade_previews"]:
                    row["upgradePreviews"] = len(result["upgrade_previews"])
                port = decision_state(result["checkpoint"])
                if "tags" in source:
                    port["tags"] = source["tags"]
                write(out / "decision-states" / (port["id"] + ".json"), port)
                row.update(status="verified", rustDecisionStateId=port["id"], projections=len(actual))
            except (ValueError, KeyError, TypeError, RuntimeError) as error:
                row.update(status="rejected", reason=str(error))
        for worker in (process, frontend):
            if worker is not None:
                worker.stdin.close()
                if worker.wait(timeout=10):
                    raise RuntimeError("A Situation port worker failed")
    write(out / "manifest.json", {"schema": 1, "situations": entries})
    counts = {status: sum(row["status"] == status for row in entries)
              for status in sorted({row["status"] for row in entries})}
    print(json.dumps(counts, sort_keys=True))
    return counts


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--project":
        project(Path(sys.argv[2]))
        return 0
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--lab", required=True, type=lambda value: Path(value).resolve())
    parser.add_argument("--server", required=True, type=lambda value: Path(value).resolve())
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--frontend-python-path", type=lambda value: Path(value).resolve(),
                        help="Lab Python backend src directory for exact current client payloads")
    args = parser.parse_args()
    if args.out.exists() and any(args.out.iterdir()):
        parser.error("use an empty output directory")
    if os.environ.get("PYTHONHASHSEED") != "0":
        parser.error("set PYTHONHASHSEED=0 for reproducible projections")
    counts = audit(args.lab, args.server, args.out, args.frontend_python_path)
    return int(bool(counts.get("rejected")))


if __name__ == "__main__":
    raise SystemExit(main())
