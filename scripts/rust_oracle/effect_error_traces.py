"""Export the pinned Python adjacency rejection trace."""

import argparse
import json
from pathlib import Path

from effect_adjacency_traces import cases
from fixture_cache import write_text
from oracle import digest, verify_sources


def export(path: Path):
    manifest = verify_sources()
    rejected = cases({"kind": "advance"}, None, rejected=True)
    if [(case["name"], case["reference_error"]) for case in rejected] != [
        ("effect-adjacency-shield-radius", "RecursionError")
    ]:
        raise RuntimeError("The pinned adjacency rejection was not reproduced")
    # The admitted error fixture predates instance equality metadata.
    for instance in rejected[0]["envelope"]["instances"]:
        instance.pop("equality", None)
    data = {"schema_version": 1, "source_sha256": manifest["source_sha256"], "cases": rejected}
    data["corpus_sha256"] = digest(data)
    write_text(path, json.dumps(data, separators=(",", ":")) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    export(parser.parse_args().output)
