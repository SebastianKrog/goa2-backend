use super::phases::Field;
use super::*;
use crate::world::CardLifecycle;

fn session(name: &str) -> NativeSession {
    let case = fixture_case(&format!("object-execution-gydion-primary-{name}"));
    NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
}
fn assert_unchanged(before: &StateEnvelope, after: &StateEnvelope) {
    assert_eq!(before.state, after.state);
    assert_eq!(before.object_bindings, after.object_bindings);
}
#[test]
fn access_body_generation_limit_does_not_change_card_scope_or_stack() {
    let mut worker = session("cantrip-text-first");
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
fn cloned_school_cast_spends_only_the_workers_physical_spell() {
    let root = session("cantrip-text-first");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    worker.advance_prefix(1).unwrap();
    let request = worker.advance_prefix(1).unwrap().request.unwrap();
    let RequestChoices::SpellCards { values, .. } = &request.choices else {
        panic!("spell menu missing")
    };
    assert_eq!(values.len(), 3);
    let card = values[0];
    let printed =
        worker.world.definitions.cards[worker.world.card(card).unwrap().definition].printed_id;
    worker.submit(request.id, Datum::Text(printed)).unwrap();
    worker.advance_prefix(1).unwrap();
    assert_eq!(
        worker.world.card(card).unwrap().lifecycle,
        CardLifecycle::OutsideSpellbook
    );
    assert_unchanged(&before, &root.export_envelope().unwrap());
}
#[test]
fn archwizard_discovery_reads_the_live_physical_wish_state() {
    let mut worker = session("prepare-text-archwizard-yes");
    worker.advance_prefix(1).unwrap();
    let printed = worker.symbols.lookup("wish").unwrap();
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
    worker.advance_prefix(1).unwrap();
    let state = worker.semantic_state().unwrap();
    assert!(
        !state["execution_stack"]
            .as_array()
            .unwrap()
            .iter()
            .any(|step| step["type"] == "offer_passive")
    );
    worker.advance_prefix(1).unwrap();
    let state = worker.semantic_state().unwrap();
    let spells = state["execution_context"]["owner_alias"]["spells"]
        .as_array()
        .unwrap();
    assert_eq!(
        spells.iter().find(|spell| spell["id"] == "wish").unwrap()["state"],
        json!("SPELLBOOK")
    );
    assert_eq!(
        state["execution_context"]["owner_alias"]["wish_cast_count"],
        json!(0)
    );
}
#[test]
fn archwizard_request_generation_limit_keeps_the_offer_and_passive_uses() {
    let mut worker = session("prepare-text-archwizard-yes");
    worker.advance_prefix(1).unwrap();
    worker.advance_prefix(1).unwrap();
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
fn preparation_worker_keeps_the_roots_book_and_physical_states_private() {
    let root = session("prepare_spells-text-first");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    worker.advance_prefix(1).unwrap();
    worker.advance_prefix(1).unwrap();
    worker.advance_prefix(1).unwrap();
    let hero = worker
        .world
        .hero(worker.symbols.lookup("hero_gydion").unwrap())
        .unwrap();
    assert_eq!(hero.spells.len(), 22);
    assert!(
        hero.spells
            .iter()
            .all(|card| worker.world.card(*card).unwrap().lifecycle == CardLifecycle::Spellbook)
    );
    assert_unchanged(&before, &root.export_envelope().unwrap());
}

#[test]
fn existing_archwizard_request_is_reused_when_generation_is_exhausted() {
    let mut worker = session("prepare-text-archwizard-yes");
    worker.advance_prefix(1).unwrap();
    worker.advance_prefix(1).unwrap();
    let request = worker.advance_prefix(1).unwrap().request.unwrap();
    worker.next_request = u64::MAX;
    let before = worker.export_envelope().unwrap();
    let repeated = worker.advance_prefix(1).unwrap().request.unwrap();
    assert_eq!(request.id, repeated.id);
    assert_unchanged(&before, &worker.export_envelope().unwrap());
}
