use super::phases::Field;
use super::*;
use crate::{
    data::Registers,
    primary::PrimaryValues,
    spatial::{HexFilter, SpatialContext},
};
fn session(name: &str) -> NativeSession {
    let case = fixture_case(&format!("object-execution-{name}"));
    NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
}
#[test]
fn theft_overflow_is_guarded_before_either_owner_or_the_step_changes() {
    let mut worker = session("tiger-primary-steal-plain");
    let hero = worker.symbols.lookup("hero_tigerclaw").unwrap();
    let model = worker
        .hero_model_bound(worker.objects.as_ref().unwrap(), hero)
        .unwrap();
    let objects = worker.objects.as_mut().unwrap();
    objects
        .arena
        .set_field(
            model,
            objects.phases.k(Field::Gold),
            ObjectValue::Atom(Datum::Int(i64::MAX)),
        )
        .unwrap();
    let before = worker.export_envelope().unwrap();
    assert!(
        worker
            .advance_prefix(1)
            .unwrap_err()
            .to_string()
            .contains("steal actor gold outside i64")
    );
    let after = worker.export_envelope().unwrap();
    assert_eq!(before.state, after.state);
    assert_eq!(before.object_bindings, after.object_bindings);
}
#[test]
fn theft_reads_live_actor_override_and_keeps_the_root_private() {
    let root = session("tiger-primary-steal-plain");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let actor = worker.symbols.lookup("hero_hanu").unwrap();
    let tiger = worker.symbols.lookup("hero_tigerclaw").unwrap();
    let objects = worker.objects.as_mut().unwrap();
    objects.write(objects.actor, Datum::Text(actor)).unwrap();
    let result = worker.advance_prefix(1).unwrap();
    assert!(
        matches!(result.events.first(),Some(NativeEvent::CoinsStolen{actor:id,..}) if *id==actor)
    );
    assert_eq!(worker.world.hero(actor).unwrap().gold, 1);
    assert_eq!(worker.world.hero(tiger).unwrap().gold, 2);
    assert_eq!(root.export_envelope().unwrap().state, before.state);
    assert_eq!(
        root.export_envelope().unwrap().object_bindings,
        before.object_bindings
    );
}
#[test]
fn terrain_filter_reads_live_wanted_value_without_mutating_root_models() {
    let root = session("tiger-primary-terrain-UNIT-True-False");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let key = worker.symbols.lookup("terrain_filter").unwrap();
    let objects = worker.objects.as_mut().unwrap();
    let Some(ObjectValue::Object(filter)) = objects.arena.context_value(key) else {
        panic!("filter alias missing")
    };
    objects
        .arena
        .set_field(
            filter,
            objects.phases.k(Field::IsTerrain),
            ObjectValue::Atom(Datum::Bool(false)),
        )
        .unwrap();
    assert!(worker.advance_prefix(1).unwrap().request.is_none());
    assert_eq!(root.export_envelope().unwrap().state, before.state);
}
#[test]
fn terrain_filter_matches_source_virtual_edge_and_physical_tiles() {
    let worker = session("tiger-primary-terrain-HEX-True-False");
    let registers = Registers::default();
    let context = SpatialContext {
        count_observer: None,
        world: &worker.world,
        registers: &registers,
        symbols: &worker.symbols,
        actor: worker.planning.actor,
        frame: None,
        coordinates: worker.coordinates,
        range_override: None,
        through_override: None,
    };
    let points = [
        Hex::new(99, 0, -99).unwrap(),
        Hex::new(0, -1, 1).unwrap(),
        Hex::new(-1, 0, 1).unwrap(),
    ];
    for wanted in [true, false] {
        let mut accepted = [true; 3];
        HexFilter::Terrain(wanted)
            .apply_scan(&context, &points, &mut accepted)
            .unwrap();
        assert_eq!(accepted, [wanted, wanted, !wanted]);
    }
}
#[test]
fn shared_radius_defaults_bind_execution_nullable_fields_and_exported_wires() {
    let worker = session("primary-text-blend_into_shadows-ordinary");
    let slot = crate::primary::catalog().unwrap()["radius_fallbacks"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, v)| **v == serde_json::json!(2))
        .unwrap()
        .0
        .parse::<i64>()
        .unwrap();
    for radius in [None, Some(0), Some(3), Some(-1)] {
        let expected = radius.filter(|v| *v != 0).unwrap_or(2);
        let values = PrimaryValues {
            hero: worker.planning.actor.unwrap(),
            card: worker.symbols.lookup("blend_into_shadows").unwrap(),
            name: worker
                .symbols
                .lookup("Tigerclaw")
                .unwrap_or(worker.planning.actor.unwrap()),
            stats: CardStats {
                primary_value: 0,
                range: 1,
                radius,
            },
            ranged: false,
        };
        assert_eq!(
            worker.primary_operands.number(slot, &values).unwrap(),
            expected
        );
        let mut nullable = Some(crate::primary::RADIUS);
        worker
            .primary_operands
            .optional_number(&mut nullable, &values)
            .unwrap();
        assert_eq!(nullable, radius);
        let mut wire = serde_json::json!({"radius":slot,"nested":[slot]});
        values.patch_wire(&mut wire, &worker.symbols, &worker.primary_operands);
        assert_eq!(
            wire,
            serde_json::json!({"radius":expected,"nested":[expected]})
        );
    }
}
#[test]
fn dense_blend_absent_radius_offers_two_space_destinations_and_exports_the_value() {
    let case =
        fixture_case("object-execution-tiger-primary-edge-blend_into_shadows-radius-none-text");
    let mut envelope: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    envelope.object_bindings.clear();
    envelope.board_keys = None;
    envelope.state["execution_context"] = serde_json::json!({});
    let mut worker = NativeSession::compile(envelope).unwrap();
    let request = worker
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let RequestChoices::Hexes(choices) = request.choices else {
        panic!("hex choices expected")
    };
    let origin = Hex::new(0, 0, 0).unwrap();
    assert!(choices.iter().all(|point| origin.distance(*point) <= 2));
    assert!(choices.iter().any(|point| origin.distance(*point) == 2));
    let state = worker.semantic_state().unwrap();
    let step = state["execution_stack"].as_array().unwrap().last().unwrap();
    assert_eq!(step["filters"][0]["max_range"], 2);
}
