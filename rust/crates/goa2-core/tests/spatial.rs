//! Source-derived effect-aware hex filter scans.
use goa2_core::{
    data::{Registers, Symbols},
    diagnostics::hex_filter_batch,
    geometry::Hex,
    spatial::{HexFilter, Origin, SpatialContext},
    world::NativeWorld,
};
use serde_json::{Value, json};
fn corpus() -> &'static Value {
    static DATA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    DATA.get_or_init(|| {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/hex-filters.json");
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    })
}
#[test]
fn complete_effect_aware_hex_filter_scans_match_python() {
    for case in corpus()["cases"].as_array().unwrap() {
        let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let actual = hex_filter_batch(envelope, case["queries"].as_array().unwrap().clone())
            .unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
        let actual = actual["results"].as_array().unwrap();
        let expected = case["expected"].as_array().unwrap();
        assert_eq!(actual.len(), expected.len());
        for (i, (got, want)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(
                got, want,
                "{} query {i}: {}",
                case["name"], case["queries"][i]["filter"]
            );
        }
    }
}
#[test]
fn geometric_range_and_inverse_straight_line_keep_their_topology_contracts() {
    let case = corpus()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "base-topology-split")
        .unwrap();
    let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    let candidate = json!({"q":-1,"r":0,"s":1});
    let queries = vec![
        json!({"filter":{"type":"range_filter","origin_id":"hero_brogan","max_range":2,"distance_mode":"topology"},"candidates":[candidate]}),
        json!({"filter":{"type":"range_filter","origin_id":"hero_brogan","max_range":2,"distance_mode":"geometric"},"candidates":[candidate]}),
        json!({"filter":{"type":"not_in_straight_line_filter","origin_id":"hero_brogan"},"candidates":[candidate]}),
    ];
    assert_eq!(
        hex_filter_batch(envelope, queries).unwrap()["results"],
        json!([[false], [true], [true]])
    );
}
#[test]
fn obstacle_filters_use_worker_private_live_barrier_origins() {
    let case = corpus()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "barrier")
        .unwrap();
    let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    let mut symbols = Symbols::default();
    let world = NativeWorld::compile(&envelope, &mut symbols).unwrap();
    let actor = symbols.lookup("hero_brogan").unwrap();
    let source = symbols.lookup("hero_arien").unwrap();
    let context = Registers::new(&symbols);
    let coords = ["q", "r", "s"].map(|s| symbols.intern(s));
    let filter = HexFilter::Obstacle {
        wanted: false,
        exclude: Origin::default(),
    };
    let scan = |world: &NativeWorld| {
        let mut accepted = [true];
        filter
            .apply_scan(
                &SpatialContext {
                    world,
                    registers: &context,
                    symbols: &symbols,
                    actor: Some(actor),
                    frame: None,
                    coordinates: coords,
                    range_override: None,
                    through_override: None,
                },
                &[Hex::new(3, 0, -3).unwrap()],
                &mut accepted,
            )
            .unwrap();
        accepted
    };
    let mut changed = world.clone();
    assert_eq!(scan(&world), [false]);
    changed
        .move_entity(source, Hex::new(-3, 0, 3).unwrap())
        .unwrap();
    assert_eq!(scan(&changed), [true]);
    assert_eq!(scan(&world), [false]);
    assert!(std::sync::Arc::ptr_eq(
        &world.definitions,
        &changed.definitions
    ));
}
