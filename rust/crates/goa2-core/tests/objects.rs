//! Source object semantics, not yet dense gameplay/frame binding coverage.
use goa2_core::{
    codec::{ObjectKind, StateEnvelope},
    objects::{ObjectArena, aggregate_trace},
};
use serde_json::{Value, json};

fn cases() -> impl Iterator<Item = Value> {
    cases_matching(|_| true)
}
fn cases_matching(predicate: impl Fn(&str) -> bool) -> impl Iterator<Item = Value> {
    let base = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
    let index: Value = serde_json::from_reader(std::io::BufReader::new(
        std::fs::File::open(base.join("object-arena.json")).unwrap(),
    ))
    .unwrap();
    assert_eq!(index["schema_version"], 2);
    index["cases"]
        .as_array()
        .unwrap()
        .clone()
        .into_iter()
        .filter(move |entry| predicate(entry["name"].as_str().unwrap()))
        .map(move |entry| {
            serde_json::from_reader(std::io::BufReader::new(
                std::fs::File::open(base.join(entry["path"].as_str().unwrap())).unwrap(),
            ))
            .unwrap()
        })
}
fn case(name: &str) -> Value {
    cases_matching(|candidate| candidate == name)
        .next()
        .unwrap()
}
fn envelope(case: &Value) -> StateEnvelope {
    serde_json::from_value(case["envelope"].clone()).unwrap()
}

#[test]
fn real_source_models_steps_and_containers_match_values_identity_and_ordered_keys() {
    let mut operations = 0;
    let mut checked = 0;
    for case in cases() {
        checked += 1;
        let source = envelope(&case);
        source.validate().unwrap();
        assert_eq!(serde_json::to_value(&source).unwrap(), case["envelope"]);
        let plan = case["operations"].as_array().unwrap().clone();
        operations += plan.len();
        assert_eq!(
            aggregate_trace(source.clone(), plan).unwrap()["records"],
            case["expected"],
            "{}",
            case["name"]
        );
    }
    assert_eq!(checked, 12);
    assert_eq!(operations, 49);
}

#[test]
fn shared_step_payloads_are_private_between_worker_forks_and_roundtrip_visible_identity() {
    let case = case("objects-step");
    let source = envelope(&case);
    let mut root = ObjectArena::compile(&source).unwrap();
    let plan = case["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| root.compile_mutation(op).unwrap())
        .collect::<Vec<_>>();
    root.apply(&plan[0]).unwrap();
    let before = root.compact_state().unwrap();
    let aliases = root.object_bindings().unwrap();
    let mut a = root.clone();
    let mut b = root.clone();
    a.apply(&plan[1]).unwrap();
    b.apply(&plan[2]).unwrap();
    assert_ne!(a.compact_state().unwrap(), b.compact_state().unwrap());
    assert_eq!(root.compact_state().unwrap(), before);
    assert_eq!(root.object_bindings().unwrap(), aliases);
    for worker in [root, a, b] {
        let boundary = worker.boundary(&source).unwrap();
        let imported = ObjectArena::compile(&boundary).unwrap();
        assert_eq!(
            worker.compact_state().unwrap(),
            imported.compact_state().unwrap()
        );
        assert_eq!(
            worker.object_bindings().unwrap(),
            imported.object_bindings().unwrap()
        );
    }
}

#[test]
fn malformed_aliases_cycles_and_frozen_writes_reject_before_semantic_mutation() {
    let case = case("objects-list");
    let source = envelope(&case);
    let mut bad = source.clone();
    bad.object_bindings[0].paths.clear();
    assert!(bad.validate().is_err());
    let mut bad = source.clone();
    let repeated = bad.object_bindings[0].paths[0].clone();
    bad.object_bindings[0].paths.push(repeated);
    assert!(bad.validate().is_err());
    let mut bad = source.clone();
    bad.object_bindings[0].kind = ObjectKind::List;
    assert!(bad.validate().is_err());
    let mut bad = source.clone();
    bad.object_bindings[0].deepcopy_shared = true;
    assert!(bad.validate().unwrap_err().contains("immutable model"));
    let mut arena = ObjectArena::compile(&source).unwrap();
    let target = case["operations"][0]["target"].clone();
    let cycle = arena
        .compile_mutation(&json!({"kind":"append","target":target,"value":{"alias":target}}))
        .unwrap();
    let before = arena.compact_state().unwrap();
    let aliases = arena.object_bindings().unwrap();
    assert!(arena.apply(&cycle).is_err());
    assert_eq!(arena.compact_state().unwrap(), before);
    assert_eq!(arena.object_bindings().unwrap(), aliases);
    assert!(
        arena
            .compile_mutation(
                &json!({"kind":"append","target":target,"value":{"literal":{"$goa2_instance":0}}})
            )
            .is_err()
    );
    let frozen = self::case("objects-frozen");
    let mut arena = ObjectArena::compile(&envelope(&frozen)).unwrap();
    let change = arena.compile_mutation(&frozen["operations"][0]).unwrap();
    let before = arena.compact_state().unwrap();
    assert!(arena.apply(&change).is_err());
    assert_eq!(arena.compact_state().unwrap(), before);
}
