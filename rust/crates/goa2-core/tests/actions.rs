//! Differential action resolution records every generated child before execution.
use goa2_core::{
    codec::StateEnvelope,
    data::Symbols,
    diagnostics::{action_trace, travel_zones},
    world::NativeWorld,
};
use serde_json::{Value, json};
use std::collections::HashMap;
#[path = "../../../test-support/cases.rs"]
mod fixtures;
fn path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
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
    let checked = fixtures::visit_array(&path("actions.json"), "cases", |case| {
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
    });
    assert_eq!(checked, 38);
}
#[test]
fn all_map_zone_candidates_and_non_enemy_entities_match_python() {
    let checked = fixtures::visit_array(&path("actions.json"), "travel", |case| {
        let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let result = travel_zones(envelope, case["queries"].as_array().unwrap().clone()).unwrap();
        assert_eq!(result["results"], case["expected"], "{}", case["name"]);
    });
    assert_eq!(checked, 13);
}
#[test]
fn fast_travel_uses_each_workers_current_locations() {
    let case = fixtures::selected_inputs(&path("actions.json"), "travel", &["generated-enemy"])
        .remove("generated-enemy")
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
fn effect_restrictions_and_card_hooks_match_full_menu_and_recipe_records() {
    let checked = fixtures::visit_array(&path("action-validation.json"), "menus", |case| {
        let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let mut actual = action_trace(envelope, case["operations"].as_array().unwrap().clone())
            .unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
        let records = actual["records"].as_array_mut().unwrap();
        let expected = case["expected"].as_array().unwrap();
        assert_eq!(records.len(), expected.len());
        for (i, (got, want)) in records.iter_mut().zip(expected).enumerate() {
            if !got["request"].is_null() {
                got["request"]["id"] = json!("request/1");
            }
            let mut ids = HashMap::new();
            for step in got["steps"].as_array_mut().unwrap() {
                normalize_step(step, &mut ids);
            }
            assert_eq!(
                got, want,
                "{} operation {i}: {}",
                case["name"], case["operations"][i]
            );
        }
    });
    assert_eq!(checked, 31);
}
