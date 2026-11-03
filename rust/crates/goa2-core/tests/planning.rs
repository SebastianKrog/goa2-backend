//! Planning oracle compares actual phase functions before stack execution.
#[path = "../../../test-support/fixture_cache.rs"]
mod fixture_cache;
use goa2_core::{
    codec::StateEnvelope,
    data::Symbols,
    diagnostics::{planning_trace, tie_trace},
    planning::Planning,
};
use serde_json::{Value, json};
use std::collections::HashMap;
fn corpus() -> Value {
    serde_json::from_str(&fixture_cache::text(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/planning.json"),
    ))
    .unwrap()
}
fn normalize(records: &mut Value) {
    let mut ids = HashMap::new();
    for record in records.as_array_mut().unwrap() {
        for step in record["semantic_state"]["execution_stack"]
            .as_array_mut()
            .unwrap()
        {
            let raw = step["step_id"].as_str().unwrap().to_owned();
            let next = format!("dynamic-step/{}", ids.len() + 1);
            step["step_id"] = json!(ids.entry(raw).or_insert(next));
        }
    }
}
#[test]
fn planning_revelation_and_queued_actor_handoff_match_python_full_state() {
    for case in corpus()["cases"].as_array().unwrap() {
        let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let mut result =
            planning_trace(envelope, case["operations"].as_array().unwrap().clone()).unwrap();
        normalize(&mut result["records"]);
        for (i, (got, want)) in result["records"]
            .as_array()
            .unwrap()
            .iter()
            .zip(case["expected"].as_array().unwrap())
            .enumerate()
        {
            assert!(got == want, "{} operation {i} differs", case["name"]);
        }
        assert_eq!(
            result["records"].as_array().unwrap().len(),
            case["expected"].as_array().unwrap().len()
        );
    }
}
#[test]
fn unported_turn_end_and_active_effect_roots_are_rejected() {
    let mut envelope: StateEnvelope =
        serde_json::from_value(corpus()["cases"][0]["envelope"].clone()).unwrap();
    let mut operations = Vec::new();
    for team in envelope.state["teams"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        for hero in team["heroes"].as_array_mut().unwrap() {
            hero["hand"] = json!([]);
            operations.push(json!({"kind":"pass","hero":hero["id"]}));
        }
    }
    assert!(
        planning_trace(envelope.clone(), operations)
            .unwrap_err()
            .contains("turn-end closure")
    );
    envelope.state["active_effects"] = json!([{"unported":true}]);
    assert!(
        Planning::compile(&envelope, &mut Symbols::default())
            .unwrap_err()
            .contains("active-effect closure")
    );
}
#[test]
fn native_planning_clones_keep_private_pending_buffers() {
    let case = &corpus()["cases"][0];
    let envelope: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    let mut symbols = Symbols::default();
    let original = Planning::compile(&envelope, &mut symbols).unwrap();
    let world = goa2_core::world::NativeWorld::compile(&envelope, &mut symbols).unwrap();
    let hero = symbols.intern("hero_arien");
    let card = world.hero(hero).unwrap().hand[0];
    let mut copied_world = world.clone();
    let mut copied = original.clone();
    copied.commit(&mut copied_world, hero, card).unwrap();
    let mut one = envelope.clone();
    let mut two = envelope.clone();
    original.export(&mut one, &world, &symbols).unwrap();
    copied.export(&mut two, &copied_world, &symbols).unwrap();
    assert!(one.state["pending_inputs"].as_object().unwrap().is_empty());
    assert_eq!(
        two.state["pending_inputs"]["hero_arien"],
        json!({"$goa2_instance":card.0})
    );
    assert!(world.hero(hero).unwrap().hand.contains(&card));
    assert!(!copied_world.hero(hero).unwrap().hand.contains(&card));
    assert!(!world.card(card).unwrap().facedown);
    assert!(copied_world.card(card).unwrap().facedown);
}

#[test]
fn tie_arbitration_matches_python_prompts_state_and_delayed_coin_order() {
    for case in corpus()["ties"].as_array().unwrap() {
        let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let tied = serde_json::from_value(case["tied"].clone()).unwrap();
        let mut result = tie_trace(
            envelope,
            tied,
            case["operations"].as_array().unwrap().clone(),
        )
        .unwrap();
        normalize(&mut result["records"]);
        assert!(
            result["records"] == case["expected"],
            "{} differs",
            case["name"]
        );
    }
}

#[test]
fn uncommit_removes_only_the_first_done_entry_like_the_source_list() {
    let mut envelope: StateEnvelope =
        serde_json::from_value(corpus()["cases"][0]["envelope"].clone()).unwrap();
    envelope.state["planning_done"] = json!(["hero_arien", "hero_arien"]);
    let mut symbols = Symbols::default();
    let mut planning = Planning::compile(&envelope, &mut symbols).unwrap();
    let mut world = goa2_core::world::NativeWorld::compile(&envelope, &mut symbols).unwrap();
    let hero = symbols.intern("hero_arien");
    let card = world.hero(hero).unwrap().hand[0];
    planning.commit(&mut world, hero, card).unwrap();
    planning.uncommit(&mut world, hero).unwrap();
    planning.export(&mut envelope, &world, &symbols).unwrap();
    assert_eq!(envelope.state["planning_done"], json!(["hero_arien"]));
}
