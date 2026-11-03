#[path = "../../../test-support/cases.rs"]
mod cases;
use goa2_core::{data::Symbols, diagnostics::targeting_batch, world::NativeWorld};
use std::sync::Arc;
fn corpus_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/targeting.json")
}
#[test]
fn all_effect_targeting_verdicts_and_ordered_filter_pools_match_python() {
    let checked = cases::visit_array(&corpus_path(), "states", |case| {
        let source = serde_json::from_value(case["envelope"].clone()).unwrap();
        let actual = targeting_batch(source, case["queries"].as_array().unwrap().clone()).unwrap();
        for (index, (got, want)) in actual["results"]
            .as_array()
            .unwrap()
            .iter()
            .zip(case["expected"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(
                got, want,
                "{} query {}: {}",
                case["name"], index, case["queries"][index]
            );
        }
        assert_eq!(
            actual["results"].as_array().unwrap().len(),
            case["expected"].as_array().unwrap().len()
        );
    });
    assert_eq!(checked, 66);
}
#[test]
fn smoke_and_actor_immunity_are_read_from_each_private_world() {
    let selected = cases::selected_inputs(&corpus_path(), "states", &["los-plain"]);
    let case = &selected["los-plain"];
    let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    let mut symbols = Symbols::default();
    let root = NativeWorld::compile(&envelope, &mut symbols).unwrap();
    let actor = symbols.lookup("hero_arien");
    let target = symbols.lookup("hero_mrak");
    assert!(!root.can_target(actor, target).unwrap().allowed);
    let mut clear = root.clone();
    let smoke = symbols.lookup("smoke").unwrap();
    clear.effects.instances.retain(|effect| effect.id != smoke);
    assert!(clear.can_target(actor, target).unwrap().allowed);
    assert!(!root.can_target(actor, target).unwrap().allowed);
    assert!(Arc::ptr_eq(&root.definitions, &clear.definitions));
}
