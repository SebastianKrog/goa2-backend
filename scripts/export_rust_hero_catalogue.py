"""Export the public registry catalogue embedded in the Rust HTTP server."""

import json

from goa2.bootstrap import register_all_effects
from goa2.data.heroes.registry import HeroRegistry


if __name__ == "__main__":
    register_all_effects()
    heroes = HeroRegistry.list_heroes(include_playtest=True)
    released = set(HeroRegistry.list_heroes())
    print(json.dumps({
        "heroes": heroes,
        "playtest_heroes": [name for name in heroes if name not in released],
        "metadata": HeroRegistry.list_hero_metadata(include_playtest=True),
    }, indent=2, ensure_ascii=False))
