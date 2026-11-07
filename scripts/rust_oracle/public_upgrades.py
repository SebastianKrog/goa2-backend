"""Collect public ordinary card-item checkpoints at revealed upgrade boundaries."""

from copy import deepcopy

from goa2.domain.card_knowledge import has_standard_loadout_provenance
from goa2.domain.models import GamePhase, StatType

ITEM_STATS = (
    StatType.ATTACK, StatType.DEFENSE, StatType.MOVEMENT,
    StatType.INITIATIVE, StatType.RANGE, StatType.RADIUS,
)


class PublicUpgradeRecorder:
    """Per-game evidence derived from the reveal snapshot and visible totals."""

    def __init__(self):
        self.batches = []
        self._levels = {}
        self._ordinary_items = {}
        self._pending = {}
        self._revision = None

    def checkpoint(self, game_id):
        """Return a JSON-compatible snapshot for this game's trusted store."""
        return deepcopy({
            "version": 1, "game_id": game_id, "revision": self._revision,
            "batches": self.batches, "levels": self._levels,
            "ordinary_items": self._ordinary_items, "pending": self._pending,
        })

    @classmethod
    def restore(cls, checkpoint, game_id):
        if checkpoint.get("version") != 1 or checkpoint.get("game_id") != game_id:
            raise ValueError("upgrade checkpoint does not match the game or schema")
        saved = deepcopy(checkpoint)
        recorder = cls()
        recorder._revision = saved["revision"]
        recorder.batches = saved["batches"]
        recorder._levels = saved["levels"]
        recorder._ordinary_items = saved["ordinary_items"]
        recorder._pending = saved["pending"]
        return recorder

    def observe(self, state, revision):
        if self._revision is not None and revision < self._revision:
            raise ValueError("upgrade observation revision moved backwards")
        self._revision = revision
        for team in state.teams.values():
            for hero in team.heroes:
                if not has_standard_loadout_provenance(hero):
                    continue
                hero_id = hero.id
                if state.phase == GamePhase.LEVEL_UP:
                    before = state.upgrade_reveal_snapshot.get(hero_id)
                    if before is not None and hero_id in self._ordinary_items:
                        self._pending.setdefault(
                            hero_id, {
                                "level": hero.level, "round": state.round,
                                "items": [before.get(stat, 0) for stat in ITEM_STATS],
                            },
                        )
                    continue

                previous_level = self._levels.get(hero_id)
                pending = self._pending.pop(hero_id, None)
                old_items = self._ordinary_items.get(hero_id)
                new_items = None
                if old_items is not None and previous_level == 7 and hero.level == 8:
                    # No ordinary item is awarded at the automatic unlock.
                    new_items = list(old_items)
                elif (
                    previous_level is not None and hero.level > previous_level
                    and old_items is not None
                    and pending is not None and pending["level"] == hero.level
                    and 0 <= state.round - pending["round"] <= 1
                ):
                    delta = tuple(
                        hero.items.get(stat, 0) - old
                        for stat, old in zip(ITEM_STATS, pending["items"])
                    )
                    chosen = min(hero.level, 7) - min(previous_level, 7)
                    if all(change >= 0 for change in delta) and sum(delta) <= chosen:
                        new_items = [old + change for old, change in zip(old_items, delta)]
                if new_items is not None:
                    self.batches.append({
                        "hero_id": hero_id,
                        "revision": revision,
                        "before": {"level": previous_level, "card_items": old_items},
                        "after": {"level": hero.level, "card_items": new_items},
                    })
                    self._ordinary_items[hero_id] = new_items
                elif previous_level is not None and min(hero.level, 7) > min(previous_level, 7):
                    # A missed or inconsistent reveal leaves the cumulative totals unknown.
                    self._ordinary_items.pop(hero_id, None)
                if hero.level == 1:
                    self._ordinary_items[hero_id] = [0] * len(ITEM_STATS)
                self._levels[hero_id] = hero.level
