"""Run a Rust export or arena command against isolated, pinned Python sources."""

import argparse
import io
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tarfile
from tempfile import TemporaryDirectory

from oracle import MANIFEST, ORACLE_REVISION, ROOT, SOURCE_ROOTS, git


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=(
        "arena_calibrate", "arena_profiles", "arena_queue", "arena_verify",
        "lab_match_fixture", "live_game_content", "live_game_views", "port_lab_situations",
    ))
    parser.add_argument("arguments", nargs=argparse.REMAINDER)
    args = parser.parse_args()

    archive = git("archive", "--format=tar", ORACLE_REVISION, *SOURCE_ROOTS)
    with TemporaryDirectory(prefix="goa2-pinned-oracle-") as temp:
        root = Path(temp)
        with tarfile.open(fileobj=io.BytesIO(archive)) as source:
            source.extractall(root, filter="data")

        # Retain the current bridge scripts and committed manifest. Their
        # existing source and runtime verification runs inside this tree.
        scripts = root / "scripts" / "rust_oracle"
        scripts.mkdir(parents=True)
        for script in Path(__file__).resolve().parent.glob("*.py"):
            shutil.copyfile(script, scripts / script.name)
        manifest = root / "rust" / MANIFEST.name
        manifest.parent.mkdir()
        shutil.copyfile(MANIFEST, manifest)

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
