"""Public progress at arena decision boundaries."""

class StallClock:
    def __init__(self, position):
        self.turn = self.start = self._turn(position)
        self.last_progress = dict.fromkeys(("combat", "lane"), self.turn)
        self.longest = dict.fromkeys(self.last_progress, 0)
        self.first_lane_progress = None
        self.snapshot = self._public_progress(position["envelope"]["state"])

    @staticmethod
    def _turn(position):
        return position["round"] * 4 + position["turn"]

    @staticmethod
    def _public_progress(state):
        locations = state["entity_locations"]
        teams = tuple(state["teams"][side] for side in ("RED", "BLUE"))
        minions = frozenset(unit["id"] for team in teams for unit in team["minions"]
                            if locations.get(unit["id"]) is not None)
        lives = tuple(team["life_counters"] for team in teams)
        return state["battle_zones"], state["wave_counters"], minions, lives

    def observe(self, position):
        turn = self._turn(position)
        snapshot = self._public_progress(position["envelope"]["state"]
                                         if position["kind"] != "finished" else position["state"])
        zones, waves, minions, lives = snapshot
        old_zones, old_waves, old_minions, old_lives = self.snapshot
        lane = zones != old_zones or waves != old_waves
        combat = (lane or bool(old_minions - minions)
                  or any(now < before for now, before in zip(lives, old_lives, strict=True)))
        if lane and self.first_lane_progress is None:
            self.first_lane_progress = self.turn - self.start + 1
        for kind, progress in (("combat", combat), ("lane", lane)):
            if progress:
                # The turn containing the change does not count as a quiet turn.
                self.last_progress[kind] = max(turn, self.turn + 1)
            self.longest[kind] = max(self.longest[kind], turn - self.last_progress[kind])
        self.turn, self.snapshot = turn, snapshot

    def age(self, kind):
        return max(0, self.turn - self.last_progress[kind])

    def result(self):
        return {"longest_quiet_turns": self.longest,
                "first_lane_progress_turn": self.first_lane_progress}
