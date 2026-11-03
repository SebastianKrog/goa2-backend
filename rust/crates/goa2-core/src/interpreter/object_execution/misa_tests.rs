use super::phases::Field;
use super::*;

fn session(name: &str) -> NativeSession {
    let case = fixture_case(&format!("object-execution-{name}"));
    NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
}
fn unchanged(before: &StateEnvelope, after: &StateEnvelope) {
    fn difference(a: &serde_json::Value, b: &serde_json::Value, path: &str) {
        if a == b {
            return;
        }
        match (a, b) {
            (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
                assert_eq!(a.len(), b.len(), "field count changed at {path}");
                for (key, value) in a {
                    let other = b
                        .get(key)
                        .unwrap_or_else(|| panic!("field removed at {path}/{key}"));
                    difference(value, other, &format!("{path}/{key}"));
                }
            }
            (serde_json::Value::Array(a), serde_json::Value::Array(b)) => {
                assert_eq!(a.len(), b.len(), "array length changed at {path}");
                for (i, (a, b)) in a.iter().zip(b).enumerate() {
                    difference(a, b, &format!("{path}/{i}"));
                }
            }
            _ => panic!("state changed at {path}: {a} -> {b}"),
        }
    }
    difference(&before.state, &after.state, "state");
    assert_eq!(before.object_bindings, after.object_bindings);
}
fn level(worker: &mut NativeSession, name: &str, value: i64) {
    let id = worker.symbols.lookup(name).unwrap();
    let model = worker
        .hero_model_bound(worker.objects.as_ref().unwrap(), id)
        .unwrap();
    let objects = worker.objects.as_mut().unwrap();
    objects
        .arena
        .set_field(
            model,
            objects.phases.k(Field::Level),
            ObjectValue::Atom(Datum::Int(value)),
        )
        .unwrap();
}
#[test]
fn misa_body_generation_guard_keeps_context_and_stack_unchanged() {
    let mut worker = session("misa-primary-challenge_accepted-text-first");
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
fn ultimate_shape_reads_live_level_and_keeps_the_root_private() {
    let root = session("misa-primary-swoop_in-text-first");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    level(&mut worker, "hero_misa", 8);
    worker.advance_prefix(1).unwrap();
    assert_eq!(
        worker.semantic_state().unwrap()["execution_stack"]
            .as_array()
            .unwrap()
            .len(),
        7
    );
    unchanged(&before, &root.export_envelope().unwrap());
}
#[test]
fn missing_live_ultimate_keeps_the_base_shape_even_at_level_eight() {
    let mut worker = session("misa-primary-swoop_in-text-first");
    level(&mut worker, "hero_misa", 8);
    let id = worker.symbols.lookup("hero_misa").unwrap();
    let model = worker
        .hero_model_bound(worker.objects.as_ref().unwrap(), id)
        .unwrap();
    let objects = worker.objects.as_mut().unwrap();
    objects
        .arena
        .set_field(
            model,
            objects.phases.k(Field::Ultimate),
            ObjectValue::Atom(Datum::Null),
        )
        .unwrap();
    worker.advance_prefix(1).unwrap();
    assert_eq!(
        worker.semantic_state().unwrap()["execution_stack"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
}
#[test]
fn copied_shape_uses_the_actual_performers_live_ultimate_and_level() {
    for (name, owner_level, expected) in [
        ("misa-primary-swoop_in-copy-first", 8, 0),
        ("misa-primary-swoop_in-foreign-ultimate", 1, 1),
    ] {
        let mut worker = session(name);
        level(&mut worker, "hero_misa", owner_level);
        let request = worker.advance_prefix(1).unwrap().request.unwrap();
        worker
            .submit(
                request.id,
                Datum::Text(worker.symbols.lookup("SKILL").unwrap()),
            )
            .unwrap();
        worker.advance_prefix(1).unwrap();
        let state = worker.semantic_state().unwrap();
        assert_eq!(
            state["execution_stack"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|step| step["type"] == "force_discard")
                .count(),
            expected
        );
    }
}
#[test]
fn swoop_request_generation_guard_precedes_selection_and_placement() {
    let mut worker = session("misa-primary-swoop_in-text-first");
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
    unchanged(&before, &worker.export_envelope().unwrap());

    let mut resumed = session("misa-primary-swoop_in-text-first");
    resumed.advance_prefix(1).unwrap();
    let request = resumed.advance_prefix(1).unwrap().request.unwrap();
    resumed.next_request = u64::MAX;
    let before = resumed.export_envelope().unwrap();
    let repeated = resumed.advance_prefix(1).unwrap().request.unwrap();
    assert_eq!(request.id, repeated.id);
    unchanged(&before, &resumed.export_envelope().unwrap());
}
#[test]
fn distance_query_reads_live_threshold_without_defaulting_missing_units_to_actor() {
    let root = session("misa-geometry-distance-False-==-4");
    let before = root.export_envelope().unwrap();
    for missing in [false, true] {
        let mut worker = root.clone();
        let step = worker.stack.last().unwrap().object.unwrap();
        let objects = worker.objects.as_mut().unwrap();
        let field = if missing {
            Field::UnitAId
        } else {
            Field::Threshold
        };
        let value = if missing { Datum::Null } else { Datum::Int(3) };
        objects
            .arena
            .set_field(step, objects.phases.k(field), ObjectValue::Atom(value))
            .unwrap();
        worker.advance_prefix(1).unwrap();
        assert!(worker.semantic_state().unwrap()["execution_context"]["distance_result"].is_null());
    }
    let mut worker = root.clone();
    worker.advance_prefix(1).unwrap();
    assert_eq!(
        worker.semantic_state().unwrap()["execution_context"]["distance_result"],
        json!(true)
    );
    unchanged(&before, &root.export_envelope().unwrap());
}
#[test]
fn relative_comparison_reads_live_operator_and_preserves_infinite_equality() {
    let root = session("misa-geometry-relative-UNIT-both-infinite-==");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let alias = worker.symbols.lookup("relative_alias").unwrap();
    let op = worker.symbols.lookup(">").unwrap();
    let objects = worker.objects.as_mut().unwrap();
    let Some(ObjectValue::Object(filter)) = objects.arena.context_value(alias) else {
        panic!("filter alias missing")
    };
    objects
        .arena
        .set_field(
            filter,
            objects.phases.k(Field::Operator),
            ObjectValue::Atom(Datum::Text(op)),
        )
        .unwrap();
    worker.advance_prefix(1).unwrap();
    assert!(worker.semantic_state().unwrap()["execution_context"]["relative_result"].is_null());
    let mut equal = root.clone();
    equal.advance_prefix(1).unwrap();
    assert_eq!(
        equal.semantic_state().unwrap()["execution_context"]["relative_result"],
        json!(true)
    );
    unchanged(&before, &root.export_envelope().unwrap());
}
#[test]
fn between_reads_mutated_plain_hex_dict_and_rejects_invalid_cube_sum() {
    let root = session("misa-geometry-between-UNIT-dicts");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let alias = worker.symbols.lookup("between_from").unwrap();
    let objects = worker.objects.as_mut().unwrap();
    let Some(ObjectValue::Object(point)) = objects.arena.context_value(alias) else {
        panic!("hex alias missing")
    };
    objects
        .arena
        .set_field(
            point,
            worker.coordinates[0],
            ObjectValue::Atom(Datum::Int(0)),
        )
        .unwrap();
    worker.advance_prefix(1).unwrap();
    assert!(worker.semantic_state().unwrap()["execution_context"]["between_result"].is_null());
    unchanged(&before, &root.export_envelope().unwrap());
}
#[test]
fn hex_adjacency_reads_the_live_target_key_and_keeps_filter_alias_private() {
    let root = session("misa-geometry-adjacency-HEX-ordinary");
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let alias = worker.symbols.lookup("adjacent_alias").unwrap();
    let missing = worker.symbols.lookup("tail").unwrap();
    let objects = worker.objects.as_mut().unwrap();
    let Some(ObjectValue::Object(filter)) = objects.arena.context_value(alias) else {
        panic!("filter alias missing")
    };
    objects
        .arena
        .set_field(
            filter,
            objects.phases.k(Field::TargetKey),
            ObjectValue::Atom(Datum::Text(missing)),
        )
        .unwrap();
    worker.advance_prefix(1).unwrap();
    assert_eq!(
        worker.semantic_state().unwrap()["execution_context"]["adjacent_result"],
        json!(0)
    );
    let mut unmodified = root.clone();
    unmodified.advance_prefix(1).unwrap();
    assert_eq!(
        unmodified.semantic_state().unwrap()["execution_context"]["adjacent_result"],
        json!(6)
    );
    unchanged(&before, &root.export_envelope().unwrap());
}
