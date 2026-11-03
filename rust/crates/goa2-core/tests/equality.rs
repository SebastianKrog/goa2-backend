//! Equality expectations come from actual Python Card.__eq__ comparisons.
use goa2_core::{
    cards::CardId, codec::StateEnvelope, data::Symbols, diagnostics::card_equality_batch,
    world::NativeWorld,
};
use serde_json::Value;
#[path = "../../../test-support/cases.rs"]
mod cases;
fn path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/card-equality.json")
}
fn fixture(name: &str) -> Value {
    cases::selected_inputs(&path(), "cases", &[name])
        .remove(name)
        .unwrap()
}
#[test]
fn all_card_model_comparisons_match_python_including_private_revisions() {
    let checked = cases::visit_array(&path(), "cases", |case| {
        let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let result = card_equality_batch(envelope, case["queries"].as_array().unwrap().clone())
            .unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
        assert_eq!(result["results"], case["expected"], "{}", case["name"]);
    });
    assert_eq!(checked, 26);
}
#[test]
fn private_equality_state_survives_typed_export_and_restore() {
    let case = fixture("equal");
    let mut source: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    let a = CardId(case["queries"][1]["a"].as_u64().unwrap() as u32);
    let b = CardId(case["queries"][1]["b"].as_u64().unwrap() as u32);
    let mut symbols = Symbols::default();
    let mut world = NativeWorld::compile(&source, &mut symbols).unwrap();
    assert!(world.card_model_equal(a, b).unwrap());
    let index = world.card_by_instance[b.0 as usize].unwrap();
    world.cards[index].equality.as_mut().unwrap().revision = 21;
    world.export_cards(&mut source, &symbols);
    assert_eq!(
        source.instances[b.0 as usize]
            .equality
            .as_ref()
            .unwrap()
            .definition_revision,
        21
    );
    let source: StateEnvelope =
        serde_json::from_value(serde_json::to_value(source).unwrap()).unwrap();
    let mut symbols = Symbols::default();
    let restored = NativeWorld::compile(&source, &mut symbols).unwrap();
    assert!(!restored.card_model_equal(a, b).unwrap());
}
#[test]
fn absent_or_unsupported_equality_metadata_is_never_guessed() {
    let case = fixture("equal");
    let original: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    let a = CardId(case["queries"][1]["a"].as_u64().unwrap() as u32);
    let b = CardId(case["queries"][1]["b"].as_u64().unwrap() as u32);
    let mut source = original.clone();
    for instance in &mut source.instances {
        instance.equality = None;
    }
    let mut symbols = Symbols::default();
    let world = NativeWorld::compile(&source, &mut symbols).unwrap();
    assert!(world.card_model_equal(a, a).unwrap());
    assert!(
        world
            .card_model_equal(a, b)
            .unwrap_err()
            .contains("private equality metadata")
    );
    let mut source = original.clone();
    source.instances[b.0 as usize]
        .equality
        .as_mut()
        .unwrap()
        .json_metadata = false;
    let mut symbols = Symbols::default();
    let world = NativeWorld::compile(&source, &mut symbols).unwrap();
    assert!(
        world
            .card_model_equal(a, b)
            .unwrap_err()
            .contains("non-JSON")
    );
    let mut source = original;
    source.instances[b.0 as usize]
        .equality
        .as_mut()
        .unwrap()
        .model
        .clear();
    assert!(source.validate().unwrap_err().contains("equality metadata"));
}
