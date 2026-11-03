//! Live operand, allocation-identity and atomic-boundary utility regressions.
use super::phases::Field;
use super::*;
fn session(name: &str) -> NativeSession {
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let index: Value = serde_json::from_reader(std::io::BufReader::new(
        crate::fixture_cache::open(base.join("object-execution.json")).unwrap(),
    ))
    .unwrap();
    let name = format!("object-execution-utility-{name}");
    let entry = index["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .unwrap();
    let case: Value = serde_json::from_reader(std::io::BufReader::new(
        crate::fixture_cache::open(base.join(entry["path"].as_str().unwrap())).unwrap(),
    ))
    .unwrap();
    NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
}
#[test]
fn coordinate_results_keep_dict_and_shared_frozen_model_roles_distinct() {
    for (name, kind, shared) in [
        ("record-plain", ObjectKind::Dict, false),
        ("compute-plain", ObjectKind::FrozenModel, true),
    ] {
        let mut worker = session(name);
        worker.advance_prefix(1).unwrap();
        let objects = worker.objects.as_ref().unwrap();
        let Some(ObjectValue::Object(output)) = objects
            .arena
            .context_value(worker.symbols.lookup("output").unwrap())
        else {
            panic!("coordinate result missing");
        };
        assert_eq!(objects.arena.kind(output), kind);
        assert_eq!(objects.arena.is_shared_frozen_model(output), shared);
    }
}
#[test]
fn computed_scale_is_live_and_zero_still_allocates_a_fresh_hex_in_one_worker() {
    let root = session("compute-plain");
    let before = root.export_envelope().unwrap();
    let mut changed = root.clone();
    let step = changed.stack.last().unwrap().object.unwrap();
    let objects = changed.objects.as_mut().unwrap();
    objects
        .arena
        .set_field(
            step,
            objects.phases.k(Field::Scale),
            ObjectValue::Atom(Datum::Int(0)),
        )
        .unwrap();
    changed.advance_prefix(1).unwrap();
    let objects = changed.objects.as_ref().unwrap();
    let output = objects
        .arena
        .context_value(changed.symbols.lookup("output").unwrap())
        .unwrap();
    let position = objects
        .arena
        .context_value(changed.symbols.lookup("position_alias").unwrap())
        .unwrap();
    let (ObjectValue::Object(output_id), ObjectValue::Object(position_id)) = (&output, &position)
    else {
        panic!("Hex handles missing");
    };
    assert_ne!(output_id, position_id);
    assert_eq!(
        changed.spatial_hex_bound(objects, &output).unwrap().0,
        changed.spatial_hex_bound(objects, &position).unwrap().0
    );
    assert_eq!(root.export_envelope().unwrap().state, before.state);
}
#[test]
fn invalid_unit_direction_is_guarded_even_at_zero_scale_before_consumption() {
    let mut worker = session("compute-plain");
    let target = worker.symbols.lookup("hero_brogan").unwrap();
    let step = worker.stack.last().unwrap().object.unwrap();
    let objects = worker.objects.as_mut().unwrap();
    let hex = objects
        .arena
        .shared_frozen_model(
            worker
                .coordinates
                .into_iter()
                .zip([2, -1, -1])
                .map(|(key, value)| (key, ObjectValue::Atom(Datum::Int(value))))
                .collect(),
        )
        .unwrap();
    let locations = objects
        .phases
        .root(&objects.arena, Field::Locations)
        .unwrap();
    objects.arena.set_field(locations, target, hex).unwrap();
    objects
        .arena
        .set_field(
            step,
            objects.phases.k(Field::Scale),
            ObjectValue::Atom(Datum::Int(0)),
        )
        .unwrap();
    let before = worker.export_envelope().unwrap();
    assert!(
        worker
            .advance_prefix(1)
            .unwrap_err()
            .to_string()
            .contains("computed direction violates source Hex validation")
    );
    let after = worker.export_envelope().unwrap();
    assert_eq!(before.state, after.state);
    assert_eq!(before.object_bindings, after.object_bindings);
}
#[test]
fn repeat_request_counter_guard_leaves_pending_state_and_stack_untouched() {
    let mut worker = session("repeat-yes");
    worker.next_request = u64::MAX;
    let before = worker.export_envelope().unwrap();
    assert!(
        worker
            .advance_prefix(1)
            .unwrap_err()
            .to_string()
            .contains("request generation counter exhausted")
    );
    let after = worker.export_envelope().unwrap();
    assert_eq!(before.state, after.state);
    assert_eq!(before.object_bindings, after.object_bindings);
    assert_eq!(worker.next_request, u64::MAX);
}
#[test]
fn repeat_copies_keep_source_step_ids_without_consuming_generation_capacity() {
    let mut worker = session("repeat-yes");
    worker.next_step = u64::MAX;
    let parent = worker.stack.last().unwrap().object.unwrap();
    let originals = worker
        .objects
        .as_ref()
        .unwrap()
        .template_objects(parent)
        .unwrap();
    let request = worker.advance_prefix(1).unwrap().request.unwrap();
    worker.submit(request.id, Datum::Text(worker.yes)).unwrap();
    worker.advance_prefix(1).unwrap();
    assert_eq!(worker.next_step, u64::MAX);
    let objects = worker.objects.as_ref().unwrap();
    for (frame, original) in worker.stack.iter().rev().zip(originals) {
        let copy = frame.object.unwrap();
        assert_ne!(copy, original);
        let field = objects.scheduler.as_ref().unwrap().step_id;
        match (
            objects.arena.field(copy, field),
            objects.arena.field(original, field),
        ) {
            (Some(ObjectValue::Atom(a)), Some(ObjectValue::Atom(b))) => assert_eq!(a, b),
            (Some(ObjectValue::GeneratedStep(a)), Some(ObjectValue::GeneratedStep(b))) => {
                assert_eq!(a, b)
            }
            _ => panic!("copied step identity representation changed"),
        }
    }
    worker.advance_prefix(20).unwrap();
    assert_eq!(
        worker.export_envelope().unwrap().state["execution_context"]["counter"],
        1
    );
}
