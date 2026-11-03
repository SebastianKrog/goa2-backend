use super::phases::Field;
use super::*;
fn session(name: &str) -> NativeSession {
    let case = fixture_case(&format!("object-execution-{name}"));
    NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
}
#[test]
fn previous_slot_filter_reads_live_owner_slots_without_mutating_the_root() {
    let root = session("mind-grip-filter-2-UNIT");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let owner = worker.symbols.lookup("hero_tigerclaw").unwrap();
    let model = worker
        .hero_model_bound(worker.objects.as_ref().unwrap(), owner)
        .unwrap();
    let objects = worker.objects.as_mut().unwrap();
    let empty = objects.arena.list(Vec::new());
    objects
        .arena
        .set_field(model, objects.phases.k(Field::Played), empty)
        .unwrap();
    assert!(worker.advance_prefix(1).unwrap().request.is_none());
    assert_eq!(root.export_envelope().unwrap().state, before.state);
    assert_eq!(
        root.export_envelope().unwrap().object_bindings,
        before.object_bindings
    );
}
#[test]
fn mind_grip_generation_guard_preserves_stack_and_current_card_context() {
    let mut worker = session("primary-text-mind_grip-ordinary");
    worker.next_step = u64::MAX;
    let before = worker.export_envelope().unwrap();
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
}
