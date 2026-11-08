"""Build the versioned static content used by Rust to create fresh games."""

import argparse
import gzip
import json
import os
from pathlib import Path

from oracle import ROOT, verify_sources
from shared_objects import export_state_with_shared_objects
from state_codec import InstanceEncoder


def content():
    from goa2.bootstrap import register_all_effects
    from goa2.data.heroes.registry import HeroRegistry
    from goa2.domain.factory import EntityFactory
    from goa2.domain.models import GamePhase, MinionType, Team, TeamColor, Token, TokenType
    from goa2.domain.models.token import HERO_TOKEN_TYPES, TOKEN_SUPPLY
    from goa2.domain.state import GameState
    from goa2.engine.map_loader import load_map
    from goa2.engine.setup import GameSetup

    manifest = verify_sources()
    register_all_effects()

    def blank(board):
        return GameState(
            board=board,
            teams={team: Team(color=team, heroes=[], minions=[], life_counters=0)
                   for team in TeamColor},
            phase=GamePhase.PLANNING,
            rules_version=manifest["rules_version"],
        )

    maps = {}
    for path in sorted((ROOT / "src/goa2/data/maps").glob("*.json")):
        envelope, _ = export_state_with_shared_objects(blank(load_map(path)))
        maps[path.stem] = envelope

    board = load_map(ROOT / "src/goa2/data/maps/forgotten_island.json")
    heroes = {}
    for name in sorted(HeroRegistry.list_heroes(include_playtest=True)):
        state = blank(board.model_copy(deep=True))
        GameSetup._setup_team(state, TeamColor.RED, [name])
        envelope, _ = export_state_with_shared_objects(state)
        source = envelope["state"]
        hero_path = ["teams", "RED", "heroes", 0]
        bindings = []
        for binding in envelope["object_bindings"]:
            paths = [path for path in binding["paths"]
                     if path[:4] == hero_path or (len(path) > 1 and path[0] == "misc_entities")]
            if paths:
                bindings.append({**binding, "paths": paths})
        heroes[name] = {
            "model": source["teams"]["RED"]["heroes"][0],
            "pieces": source["misc_entities"],
            "bindings": bindings,
            "definitions": envelope["definitions"],
            "instances": envelope["instances"],
            "token_types": [kind.value for kind in TokenType
                            if kind in HERO_TOKEN_TYPES.get(name, ())],
        }

    tokens = []
    for kind in TokenType:
        count = TOKEN_SUPPLY.get(kind, 0)
        if not count:
            continue
        is_mine = kind in (TokenType.MINE_BLAST, TokenType.MINE_DUD)
        encoder = InstanceEncoder()
        encoder.pack_token(Token(
            id="setup_template",
            name=kind.value.replace("_", " ").title(),
            token_type=kind,
            is_passable=is_mine,
            is_facedown=is_mine,
            persists_end_of_round=kind in (TokenType.ZOMBIE, TokenType.PYRO, TokenType.TREE),
        ))
        tokens.append({"kind": kind.value, "count": count,
                       "definition": encoder.definitions[0], "instance": encoder.instances[0]})

    minions = {team.value: {
        kind.value: EntityFactory.create_minion(blank(board), team, kind).model_dump(mode="json")
        for kind in MinionType
    } for team in TeamColor}
    return {
        "version": 1,
        "oracle_revision": manifest["oracle_revision"],
        "source_sha256": manifest["source_sha256"],
        "maps": maps,
        "heroes": heroes,
        "tokens": tokens,
        "minions": minions,
    }


def parity_cases(pack):
    from goa2.domain.models import TeamColor
    from goa2.engine.setup import GameSetup

    maps = list(pack["maps"])
    names = list(pack["heroes"])
    setups = []
    for name in maps:
        for mode in ("QUICK", "LONG"):
            seed = len(setups)
            setups.append({
                "map": name,
                "red_heroes": ["Arien", "Min"],
                "blue_heroes": ["Razzle", "Gydion"],
                "seed": seed,
                "mode": mode,
                "cheats_enabled": bool(seed % 2),
            })
    for index in range(0, len(names), 4):
        roster = names[index:index + 4]
        unused = [name for name in names if name not in roster]
        roster.extend(unused[:4 - len(roster)])
        setups.append({
            "map": maps[index % len(maps)],
            "red_heroes": roster[:2],
            "blue_heroes": roster[2:],
            "seed": 2**64 - 1 - index,
            "mode": "LONG",
        })
    setups.append({
        "map": "across_the_river",
        "red_heroes": names[:5],
        "blue_heroes": names[5:10],
        "seed": 999,
        "mode": "LONG",
        "tie_breaker": "BLUE",
    })
    for size in (2, 5, 6):
        for mode in ("QUICK", "LONG"):
            setups.append({
                "map": "forgotten_island",
                "red_heroes": names[:size // 2],
                "blue_heroes": names[size // 2:size],
                "seed": 1200 + size,
                "mode": mode,
            })
    for setup in setups:
        path = ROOT / "src/goa2/data/maps" / f"{setup['map']}.json"
        tie = TeamColor(setup["tie_breaker"]) if "tie_breaker" in setup else None
        state = GameSetup.create_game(
            path, setup["red_heroes"], setup["blue_heroes"], setup.get("cheats_enabled", False),
            game_type=setup["mode"], seed=setup["seed"], tie_breaker_team=tie,
        )
        envelope, _ = export_state_with_shared_objects(state)
        yield {"setup": setup, "expected": envelope}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--parity-fixtures", type=Path)
    args = parser.parse_args()
    if os.environ.get("PYTHONHASHSEED") != "0":
        parser.error("run with PYTHONHASHSEED=0")
    pack = content()
    args.output.write_text(json.dumps(pack, separators=(",", ":"), ensure_ascii=False),
                           encoding="utf-8")
    print(f"Exported {len(pack['maps'])} maps and {len(pack['heroes'])} heroes")
    if args.parity_fixtures:
        with args.parity_fixtures.open("wb") as target:
            with gzip.GzipFile(filename="", mode="wb", fileobj=target, mtime=0) as archive:
                count = 0
                for case in parity_cases(pack):
                    row = json.dumps(case, separators=(",", ":"), ensure_ascii=False)
                    archive.write((row + "\n").encode("utf-8"))
                    count += 1
        print(f"Exported {count} fresh game parity cases")


if __name__ == "__main__":
    main()
