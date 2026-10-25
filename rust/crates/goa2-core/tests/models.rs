//! Ground truth comes from actual Python stats and Hero methods.
use goa2_core::diagnostics::{card_trace, stats_batch};
use serde_json::Value;
#[test]
fn items_markers_card_stats_and_minion_defense_match_python() {
    let corpus: Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/models.json")).unwrap();
    for case in corpus["stats"].as_array().unwrap() {
        let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let result = stats_batch(envelope, case["queries"].as_array().unwrap().clone()).unwrap();
        assert_eq!(result["results"], case["expected"], "{}", case["name"]);
    }
}
#[test]
fn physical_card_lifecycle_and_partial_errors_match_python_full_state() {
    let corpus: Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/models.json")).unwrap();
    for case in corpus["cards"].as_array().unwrap() {
        let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let result = card_trace(envelope, case["operations"].as_array().unwrap().clone()).unwrap();
        let actual = result["records"].as_array().unwrap();
        let expected = case["expected"].as_array().unwrap();
        assert_eq!(actual.len(), expected.len());
        for (i, (got, want)) in actual.iter().zip(expected).enumerate() {
            assert!(got == want, "{} operation {} differs", case["name"], i);
        }
    }
}

fn effect_stats() -> &'static Value {
    static FIXTURE: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    FIXTURE.get_or_init(|| {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/effect-stats.json");
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    })
}
#[test]
fn effect_stats_immunity_and_topology_match_frozen_python() {
    for case in effect_stats()["cases"].as_array().unwrap() {
        let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let actual = stats_batch(envelope, case["queries"].as_array().unwrap().clone()).unwrap();
        for (i, (got, want)) in actual["results"]
            .as_array()
            .unwrap()
            .iter()
            .zip(case["expected"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(
                got, want,
                "{} query {}: {}",
                case["name"], i, case["queries"][i]
            );
        }
        assert_eq!(
            actual["results"].as_array().unwrap().len(),
            case["expected"].as_array().unwrap().len()
        );
    }
}
#[test]
fn effect_stat_workers_keep_activation_and_occupancy_private() {
    use goa2_core::{data::Symbols, stats::Stat, world::NativeWorld};
    use std::sync::Arc;
    let case = effect_stats()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "effect-stat-duration-THIS_TURN-current")
        .unwrap();
    let source = serde_json::from_value(case["envelope"].clone()).unwrap();
    let mut symbols = Symbols::default();
    let root = NativeWorld::compile(&source, &mut symbols).unwrap();
    let actor = symbols.lookup("hero_arien").unwrap();
    let a = root.clone();
    let mut b = root.clone();
    b.effects.instances[0].active = false;
    assert_eq!(a.computed_stat(actor, Stat::Attack, 4).unwrap(), 9);
    assert_eq!(b.computed_stat(actor, Stat::Attack, 4).unwrap(), 6);
    assert_eq!(root.computed_stat(actor, Stat::Attack, 4).unwrap(), 9);
    assert!(Arc::ptr_eq(&a.definitions, &b.definitions));
    assert!(Arc::ptr_eq(
        &a.effects.instances[0].definition,
        &b.effects.instances[0].definition
    ));
    b.remove_entity(actor).unwrap();
    assert!(root.position(actor).is_some());
}
