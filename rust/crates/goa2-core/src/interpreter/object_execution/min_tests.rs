use super::phases::Field;
use super::*;

fn session(name: &str) -> NativeSession {
    let case = fixture_case(&format!("object-execution-{name}"));
    NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
}
fn unchanged(before: &StateEnvelope, after: &StateEnvelope) {
    assert_eq!(before.state, after.state);
    assert_eq!(before.object_bindings, after.object_bindings);
}
fn clear_collection(worker: &mut NativeSession, field: Field) {
    let hero = worker.symbols.lookup("hero_min").unwrap();
    let model = worker
        .hero_model_bound(worker.objects.as_ref().unwrap(), hero)
        .unwrap();
    let objects = worker.objects.as_mut().unwrap();
    let Some(ObjectValue::Object(list)) = objects.arena.field(model, objects.phases.k(field))
    else {
        panic!("collection missing");
    };
    objects.arena.clear_list(list).unwrap();
}
#[test]
fn perfect_self_generation_guard_keeps_retired_cards_context_and_stack_private() {
    let mut worker = session("min-primary-perfect_self-option-both");
    worker.next_step = u64::MAX;
    let before = worker.export_envelope().unwrap();
    assert!(
        worker
            .advance_prefix(1)
            .unwrap_err()
            .to_string()
            .contains("step generation counter exhausted")
    );
    unchanged(&before, &worker.export_envelope().unwrap());
}
#[test]
fn deferred_attack_only_body_needs_no_generation_capacity_for_discarded_prefix() {
    let mut worker = session("min-after-attack-only");
    worker.next_step = u64::MAX;
    worker.next_request = u64::MAX;
    worker.advance_prefix(1).unwrap();
    assert_eq!(worker.next_step, u64::MAX);
    assert_eq!(worker.stack.len(), 1);
}
#[test]
fn deferred_suffix_generation_guard_precedes_target_channel_propagation() {
    let mut worker = session("min-after-custom-target");
    worker.next_step = u64::MAX;
    let before = worker.export_envelope().unwrap();
    assert!(
        worker
            .advance_prefix(1)
            .unwrap_err()
            .to_string()
            .contains("step generation counter exhausted")
    );
    unchanged(&before, &worker.export_envelope().unwrap());
}
#[test]
fn conversion_updates_every_shared_item_cache_without_mutating_the_root() {
    let root = session("min-item-shared-items");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    worker.next_step = u64::MAX;
    worker.next_request = u64::MAX;
    worker.advance_prefix(1).unwrap();
    for name in ["hero_min", "hero_wasp"] {
        let id = worker.symbols.lookup(name).unwrap();
        assert_eq!(
            worker.world.hero(id).unwrap().items[Stat::Defense.index()],
            1
        );
    }
    unchanged(&before, &root.export_envelope().unwrap());
}
#[test]
fn conversion_reads_live_hero_operand_instead_of_using_cached_owner() {
    let root = session("min-item-ordinary");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let actor = worker.symbols.lookup("hero_wasp").unwrap();
    let step = worker.stack.last().unwrap().object.unwrap();
    let objects = worker.objects.as_mut().unwrap();
    objects
        .arena
        .set_field(
            step,
            objects.phases.k(Field::HeroId),
            ObjectValue::Atom(Datum::Text(actor)),
        )
        .unwrap();
    assert!(worker.advance_prefix(1).unwrap().events.is_empty());
    assert_eq!(worker.world.hero(actor).unwrap().items, [0; 6]);
    unchanged(&before, &root.export_envelope().unwrap());
}
#[test]
fn perfect_self_reads_live_deck_membership_and_allocates_only_the_selected_variant() {
    let root = session("min-primary-perfect_self-option-both");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    clear_collection(&mut worker, Field::Deck);
    worker.advance_prefix(1).unwrap();
    let state = worker.semantic_state().unwrap();
    let stack = state["execution_stack"].as_array().unwrap();
    assert_eq!(stack.len(), 2);
    assert_eq!(stack[1]["type"], "create_effect");
    unchanged(&before, &root.export_envelope().unwrap());
}
#[test]
fn deferred_lookup_reads_live_discard_membership_at_the_execution_boundary() {
    let root = session("min-after-ordinary");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    clear_collection(&mut worker, Field::Discard);
    worker.next_step = u64::MAX;
    worker.advance_prefix(1).unwrap();
    assert_eq!(worker.stack.len(), 1);
    unchanged(&before, &root.export_envelope().unwrap());
}
