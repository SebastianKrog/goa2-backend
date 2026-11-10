"""Portable synthetic baseline drivers and allowlisted observations; no credentials/checkpoints are recorded."""

import hashlib
import inspect
import json
import re
import tempfile
from contextlib import ExitStack, contextmanager
from pathlib import Path
from unittest.mock import patch

from . import auth, bots, clocks, process, votes

DRIVERS = {
    "manual_rollback_restart_replay": auth.smoke,
    "bots": bots.bot_smoke,
    "timed_play": clocks.clock_smoke,
    "upgrades": votes.card_patch_vote_smoke,
}


def driver_identity(function):
    return hashlib.sha256(inspect.getsource(function).encode()).hexdigest()


def projection(value):
    """Keep only deliberate observable contract fields, never arbitrary response bodies."""
    if not isinstance(value, dict):
        return {}
    out = {key: value[key] for key in ("type", "revision") if key in value}
    if isinstance(value.get("events"), list):
        out["ordered_event_types"] = [
            event.get("event_type", event.get("type"))
            for event in value["events"]
            if isinstance(event, dict)
        ]
    if isinstance(value.get("view"), dict):
        view = value["view"]
        out["view"] = {
            key: view[key] for key in ("phase", "round", "turn", "current_actor_id") if key in view
        }
        if isinstance(view.get("clock"), dict):
            out["view"]["clock_status"] = view["clock"].get("status")
        out["view"]["heroes"] = [
            {
                "id": hero.get("id"),
                "level": hero.get("level"),
                "gold": hero.get("gold"),
                "hand_count": len(hero.get("hand", [])),
                "current_card": (
                    {
                        key: hero["current_turn_card"][key]
                        for key in ("id", "is_facedown")
                        if key in hero["current_turn_card"]
                    }
                    if isinstance(hero.get("current_turn_card"), dict)
                    else None
                ),
            }
            for team in view.get("teams", {}).values()
            for hero in team.get("heroes", [])
        ]
    request = value.get("input_request")
    if isinstance(request, dict):
        out["request_type"] = request.get("type")
    return out


def command_projection(value):
    if not isinstance(value, dict):
        return None
    out = {
        key: value[key]
        for key in ("type", "card_id", "hero_id", "amount", "ready", "expected_revision")
        if key in value
    }
    if "selection" in value:
        selection = value["selection"]
        if isinstance(selection, str):
            out["selection"] = (
                selection
                if selection in {"HOLD", "CONFIRM", "PASS", "SKIP", "NO", "YES"}
                else "<typed-choice>"
            )
        elif isinstance(selection, dict):
            out["selection"] = {
                key: selection[key]
                for key in ("hero_id", "card_id", "card_ids")
                if key in selection
            }
        else:
            out["selection_type"] = type(selection).__name__
    return out


class Recorder:
    def __init__(self):
        self.rows = []
        self.games = {}

    def path(self, path):
        def replace(match):
            name = match.group(0)
            return self.games.setdefault(name, f"game-{len(self.games)+1}")

        return re.sub(r"rust-[a-f0-9]{32}", replace, path.split("?", 1)[0])

    def observe(self, **row):
        self.rows.append(row)


def capture(function, binary, content, database):
    recorder = Recorder()
    original_request, original_sockets = process.request, process.sockets

    def request(server, path, **kwargs):
        result = original_request(server, path, **kwargs)
        recorder.observe(
            transport="HTTP",
            path=recorder.path(path),
            method=kwargs.get("method") or ("POST" if kwargs.get("body") is not None else "GET"),
            command=command_projection(kwargs.get("body")),
            expected_status=kwargs.get("expect", 200),
            response=projection(result),
        )
        return result

    @contextmanager
    def sockets(*args, **kwargs):
        with original_sockets(*args, **kwargs) as socket:

            class Observed:
                def send(self, payload):
                    recorder.observe(
                        transport="WS-SEND", command=command_projection(json.loads(payload))
                    )
                    return socket.send(payload)

                def recv(self, *args, **kwargs):
                    payload = socket.recv(*args, **kwargs)
                    recorder.observe(transport="WS-RECV", response=projection(json.loads(payload)))
                    return payload

                def __getattr__(self, name):
                    return getattr(socket, name)

            yield Observed()

    modules = (auth, bots, clocks, votes, process)
    with ExitStack() as stack:
        for module in modules:
            if hasattr(module, "request"):
                stack.enter_context(patch.object(module, "request", request))
            if hasattr(module, "sockets"):
                stack.enter_context(patch.object(module, "sockets", sockets))
        function(binary, content, database)
    assert recorder.rows
    return recorder.rows


def verify_contract(case, rows):
    responses = [row["response"] for row in rows if "response" in row]
    for path in case["required_routes"]:
        assert any(path in row.get("path", "") for row in rows), (case["id"], path)
    for phase in case.get("required_phases", []):
        assert any(row.get("view", {}).get("phase") == phase for row in responses), (
            case["id"],
            phase,
        )
    if case["id"] == "upgrades":
        assert any(row.get("request_type") == "UPGRADE_PHASE" for row in responses)
        assert any(
            hero.get("level", 0) >= 2
            for row in responses
            for hero in row.get("view", {}).get("heroes", [])
        )
    if case["id"] == "timed_play":
        assert any(row.get("view", {}).get("clock_status") == "RUNNING" for row in responses)


def run(binary, content, manifest):
    if hashlib.sha256(content.read_bytes()).hexdigest() != manifest["content_sha256"]:
        raise ValueError("Baseline content identity mismatch")
    results = []
    with tempfile.TemporaryDirectory(prefix="goa2-baselines-") as temp:
        for case in manifest["cases"]:
            function = DRIVERS[case["driver"]]
            if driver_identity(function) != case["driver_sha256"]:
                raise ValueError(
                    f"Baseline driver changed: {case['driver']}; review and update its recorded identity"
                )
            rows = capture(function, binary, content, Path(temp) / (case["id"] + ".sqlite"))
            verify_contract(case, rows)
            results.append(
                {
                    "id": case["id"],
                    "covered": case["covered"],
                    "observations": rows,
                    "status": "passed",
                }
            )
    return {
        "schema_version": 1,
        "scope": "Synthetic scoped observation baseline; predicates live in admitted drivers. Poll counts and timing are not frozen oracle output.",
        "content_sha256": manifest["content_sha256"],
        "oracle_revision": manifest["oracle_revision"],
        "source_sha256": manifest["source_sha256"],
        "cases": results,
    }
