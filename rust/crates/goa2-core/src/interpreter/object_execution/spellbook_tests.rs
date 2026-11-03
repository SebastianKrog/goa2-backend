use super::phases::Field;
use super::*;

fn session(name: &str) -> NativeSession {
    let case = fixture_case(&format!("object-execution-spellbook-{name}"));
    NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
}
fn unchanged(worker: &mut NativeSession, text: &str) {
    let before = worker.export_envelope().unwrap();
    assert!(
        worker
            .advance_prefix(1)
            .unwrap_err()
            .to_string()
            .contains(text)
    );
    let after = worker.export_envelope().unwrap();
    assert_eq!(before.state, after.state);
    assert_eq!(before.object_bindings, after.object_bindings);
}
#[test]
fn exhausted_spell_request_generation_does_not_pop_or_spend() {
    let mut worker = session("cast-ordinary");
    worker.next_request = u64::MAX;
    unchanged(&mut worker, "request generation counter exhausted");
}
#[test]
fn exhausted_cast_child_generation_keeps_the_physical_spell_prepared() {
    let mut worker = session("cast-ordinary");
    let spell = worker.symbols.lookup("magic_missile").unwrap();
    let objects = worker.objects.as_mut().unwrap();
    let step = objects.step(1).unwrap();
    let selected = objects
        .arena
        .dictionary(vec![(
            worker.selection_key,
            ObjectValue::Atom(Datum::Text(spell)),
        )])
        .unwrap();
    objects
        .arena
        .set_field(step, objects.pending, selected)
        .unwrap();
    worker.next_step = u64::MAX;
    unchanged(&mut worker, "step generation counter exhausted");
}
#[test]
fn wish_overflow_keeps_started_count_and_continuation_unchanged() {
    let mut worker = session("wish-ordinary");
    let hero = worker.symbols.lookup("hero_gydion").unwrap();
    let model = worker
        .hero_model_bound(worker.objects.as_ref().unwrap(), hero)
        .unwrap();
    let objects = worker.objects.as_mut().unwrap();
    objects
        .arena
        .set_field(
            model,
            objects.phases.k(Field::WishCount),
            ObjectValue::Atom(Datum::Int(i64::MAX)),
        )
        .unwrap();
    unchanged(&mut worker, "Wish count outside i64");
}
#[test]
fn wish_worker_mutations_keep_root_counter_steps_and_aliases_private() {
    let root = session("wish-ordinary");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    worker.advance_prefix(1).unwrap();
    assert_eq!(
        worker.semantic_state().unwrap()["execution_context"]["owner_alias"]["wish_cast_count"],
        1
    );
    let after = root.export_envelope().unwrap();
    assert_eq!(before.state, after.state);
    assert_eq!(before.object_bindings, after.object_bindings);
}
#[test]
fn caster_selection_reads_live_step_fields_and_does_not_change_owner() {
    let mut worker = session("cast-ordinary");
    let caster = worker.symbols.lookup("hero_hanu").unwrap();
    let objects = worker.objects.as_mut().unwrap();
    let step = objects.step(1).unwrap();
    objects
        .arena
        .set_field(
            step,
            objects.phases.k(Field::CasterId),
            ObjectValue::Atom(Datum::Text(caster)),
        )
        .unwrap();
    let request = worker.advance_prefix(1).unwrap().request.unwrap();
    assert_eq!(request.owner, caster);
    let wire = worker.request_json(&request);
    assert_eq!(wire["decision_context"]["actor_id"], "hero_hanu");
    assert_eq!(wire["options"][0]["text"], "Magic Missile");
}
#[test]
fn multiple_live_spellbook_owners_stay_guarded_before_pop() {
    let mut worker = session("cast-ordinary");
    let owner = worker.symbols.lookup("hero_gydion").unwrap();
    let other = worker.symbols.lookup("hero_hanu").unwrap();
    let objects = worker.objects.as_ref().unwrap();
    let owner = worker.hero_model_bound(objects, owner).unwrap();
    let other = worker.hero_model_bound(objects, other).unwrap();
    let objects = worker.objects.as_mut().unwrap();
    let field = objects.phases.k(Field::Spells);
    let values = objects.arena.field(owner, field).unwrap();
    objects.arena.set_field(other, field, values).unwrap();
    unchanged(&mut worker, "multiple spellbook owners");
}
