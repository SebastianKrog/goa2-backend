"""Run a Rust export or arena command against isolated, pinned Python sources."""

import argparse
import json
import os
import shutil
import signal
import subprocess
import sys
from pathlib import Path
from tempfile import TemporaryDirectory

from fixture_cache import COMMITTED
from oracle import MANIFEST, ORACLE_REVISION, ROOT, SOURCE_ROOTS
from pinned_source import archive_bytes, extract_verified


def copy_fixture_metadata(source_root, destination_root):
    source = source_root / "rust/tests/fixtures"
    destination = destination_root / "rust/tests/fixtures"
    destination.mkdir(parents=True)
    copied = []
    for name in sorted(COMMITTED):
        path = source / name
        if path.is_file():
            shutil.copyfile(path, destination / name)
            copied.append(name)
    return copied


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "command",
        choices=(
            "arena_calibrate",
            "arena_profiles",
            "arena_queue",
            "arena_verify",
            "lab_match_fixture",
            "live_game_content",
            "live_game_views",
            "port_lab_situations",
            "simulation_bench",
            "rollout_parity",
            "action_rule_traces",
            "action_traces",
            "card_equality_traces",
            "displacement_effect_traces",
            "effect_stat_traces",
            "effect_traces",
            "mine_traces",
            "model_traces",
            "movement_sequence_traces",
            "object_arena_traces",
            "object_execution_traces",
            "oracle",
            "planning_traces",
            "stack_traces",
            "state_codec",
            "streamed_games",
            "targeting_rules",
            "terrain_traces",
        ),
    )
    parser.add_argument("arguments", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    archive = archive_bytes(ROOT, MANIFEST, ORACLE_REVISION, SOURCE_ROOTS)
    with TemporaryDirectory(prefix="goa2-pinned-oracle-") as temp:
        root = Path(temp)
        extract_verified(archive, root, json.loads(MANIFEST.read_text()))

        # Retain the current bridge scripts and committed manifest. Their
        # existing source and runtime verification runs inside this tree.
        scripts = root / "scripts" / "rust_oracle"
        scripts.mkdir(parents=True)
        for script in Path(__file__).resolve().parent.glob("*.py"):
            shutil.copyfile(script, scripts / script.name)
        manifest = root / "rust" / MANIFEST.name
        manifest.parent.mkdir()
        shutil.copyfile(MANIFEST, manifest)

        # Copy only bounded, committed index/bootstrap metadata. Corpus payloads
        # remain in GOA2_FIXTURE_CACHE and are read one case at a time.
        copy_fixture_metadata(ROOT, root)
        (root / "rust/artifacts").mkdir()

        env = {**os.environ, "PYTHONPATH": str(root / "src"), "PYTHONHASHSEED": "0"}
        command = [sys.executable, str(scripts / (args.command + ".py"))]
        if args.command == "arena_queue":
            command.extend(("--repo", str(ROOT)))
        with subprocess.Popen([*command, *args.arguments], env=env) as child:
            previous = signal.signal(signal.SIGTERM, lambda signum, _: child.send_signal(signum))
            try:
                status = child.wait()
            finally:
                signal.signal(signal.SIGTERM, previous)
        return status if status >= 0 else 128 - status


if __name__ == "__main__":
    raise SystemExit(main())
