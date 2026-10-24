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
