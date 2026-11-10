"""Run native service contracts against disposable processes and databases."""

import argparse
import json
import tempfile
from pathlib import Path

from rust_verification import auth, bots, clocks, shutdown, votes

# Compatibility exports used by the hosted/replay-share verifiers.
from rust_verification.process import (
    backup_database as backup_database,
)
from rust_verification.process import (
    hero as hero,
)
from rust_verification.process import (
    packet as packet,
)
from rust_verification.process import (
    request as request,
)
from rust_verification.process import (
    serve as serve,
)
from rust_verification.process import (
    sockets as sockets,
)

SUITES = {
    "auth": (
        auth.smoke,
        auth.deployment_smoke,
    ),
    "bots": (
        bots.bot_smoke,
        bots.timed_bot_smoke,
        bots.reopened_bot_smoke,
    ),
    "clocks": (
        clocks.clock_smoke,
        clocks.pause_vote_smoke,
    ),
    "votes": (
        votes.rewind_vote_smoke,
        votes.unstick_vote_smoke,
        votes.patch_vote_smoke,
        votes.board_patch_vote_smoke,
        votes.card_patch_vote_smoke,
        votes.effect_patch_vote_smoke,
    ),
    "shutdown": (shutdown.shutdown_smoke,),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("content", type=Path)
    parser.add_argument(
        "--suite",
        choices=sorted(SUITES),
        action="append",
        help="Repeat to select suites; default: all",
    )
    args = parser.parse_args()
    selected = list(dict.fromkeys(args.suite or SUITES))
    reports = []
    with tempfile.TemporaryDirectory(prefix="goa2-server-contract-") as root:
        for name in selected:
            functions = SUITES[name]
            for function in functions:
                function(
                    args.binary.resolve(),
                    args.content.resolve(),
                    Path(root) / (function.__name__ + ".sqlite"),
                )
            reports.append(
                {
                    "suite": name,
                    "expected": len(functions),
                    "executed": len(functions),
                    "failed": 0,
                    "missing": 0,
                }
            )
    print(json.dumps(reports, indent=2))


if __name__ == "__main__":
    main()
