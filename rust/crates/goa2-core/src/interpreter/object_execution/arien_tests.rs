//! New Arien bodies exercise full live rules, filters and physical effects.
use super::phases::Field;
use super::*;
fn session(name: &str) -> NativeSession {
    let case = fixture_case(&format!("object-execution-arien-primary-{name}"));
    NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
}
#[test]
fn behind_target_length_is_live_and_private_to_a_worker() {
    let root = session("line-direction-0-UNIT-1");
    let before = root.export_envelope().unwrap();
    let mut short = root.clone();
    let mut long = root.clone();
    let step = long.stack.last().unwrap().object.unwrap();
    let objects = long.objects.as_mut().unwrap();
    let Some(ObjectValue::Object(filters)) =
        objects.arena.field(step, objects.phases.k(Field::Filters))
    else {
        panic!()
    };
    let ObjectValue::Object(filter) = objects.arena.sequence(filters).unwrap()[0] else {
        panic!()
    };
    objects
        .arena
        .set_field(
            filter,
            objects.phases.k(Field::BehindLength),
            ObjectValue::Atom(Datum::Int(2)),
        )
        .unwrap();
    for (worker, count) in [(&mut short, 1), (&mut long, 2)] {
        let request = worker.advance_prefix(1).unwrap().request.unwrap();
        assert_eq!(
            worker.request_json(&request)["options"]
                .as_array()
                .unwrap()
                .len(),
            count
        );
    }
    assert_eq!(root.export_envelope().unwrap().state, before.state);
}
#[test]
fn absent_radius_preserves_zero_defaults_without_changing_nullable_filters() {
    let mut worker = session("radius-spell_break-None-text");
    worker.advance_prefix(1).unwrap();
    let state = worker.semantic_state().unwrap();
    assert_eq!(
        state["execution_stack"].as_array().unwrap().last().unwrap()["scope"]["range"],
        json!(0)
    );
    let case = fixture_case("object-execution-primary-text-listen_up-ordinary");
    let mut envelope: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    for definition in &mut envelope.definitions {
        if definition.fields["id"] == "listen_up" {
            definition.fields.insert("radius_value".into(), Value::Null);
        }
    }
    let mut worker = NativeSession::compile(envelope).unwrap();
    worker.advance_prefix(1).unwrap();
    fn has_nullable_range(value: &Value) -> bool {
        match value {
            Value::Object(fields) => {
                (fields.get("type") == Some(&json!("range_filter"))
                    && fields.get("max_range").is_some_and(Value::is_null))
                    || fields.values().any(has_nullable_range)
            }
            Value::Array(values) => values.iter().any(has_nullable_range),
            _ => false,
        }
    }
    assert!(has_nullable_range(
        &worker.semantic_state().unwrap()["execution_stack"]
    ));
}
#[test]
fn new_effect_body_generation_guard_precedes_context_and_stack_mutation() {
    let mut worker = session("spell_break-text-first");
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
#[test]
fn dangerous_current_defeats_the_cardless_hero_behind_its_target() {
    let case = fixture_case("object-execution-arien-primary-dangerous_current-action-first");
    let mut envelope: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    envelope.state["teams"]["BLUE"]["heroes"][0]["hand"] = json!([]);
    let mut worker = NativeSession::compile(envelope).unwrap();
    for _ in 0..100 {
        if worker.stack.is_empty() {
            break;
        }
        if let Some(request) = worker.advance_prefix(1).unwrap().request {
            let value = worker.request_json(&request)["options"][0]["id"].clone();
            let value = worker.import_selection(&value).unwrap();
            worker.submit(request.id, value).unwrap();
        }
    }
    assert!(worker.stack.is_empty());
    let hero = worker.symbols.lookup("hero_brogan").unwrap();
    assert!(worker.world.position(hero).is_none());
}
