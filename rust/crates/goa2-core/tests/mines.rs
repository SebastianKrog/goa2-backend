//! Mine paths are sets; explicit trigger/event/path ordering is still exact.
use goa2_core::{
    data::Symbols,
    diagnostics::{mine_path_batch, normalize_mine_choice_record},
    geometry::Hex,
    world::NativeWorld,
};
use serde_json::{Value, json};
fn corpus() -> &'static Value {
    static DATA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    DATA.get_or_init(|| {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/mine-paths.json");
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
    })
}
#[test]
fn mine_sets_and_exact_option_paths_match_python() {
    for case in corpus()["cases"].as_array().unwrap() {
        let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let actual = mine_path_batch(envelope, case["queries"].as_array().unwrap().clone())
            .unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
        let actual = actual["results"].as_array().unwrap();
        let expected = case["expected"].as_array().unwrap();
        assert_eq!(actual.len(), expected.len());
        for (i, (got, want)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(
                got, want,
                "{} query {i}: {}",
                case["name"], case["queries"][i]
            );
        }
    }
}
#[test]
fn mine_path_workers_read_private_current_token_locations() {
    let case = corpus()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "mixed")
        .unwrap();
    let source = serde_json::from_value(case["envelope"].clone()).unwrap();
    let mut symbols = Symbols::default();
    let world = NativeWorld::compile(&source, &mut symbols).unwrap();
    let actor = symbols.lookup("hero_arien").unwrap();
    let mine = symbols.lookup("mine_a").unwrap();
    let team = symbols.lookup("BLUE").unwrap();
    let origin = Hex::new(-2, 0, 2).unwrap();
    let has_mine = |world: &NativeWorld| {
        world
            .mine_paths(origin, 4, Some(actor), Some(team), &[actor])
            .unwrap()
            .iter()
            .flatten()
            .any(|p| p.mines.contains(&mine))
    };
    assert!(has_mine(&world));
    let mut changed = world.clone();
    changed.remove_entity(mine).unwrap();
    assert!(!has_mine(&changed));
    assert!(has_mine(&world));
    assert!(std::sync::Arc::ptr_eq(
        &world.definitions,
        &changed.definitions
    ));
}
#[test]
fn choice_normalization_does_not_hide_ordered_trigger_or_path_differences() {
    let path = json!([{"q":1,"r":0,"s":-1},{"q":0,"r":0,"s":0}]);
    let mut record = json!({"semantic_state":{"execution_context":{"chosen":["b","a"],"trigger_queue":["b","a"]}},"events":[{"target_id":"b"},{"target_id":"a"}],"request":{"prompt":"Choose which mines to move through","options":[{"id":"1","metadata":{"metadata":{"mine_hexes":path,"path":path}}},{"id":"0","metadata":{"metadata":{"mine_hexes":[],"path":path}}}]}});
    let before = record.clone();
    normalize_mine_choice_record(&mut record, &["chosen".into()]);
    assert_eq!(
        record["semantic_state"]["execution_context"]["chosen"],
        json!(["a", "b"])
    );
    assert_eq!(
        record["semantic_state"]["execution_context"]["trigger_queue"],
        before["semantic_state"]["execution_context"]["trigger_queue"]
    );
    assert_eq!(record["events"], before["events"]);
    assert_eq!(record["request"]["options"][0]["id"], "1");
    assert_eq!(
        record["request"]["options"][0]["metadata"]["metadata"]["path"],
        path
    );
}

#[test]
fn native_mine_order_does_not_depend_on_symbol_interning_history() {
    let case = corpus()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "mixed")
        .unwrap();
    let source: goa2_core::codec::StateEnvelope =
        serde_json::from_value(case["envelope"].clone()).unwrap();
    let scan = |seed: Option<&str>| {
        let mut symbols = Symbols::default();
        if let Some(seed) = seed {
            symbols.intern(seed);
        }
        let world = NativeWorld::compile(&source, &mut symbols).unwrap();
        let actor = symbols.lookup("hero_arien").unwrap();
        let team = symbols.lookup("BLUE").unwrap();
        world
            .mine_paths(
                Hex::new(-2, 0, 2).unwrap(),
                4,
                Some(actor),
                Some(team),
                &[actor],
            )
            .unwrap()
            .iter()
            .map(|options| {
                options
                    .iter()
                    .map(|path| {
                        path.mines
                            .iter()
                            .map(|id| symbols.name(*id).to_owned())
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(scan(None), scan(Some("mine_d")));
    assert_eq!(scan(None), scan(Some("mine_b")));
}
