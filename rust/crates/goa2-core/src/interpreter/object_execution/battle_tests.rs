use super::*;

use super::test_support::object_session;

#[test]
fn battle_generation_guards_precede_wave_spawn_and_pending_mutations() {
    for name in [
        "object-execution-live-battle-multi-same-direction-first",
        "object-execution-live-battle-casualties-force-over-pending-forced-first",
    ] {
        let root = object_session(name);
        let mut worker = root.clone();
        worker.next_step = u64::MAX;
        let before = worker.export_envelope().unwrap();
        let waves = worker.world.waves.clone();
        let locations = worker.world.locations.clone();
        assert!(
            worker
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("step generation counter exhausted")
        );
        let after = worker.export_envelope().unwrap();
        assert_eq!(before.state, after.state);
        assert_eq!(before.object_bindings, after.object_bindings);
        assert_eq!(waves, worker.world.waves);
        assert_eq!(locations, worker.world.locations);
        assert_eq!(root.export_envelope().unwrap().state, before.state);
    }
}

#[test]
fn battle_noop_with_populated_zones_needs_no_generation_capacity() {
    let root = object_session("object-execution-live-battle-tied-first");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    worker.next_step = u64::MAX;
    worker.advance_prefix(1).unwrap();
    assert_eq!(worker.next_step, u64::MAX);
    assert_eq!(root.export_envelope().unwrap().state, before.state);
}

#[test]
fn live_wave_counter_controls_the_push_without_replacing_retained_maps() {
    let mut worker = object_session("object-execution-live-battle-red-push-first");
    worker.advance_prefix(1).unwrap();
    worker
        .with_live_objects(|session, objects| {
            let waves = session.battle_root_map(objects, "wave_counters")?;
            let lane = session.symbols.lookup("lane_1").unwrap();
            objects
                .arena
                .set_field(waves, lane, ObjectValue::Atom(Datum::Int(1)))
                .map_err(unsupported)
        })
        .unwrap();
    worker.advance_prefix(1).unwrap();
    let result = worker.export_envelope().unwrap();
    assert_eq!(result.state["wave_counters"]["lane_1"], 0);
    assert_eq!(result.state["execution_context"]["wave_alias"]["lane_1"], 0);
    assert_eq!(result.state["battle_zones"]["lane_1"], "lane_1-2");
    let next = result.state["execution_stack"]
        .as_array()
        .unwrap()
        .last()
        .unwrap();
    assert_eq!(next["type"], "trigger_game_over");
    assert_eq!(next["condition"], "LAST_PUSH");
}

#[test]
fn live_removal_count_takes_the_forced_branch_and_keeps_stale_pending_input() {
    let mut worker = object_session("object-execution-live-battle-casualties-choice-first");
    worker
        .with_live_objects(|session, objects| {
            let step = objects
                .steps_in_list(objects.stack)?
                .last()
                .copied()
                .unwrap();
            let count = session.battle_key("remaining_to_remove")?;
            objects
                .arena
                .set_field(step, count, ObjectValue::Atom(Datum::Int(100)))
                .map_err(unsupported)?;
            let pending = objects.arena.allocate(&Datum::Object(vec![(
                session.selection_key,
                Datum::Text(session.yes),
            )]));
            objects
                .arena
                .set_field(step, objects.pending, pending)
                .map_err(unsupported)
        })
        .unwrap();
    worker.advance_prefix(1).unwrap();
    assert!(worker.last_request.is_none());
    let result = worker.export_envelope().unwrap();
    let alias = &result.state["execution_context"]["battle_step_alias"];
    assert_eq!(alias["remaining_to_remove"], 100);
    assert!(alias["pending_input"].is_object());
    assert_eq!(
        result.state["execution_stack"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["type"],
        "remove_unit"
    );
}
