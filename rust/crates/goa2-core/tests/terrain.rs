//! Full frozen-Python terrain, token traversal, and effect-aware movement parity.
#[path = "../../../test-support/cases.rs"]
mod cases;
use goa2_core::{
    data::Symbols,
    diagnostics::terrain_batch,
    geometry::{Hex, MovementQuery},
    world::NativeWorld,
};

fn corpus_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/terrain-validation.json")
}
fn named_cases() -> &'static std::collections::HashMap<String, serde_json::Value> {
    static SELECTED: std::sync::OnceLock<std::collections::HashMap<String, serde_json::Value>> =
        std::sync::OnceLock::new();
    SELECTED.get_or_init(|| {
        cases::selected_inputs(
            &corpus_path(),
            "cases",
            &["barrier", "mine", "mine-supply-only"],
        )
    })
}
#[test]
fn actor_obstacles_petrification_tokens_and_stateful_paths_match_python() {
    let checked = cases::visit_array(&corpus_path(), "cases", |case| {
        let source = serde_json::from_value(case["envelope"].clone()).unwrap();
        let actual = terrain_batch(source, case["queries"].as_array().unwrap().clone())
            .unwrap_or_else(|error| panic!("{}: {error}", case["name"]));
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
    });
    assert_eq!(checked, 196);
}
#[test]
fn moving_barrier_origins_are_private_to_workers() {
    let case = &named_cases()["barrier"];
    let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    let mut symbols = Symbols::default();
    let root = NativeWorld::compile(&envelope, &mut symbols).unwrap();
    let source = symbols.lookup("hero_arien").unwrap();
    let actor = symbols.lookup("hero_brogan").unwrap();
    let target = Hex::new(3, 0, -3).unwrap();
    assert!(root.obstacle_for_actor(target, Some(actor)).unwrap());
    let a = root.clone();
    let mut b = root.clone();
    b.move_entity(source, Hex::new(-3, 0, 3).unwrap()).unwrap();
    assert!(!b.obstacle_for_actor(target, Some(actor)).unwrap());
    assert!(a.obstacle_for_actor(target, Some(actor)).unwrap());
    assert!(root.obstacle_for_actor(target, Some(actor)).unwrap());
    assert!(std::sync::Arc::ptr_eq(&a.definitions, &b.definitions));
}
#[test]
fn shared_path_batch_does_not_cache_locations_between_mutations() {
    let case = &named_cases()["barrier"];
    let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    let mut symbols = Symbols::default();
    let root = NativeWorld::compile(&envelope, &mut symbols).unwrap();
    let actor = symbols.lookup("hero_brogan").unwrap();
    let source = symbols.lookup("hero_arien").unwrap();
    let query = MovementQuery {
        start: root.position(actor).unwrap(),
        ends: vec![Hex::new(3, 0, -3).unwrap()],
        max_steps: 3,
        ignore_obstacles: false,
        pass_through_obstacles: false,
    };
    let mut moved = root.clone();
    assert_eq!(
        root.movement_paths(&query, Some(actor), Some(&[actor]), &[])
            .unwrap(),
        [false]
    );
    moved
        .move_entity(source, Hex::new(-3, 0, 3).unwrap())
        .unwrap();
    assert_eq!(
        moved
            .movement_paths(&query, Some(actor), Some(&[actor]), &[])
            .unwrap(),
        [true]
    );
    assert_eq!(
        root.movement_paths(&query, Some(actor), Some(&[actor]), &[])
            .unwrap(),
        [false]
    );
}

#[test]
fn passable_tokens_allow_crossing_but_not_landing_or_a_moved_token() {
    let case = &named_cases()["mine"];
    let mut envelope: goa2_core::codec::StateEnvelope =
        serde_json::from_value(case["envelope"].clone()).unwrap();
    let corridor = [
        Hex::new(1, 0, -1).unwrap(),
        Hex::new(2, -1, -1).unwrap(),
        Hex::new(3, -2, -1).unwrap(),
    ];
    for tile in envelope.state["board"]["tiles"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        let hex: Hex = serde_json::from_value(tile["hex"].clone()).unwrap();
        tile["is_terrain"] = serde_json::json!(!corridor.contains(&hex));
    }
    let mut symbols = Symbols::default();
    let world = NativeWorld::compile(&envelope, &mut symbols).unwrap();
    let hero = symbols.lookup("hero_brogan").unwrap();
    let token = symbols.lookup("restriction-token").unwrap();
    let mut query = MovementQuery {
        start: corridor[0],
        ends: vec![corridor[1], corridor[2]],
        max_steps: 2,
        ignore_obstacles: false,
        pass_through_obstacles: false,
    };
    assert_eq!(
        world
            .movement_paths(&query, Some(hero), Some(&[hero]), &[hero])
            .unwrap(),
        [false, true]
    );
    assert_eq!(
        world
            .movement_paths(&query, Some(hero), Some(&[token]), &[token])
            .unwrap(),
        [false, false]
    );
    query.ignore_obstacles = true;
    assert_eq!(
        world
            .movement_paths(&query, Some(hero), Some(&[hero]), &[hero])
            .unwrap(),
        [true, true]
    );
}
#[test]
fn supply_only_mines_are_detected_without_rejecting_movement() {
    let case = &named_cases()["mine-supply-only"];
    let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    let mut symbols = Symbols::default();
    let world = NativeWorld::compile(&envelope, &mut symbols).unwrap();
    assert!(world.ensure_move_closure(false).is_ok());
    let semantic = envelope.materialize().unwrap();
    assert!(
        semantic["entity_locations"]
            .as_object()
            .unwrap()
            .keys()
            .any(|name| {
                symbols
                    .lookup(name)
                    .is_some_and(|id| world.entity(id).is_none() && world.position(id).is_some())
            })
    );
}
