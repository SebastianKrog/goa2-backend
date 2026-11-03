use super::phases::Field;
use super::*;

fn session(name: &str) -> NativeSession {
    let case = fixture_case(&format!("object-execution-{name}"));
    NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
}

#[test]
fn card_count_reads_live_container_without_changing_root_aliases() {
    let root = session("brogan-query-count-HAND-actor");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let key = worker.symbols.lookup("hand_alias").unwrap();
    let objects = worker.objects.as_mut().unwrap();
    let Some(ObjectValue::Object(hand)) = objects.arena.context_value(key) else {
        panic!("hand alias missing")
    };
    objects.arena.clear_list(hand).unwrap();
    worker.advance_prefix(1).unwrap();
    assert_eq!(
        worker.semantic_state().unwrap()["execution_context"]["query_result"],
        0
    );
    let after = root.export_envelope().unwrap();
    assert_eq!(before.state, after.state);
    assert_eq!(before.object_bindings, after.object_bindings);
}

#[test]
fn round_defeat_query_uses_live_root_collection_and_preserves_none() {
    let root = session("brogan-query-defeat-nonempty");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let objects = worker.objects.as_mut().unwrap();
    let Some(ObjectValue::Object(marks)) = objects
        .arena
        .root_field(objects.phases.k(Field::DefeatedHeroes))
    else {
        panic!("defeat marks missing")
    };
    objects.arena.clear_list(marks).unwrap();
    worker.advance_prefix(1).unwrap();
    assert!(worker.semantic_state().unwrap()["execution_context"]["query_result"].is_null());
    let after = root.export_envelope().unwrap();
    assert_eq!(before.state, after.state);
    assert_eq!(before.object_bindings, after.object_bindings);
}
