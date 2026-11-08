"""Generate versioned LONG arena profiles from reproducible setup recipes."""

import argparse
import json
from hashlib import sha256
from pathlib import Path
from random import Random

from arena_acceptance import confirmation_pair_counts, validate_acceptance
from arena_reports import _write, sha256_file
from oracle import ROOT, canonical, digest

MAPS = ("forgotten_island", "across_the_river", "magma_and_ice",
        "narrow_passages", "vexing_cliffs", "vexing_cliffs_v2")
PREFLIGHT = (
    ("narrow_passages", 919, ["Arien", "Hanu"], ["Brogan", "Tigerclaw"]),
    ("magma_and_ice", 929, ["Min", "Trinkets"], ["Misa", "Gydion"]),
)


def generated_starts(name, seed, count, heroes):
    for index in range(count):
        material = f"goa2:arena_starts_v1:{name}:{seed}:{index}".encode("utf-8")
        rng = Random(int.from_bytes(sha256(material).digest(), "big"))
        roster = rng.sample(heroes, 4)
        yield MAPS[index % len(MAPS)], rng.getrandbits(64), roster[:2], roster[2:]


def generate_profile(template, output):
    from goa2.bootstrap import register_all_effects
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.engine.setup import GameSetup

    from rollout_parity import STRENGTH_SCREEN_STARTS
    from shared_objects import export_state_with_shared_objects
    from state_codec import verified_instance_exporter

    register_all_effects()
    config = template["acceptance"]
    preflight = config["preflight_pairs"]
    screen = config["screen_pairs"]
    extend = sum(config["extend_batches"])
    confirm = config["confirm_pairs"]
    validate_acceptance(config, range(preflight + screen + extend + confirm))
    if preflight > len(PREFLIGHT) or screen > len(STRENGTH_SCREEN_STARTS):
        raise ValueError("requested more fixed preflight or screen starts than available")
    if template["parameters"]["turns"] != 64:
        raise ValueError("strength profiles require the 64-turn cap")
    seeds = [template[key] for key in ("tuning_seed", "holdout_seed")]
    if any(type(seed) is not int or not 0 <= seed < 2**64 for seed in seeds):
        raise ValueError("recipe seeds must be unsigned 64-bit integers")
    heroes = sorted(HeroRegistry.list_heroes())
    cases = [
        *PREFLIGHT[:preflight], *STRENGTH_SCREEN_STARTS[:screen],
        *generated_starts("extend", seeds[0], extend, heroes),
        *generated_starts("confirm", seeds[1], confirm, heroes),
    ]
    instance_exporter = verified_instance_exporter()
    paths = []
    for map_id, seed, red, blue in cases:
        state = GameSetup.create_game(
            str(ROOT / "src/goa2/data/maps" / f"{map_id}.json"),
            red, blue, seed=seed, game_type="LONG",
        )
        envelope, _ = export_state_with_shared_objects(state, instance_exporter=instance_exporter)
        identity = digest(envelope)
        relative = f"starts/{identity}.json"
        _write(output / relative, canonical(envelope).encode("utf-8"))
        paths.append({"envelope": relative, "seed": seed})
    if len({row["envelope"] for row in paths}) != len(paths):
        raise ValueError("arena recipe generated duplicate starts")
    profile = {
        "schema_version": template.get("schema_version", 2),
        "name": template["name"], "parameters": template["parameters"],
        "acceptance": config, "starts": paths,
        "recipe": {"version": "arena_starts_v1", "tuning_seed": seeds[0], "holdout_seed": seeds[1]},
    }
    if "budget_calibration" in template:
        profile["budget_calibration"] = template["budget_calibration"]
    path = output / "profile.json"
    _write(path, canonical(profile).encode("utf-8"))
    return path


def apply_calibrated_tier(template, path, target_ms):
    calibration = json.loads(path.read_text(encoding="utf-8"))
    if template.get("schema_version", 2) != 2 or calibration.get("protocol") != "nodes_v2":
        raise ValueError("calibrated node tiers require arena v2")
    tier = next((row for row in calibration["node_tiers"] if row["target_ms"] == target_ms), None)
    if tier is None:
        raise ValueError(f"no calibration tier for {target_ms} ms")
    for phase in ("planning", "resolution"):
        key = f"{phase}_nodes"
        value = tier[key]
        if type(value) is not int or value <= 0:
            raise ValueError(f"invalid {phase} node count")
        template["parameters"][key] = value
    template["budget_calibration"] = {
        "target_ms": target_ms, "report_sha256": sha256_file(path),
        "worker_sha256": calibration["worker_sha256"],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--template", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--calibration", type=Path)
    parser.add_argument("--target-ms", type=int)
    parser.add_argument("--assumed-advantage", type=float)
    parser.add_argument("--power", type=float, default=0.8,
                        help="target power for the Hoeffding sizing bound")
    args = parser.parse_args()
    if (args.calibration is None) != (args.target_ms is None):
        parser.error("--calibration and --target-ms must be supplied together")
    if args.output is None and args.assumed_advantage is None:
        parser.error("provide --output, --assumed-advantage, or both")
    try:
        template = json.loads(args.template.read_text(encoding="utf-8"))
        if args.calibration is not None:
            apply_calibrated_tier(template, args.calibration, args.target_ms)
        if args.assumed_advantage is not None:
            print(canonical(confirmation_pair_counts(template["acceptance"], args.assumed_advantage, args.power)))
        if args.output is not None:
            print(generate_profile(template, args.output))
    except (KeyError, ValueError, OSError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
