//! Differential action resolution records every generated child before execution.
use goa2_core::{
    codec::StateEnvelope,
    data::Symbols,
    diagnostics::{action_trace, travel_zones},
    world::NativeWorld,
};
use serde_json::{Value, json};
use std::collections::HashMap;
fn corpus() -> Value {
    serde_json::from_str(include_str!("../../../tests/fixtures/actions.json")).unwrap()
}
fn normalize_step(step: &mut Value, ids: &mut HashMap<String, String>) {
    if let Some(raw) = step["step_id"]
        .as_str()
        .filter(|s| s.starts_with("native-step-"))
        .map(str::to_owned)
    {
        let next = format!("dynamic-step/{}", ids.len() + 1);
        step["step_id"] = json!(ids.entry(raw).or_insert(next));
    }
    if let Some(children) = step.get_mut("steps_template").and_then(Value::as_array_mut) {
        for child in children {
            normalize_step(child, ids);
        }
    }
}
#[test]
fn every_registered_hero_menu_and_action_recipe_matches_python() {
    for case in corpus()["cases"].as_array().unwrap() {
        let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let mut result =
            action_trace(envelope, case["operations"].as_array().unwrap().clone()).unwrap();
        let records = result["records"].as_array_mut().unwrap();
        assert_eq!(records.len(), case["expected"].as_array().unwrap().len());
        for (i, (record, want)) in records
            .iter_mut()
            .zip(case["expected"].as_array().unwrap())
            .enumerate()
        {
            if !record["request"].is_null() {
                record["request"]["id"] = json!("request/1");
            }
            let mut ids = HashMap::new();
            for step in record["steps"].as_array_mut().unwrap() {
                normalize_step(step, &mut ids);
            }
            assert!(record == want, "{} operation {i} differs", case["name"]);
        }
    }
}
#[test]
fn all_map_zone_candidates_and_non_enemy_entities_match_python() {
    for case in corpus()["travel"].as_array().unwrap() {
        let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let result = travel_zones(envelope, case["queries"].as_array().unwrap().clone()).unwrap();
        assert_eq!(result["results"], case["expected"], "{}", case["name"]);
    }
}
#[test]
fn fast_travel_uses_each_workers_current_locations() {
    let cases = corpus();
    let case = cases["travel"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "generated-enemy")
        .unwrap();
    let envelope: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    let mut symbols = Symbols::default();
    let root = NativeWorld::compile(&envelope, &mut symbols).unwrap();
    let red = symbols.intern("RED");
    let start = symbols.intern("start");
    let blue = symbols.intern("hero_blue");
    let mut worker = root.clone();
    let original = root.safe_travel_zones(red, start);
    assert!(original.is_empty());
    worker
        .move_entity(blue, goa2_core::geometry::Hex::new(4, -4, 0).unwrap())
        .unwrap();
    assert!(!worker.safe_travel_zones(red, start).is_empty());
    assert_eq!(root.safe_travel_zones(red, start), original);
    assert!(std::sync::Arc::ptr_eq(
        &root.definitions,
        &worker.definitions
    ));
}
#[test]
fn unported_action_effect_closure_never_becomes_an_unrestricted_menu() {
    let source = corpus();
    let case = &source["cases"][0];
    let mut envelope: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    let effects: Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/effects.json"),
        )
        .unwrap(),
    )
    .unwrap();
    envelope.state["active_effects"] =
        effects["cases"][0]["envelope"]["state"]["active_effects"].clone();
    let op = case["operations"][0].clone();
    assert!(
        action_trace(envelope, vec![op])
            .unwrap_err()
            .contains("action prevention")
    );
}
