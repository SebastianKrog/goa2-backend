use super::phases::Field;
use super::*;

fn session(name: &str) -> NativeSession {
    let case = fixture_case(&format!("object-execution-{name}"));
    NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
}
fn assert_unchanged(before: &StateEnvelope, after: &StateEnvelope) {
    assert_eq!(before.state, after.state);
    assert_eq!(before.object_bindings, after.object_bindings);
}
#[test]
fn spell_body_generation_limit_keeps_pending_choice_and_scope_unchanged() {
    let mut worker = session("spell-body-dimension_door-copy-first");
    let request = worker.advance_prefix(1).unwrap().request.unwrap();
    worker
        .submit(
            request.id,
            Datum::Text(worker.symbols.lookup("SKILL").unwrap()),
        )
        .unwrap();
    worker.next_step = u64::MAX;
    let before = worker.export_envelope().unwrap();
    assert!(
        worker
            .advance_prefix(1)
            .unwrap_err()
            .to_string()
            .contains("step generation counter exhausted")
    );
    assert_unchanged(&before, &worker.export_envelope().unwrap());
}
#[test]
fn midas_uses_live_other_spell_states_and_keeps_root_physical_cards_private() {
    let root = session("spell-body-midas_touch-copy-first");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let request = worker.advance_prefix(1).unwrap().request.unwrap();
    worker
        .submit(
            request.id,
            Datum::Text(worker.symbols.lookup("SKILL").unwrap()),
        )
        .unwrap();
    for name in ["magic_missile", "shocking_grasp"] {
        let printed = worker.symbols.lookup(name).unwrap();
        let card = worker.world.card_by_printed(printed).unwrap();
        let physical = worker.world.card(card).unwrap().source_instance;
        let objects = worker.objects.as_mut().unwrap();
        objects
            .arena
            .set_physical_field(
                physical,
                objects.phases.k(Field::CardState),
                Datum::Text(objects.phases.k(Field::OutsideSpellbookState)),
            )
            .unwrap();
    }
    let result = worker.advance_prefix(100).unwrap();
    assert!(result.request.is_none());
    let owner = worker.symbols.lookup("hero_gydion").unwrap();
    assert_eq!(worker.world.hero(owner).unwrap().gold, 1);
    assert_unchanged(&before, &root.export_envelope().unwrap());
}
#[test]
fn container_predicate_reads_live_visibility_flag_and_played_holes() {
    let root = session("spell-primitive-cards-PLAYED-True-hero_gydion");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let alias = worker.symbols.lookup("predicate_alias").unwrap();
    let objects = worker.objects.as_mut().unwrap();
    let Some(ObjectValue::Object(predicate)) = objects.arena.context_value(alias) else {
        panic!("predicate missing")
    };
    objects
        .arena
        .set_field(
            predicate,
            objects.phases.k(Field::ExcludeFacedown),
            ObjectValue::Atom(Datum::Bool(false)),
        )
        .unwrap();
    worker.advance_prefix(1).unwrap();
    assert!(worker.semantic_state().unwrap()["execution_context"]["predicate_result"].is_null());
    assert_unchanged(&before, &root.export_envelope().unwrap());
}
#[test]
fn candidate_count_reads_live_bounds_and_preserves_existing_origin_context() {
    let mut worker = session("spell-primitive-count-UNIT-neighbor-False");
    let alias = worker.symbols.lookup("predicate_alias").unwrap();
    let objects = worker.objects.as_mut().unwrap();
    let Some(ObjectValue::Object(predicate)) = objects.arena.context_value(alias) else {
        panic!("predicate missing")
    };
    objects
        .arena
        .set_field(
            predicate,
            objects.phases.k(Field::MaxCount),
            ObjectValue::Atom(Datum::Int(0)),
        )
        .unwrap();
    worker.advance_prefix(1).unwrap();
    let state = worker.semantic_state().unwrap();
    assert!(state["execution_context"]["predicate_result"].is_null());
    assert_eq!(
        state["execution_context"]["_cmf_origin_hex"],
        json!({"retained":true})
    );
}
#[test]
fn minion_respawn_request_limit_keeps_stack_supply_and_positions_unchanged() {
    let mut worker = session("spell-primitive-respawn-ordinary");
    worker.next_request = u64::MAX;
    let before = worker.export_envelope().unwrap();
    assert!(
        worker
            .advance_prefix(1)
            .unwrap_err()
            .to_string()
            .contains("request generation counter exhausted")
    );
    assert_unchanged(&before, &worker.export_envelope().unwrap());
}
#[test]
fn respawn_placement_uses_shared_location_container_without_mutating_root() {
    let root = session("spell-primitive-respawn-one-type");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let request = worker.advance_prefix(1).unwrap().request.unwrap();
    let RequestChoices::Hexes(ref choices) = request.choices else {
        panic!("hex prompt missing")
    };
    let hex = choices[0];
    worker
        .submit(request.id, native_hex(hex, worker.coordinates))
        .unwrap();
    let result = worker.advance_prefix(1).unwrap();
    let Some(NativeEvent::UnitPlaced {
        unit,
        from: None,
        to,
    }) = result.events.first()
    else {
        panic!("placement event missing")
    };
    assert_eq!(*to, hex);
    assert_eq!(worker.world.position(*unit), Some(hex));
    let objects = worker.objects.as_ref().unwrap();
    let alias = worker.symbols.lookup("locations_alias").unwrap();
    let root_key = worker.symbols.lookup("entity_locations").unwrap();
    let Some(ObjectValue::Object(alias_id)) = objects.arena.context_value(alias) else {
        panic!("location alias missing")
    };
    let Some(ObjectValue::Object(root_id)) = objects.arena.root_field(root_key) else {
        panic!("root locations missing")
    };
    assert_eq!(alias_id, root_id);
    assert_unchanged(&before, &root.export_envelope().unwrap());
}
