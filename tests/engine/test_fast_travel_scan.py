"""Prepared fast-travel scans reuse invariants only while state is unchanged."""
from unittest.mock import patch

from goa2.bootstrap import register_all_effects
from goa2.domain.models import TeamColor
from goa2.engine.filters_hex import FastTravelDestinationFilter
from goa2.engine.setup import GameSetup
import goa2.engine.rules as rules


def test_fast_travel_scan_shares_checks_and_reprepares_after_enemy_moves():
    register_all_effects()
    state = GameSetup.create_game("src/goa2/data/maps/forgotten_island.json",["Wasp"],["Arien"],seed=7)
    actor = state.teams[TeamColor.RED].heroes[0]
    destination_filter = FastTravelDestinationFilter(unit_id=str(actor.id))
    context = state.execution_context
    expected = [h for h in state.board.tiles if destination_filter.apply(h,state,context)]
    assert expected
    validator_type = type(state.validator)
    original = validator_type.can_fast_travel
    checks = []
    def counted(self,*args,**kwargs):
        checks.append(1)
        return original(self,*args,**kwargs)
    with patch.object(validator_type,"can_fast_travel",counted), patch.object(
        rules,"get_safe_zones_for_fast_travel",wraps=rules.get_safe_zones_for_fast_travel
    ) as safe:
        accepts = destination_filter.prepare(state,context)
        assert [h for h in state.board.tiles if accepts(h)] == expected
        assert not accepts("not a hex")
        assert len(checks) == 1
        assert safe.call_count == 1
    actor_zone = state.board.get_zone_for_hex(state.get_position(str(actor.id)))
    invaded = next(h for h in expected if state.board.get_zone_for_hex(h) == actor_zone)
    enemy = state.teams[TeamColor.BLUE].heroes[0]
    state.move_unit(str(enemy.id),invaded)
    # Reusing the filter object does not reuse any prepared result.
    assert not any(destination_filter.prepare(state,context)(h) for h in state.board.tiles)
    assert not destination_filter.apply(expected[0],state,context)


def test_plain_topology_queries_do_not_resolve_occupants_without_topology_effects():
    from goa2.engine.topology import get_topology_service
    register_all_effects()
    state = GameSetup.create_game("src/goa2/data/maps/forgotten_island.json",["Wasp"],["Arien"],seed=7)
    assert not state.active_effects
    first,second = list(state.board.tiles)[:2]
    topology = get_topology_service()
    with patch.object(topology,"_query_unit_ids",side_effect=AssertionError("unnecessary unit scan")):
        assert topology.are_connected(first,second,state)
        assert topology.distance(first,second,state) == first.distance(second)
