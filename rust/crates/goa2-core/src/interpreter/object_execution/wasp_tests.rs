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
fn set(worker: &mut NativeSession, field: Field, value: Datum) {
    let step = worker.stack.last().unwrap().object.unwrap();
    let objects = worker.objects.as_mut().unwrap();
    objects
        .arena
        .set_field(step, objects.phases.k(field), ObjectValue::Atom(value))
        .unwrap();
}
#[test]
fn orbit_body_generation_guard_preserves_context_and_nested_templates() {
    let mut worker = session("wasp-primary-center_of_mass-text-first");
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
fn boolean_query_reads_live_operation_and_keeps_false_as_boolean() {
    let root = session("wasp-query-boolean-OR-1");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let op = worker.symbols.lookup("AND").unwrap();
    set(&mut worker, Field::Operation, Datum::Text(op));
    worker.advance_prefix(1).unwrap();
    assert_eq!(
        worker.semantic_state().unwrap()["execution_context"]["query_result"],
        json!(false)
    );
    let mut ordinary = root.clone();
    ordinary.advance_prefix(1).unwrap();
    assert_eq!(
        ordinary.semantic_state().unwrap()["execution_context"]["query_result"],
        json!(true)
    );
    unchanged(&before, &root.export_envelope().unwrap());
}
#[test]
fn unit_type_reads_live_expected_type_and_resolves_a_piece_owner() {
    let root = session("wasp-query-type-HERO-piece");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let kind = worker.symbols.lookup("MINION").unwrap();
    set(&mut worker, Field::ExpectedType, Datum::Text(kind));
    worker.advance_prefix(1).unwrap();
    assert_eq!(
        worker.semantic_state().unwrap()["execution_context"]["query_result"],
        json!(false)
    );
    let mut ordinary = root.clone();
    ordinary.advance_prefix(1).unwrap();
    assert_eq!(
        ordinary.semantic_state().unwrap()["execution_context"]["query_result"],
        json!(true)
    );
    unchanged(&before, &root.export_envelope().unwrap());
}
#[test]
fn noninteractive_queries_need_no_generation_capacity_and_keep_source_identity_rules() {
    for (name, expected) in [
        ("wasp-query-type-HERO-numeric", true),
        ("wasp-query-type-MINION-unknown", true),
        ("wasp-query-type-HERO-missing", false),
        ("wasp-query-boolean-OR-missing", false),
    ] {
        let mut worker = session(name);
        worker.next_step = u64::MAX;
        worker.next_request = u64::MAX;
        worker.advance_prefix(1).unwrap();
        assert_eq!(
            worker.semantic_state().unwrap()["execution_context"]["query_result"],
            json!(expected)
        );
    }
}
#[test]
fn malformed_live_query_operand_guards_before_popping_or_writing() {
    let mut worker = session("wasp-query-type-HERO-hero");
    set(&mut worker, Field::ExpectedType, Datum::Int(7));
    let before = worker.export_envelope().unwrap();
    assert!(worker.advance_prefix(1).is_err());
    unchanged(&before, &worker.export_envelope().unwrap());
}
