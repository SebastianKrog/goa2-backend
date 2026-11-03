use goa2_core::{
    data::Symbols, diagnostics::displacement_batch, geometry::Hex, validation::DisplacementKind,
    world::NativeWorld,
};
use serde_json::Value;
#[path = "../../../test-support/cases.rs"]
mod fixtures;
fn path(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}
#[test]
fn displacement_legality_matches_full_python_verdicts() {
    let checked = fixtures::visit_array(&path("displacement-validation.json"), "cases", |case| {
        let source = serde_json::from_value(case["envelope"].clone()).unwrap();
        let actual =
            displacement_batch(source, case["queries"].as_array().unwrap().clone()).unwrap();
        let actual = actual["results"].as_array().unwrap();
        let expected = case["expected"].as_array().unwrap();
        assert_eq!(actual.len(), expected.len());
        for (index, (got, want)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(
                got, want,
                "{} query {}: {}",
                case["name"], index, case["queries"][index]
            );
        }
    });
    assert_eq!(checked, 159);
}
#[test]
fn legality_reads_each_workers_live_effect_origin_without_mutating_shared_definitions() {
    let case = fixtures::selected_inputs(
        &path("displacement-validation.json"),
        "cases",
        &["displacement-blocked"],
    )
    .remove("displacement-blocked")
    .unwrap();
    let source = serde_json::from_value(case["envelope"].clone()).unwrap();
    let mut symbols = Symbols::default();
    let root = NativeWorld::compile(&source, &mut symbols).unwrap();
    let hero = symbols.lookup("hero_brogan").unwrap();
    let origin = symbols.lookup("hero_arien").unwrap();
    let a = root.clone();
    let mut b = root.clone();
    assert!(
        !a.can_displace(hero, Some(hero), DisplacementKind::Place, None)
            .unwrap()
            .allowed
    );
    b.move_entity(origin, Hex::new(-3, 0, 3).unwrap()).unwrap();
    assert!(
        b.can_displace(hero, Some(hero), DisplacementKind::Place, None)
            .unwrap()
            .allowed
    );
    assert!(
        !root
            .can_displace(hero, Some(hero), DisplacementKind::Place, None)
            .unwrap()
            .allowed
    );
    assert_eq!(root.position(origin), Some(Hex::new(0, 0, 0).unwrap()));
    assert!(std::sync::Arc::ptr_eq(&a.definitions, &b.definitions));
    assert!(std::sync::Arc::ptr_eq(
        &a.effects.instances[0].definition,
        &b.effects.instances[0].definition
    ));
}

fn action_case(name: &str) -> &'static Value {
    static NAMED: std::sync::OnceLock<std::collections::HashMap<String, Value>> =
        std::sync::OnceLock::new();
    NAMED
        .get_or_init(|| {
            fixtures::selected_inputs(
                &path("action-validation.json"),
                "cases",
                &["witch-ordinary-negative", "blocked"],
            )
        })
        .get(name)
        .unwrap()
}
fn negative_menu() -> &'static Value {
    static MENU: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    MENU.get_or_init(|| {
        fixtures::selected_inputs(
            &path("action-validation.json"),
            "menus",
            &["witch-ordinary-negative"],
        )
        .remove("witch-ordinary-negative")
        .unwrap()
    })
}
#[test]
fn actions_repeats_and_movement_caps_match_full_python_verdicts() {
    let checked = fixtures::visit_array(&path("action-validation.json"), "cases", |case| {
        let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let actual = goa2_core::diagnostics::action_legality_batch(
            envelope,
            case["queries"].as_array().unwrap().clone(),
        )
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
    assert_eq!(checked, 198);
}
#[test]
fn negative_attack_values_are_legal_without_a_specific_card_restriction() {
    let case = action_case("witch-ordinary-negative");
    let queries: Vec<Value> = case["queries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|q| {
            q["kind"] == "action"
                && q["action"] == "ATTACK"
                && q["unit"] == "hero_arien"
                && !q["card"].is_null()
        })
        .cloned()
        .collect();
    assert!(!queries.is_empty());
    let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    let actual = goa2_core::diagnostics::action_legality_batch(envelope, queries).unwrap();
    assert!(
        actual["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v["allowed"] == true)
    );
    let menu = negative_menu();
    let envelope = serde_json::from_value(menu["envelope"].clone()).unwrap();
    let records = goa2_core::diagnostics::action_trace(
        envelope,
        menu["operations"].as_array().unwrap().clone(),
    )
    .unwrap();
    assert!(
        records["records"].as_array().unwrap().iter().any(|record| {
            record["request"]["options"]
                .as_array()
                .is_some_and(|options| {
                    options.iter().any(|option| {
                        option["id"] == "ATTACK"
                            && option["metadata"]["value"]
                                .as_i64()
                                .is_some_and(|value| value < 0)
                    })
                })
        }),
        "ordinary negative attacks must be offered with their actual negative value"
    );
}

#[test]
fn action_repeat_and_movement_rules_follow_each_workers_live_origin() {
    let case = action_case("blocked");
    let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    let mut symbols = Symbols::default();
    let root = NativeWorld::compile(&envelope, &mut symbols).unwrap();
    let actor = symbols.lookup("hero_brogan").unwrap();
    let origin = symbols.lookup("hero_arien").unwrap();
    let context = goa2_core::data::Registers::new(&symbols);
    let a = root.clone();
    let mut b = root.clone();
    let check = |world: &NativeWorld| {
        [
            world
                .action_legality(actor, goa2_core::world::ActionKind::Attack, None, &context)
                .unwrap()
                .allowed,
            world.repeat_legality(actor).unwrap().allowed,
            world
                .movement_legality(actor, 2, false, None, &context)
                .unwrap()
                .allowed,
        ]
    };
    assert_eq!(check(&root), [false; 3]);
    b.move_entity(origin, Hex::new(-3, 0, 3).unwrap()).unwrap();
    assert_eq!(check(&b), [true; 3]);
    assert_eq!(check(&a), [false; 3]);
    assert_eq!(check(&root), [false; 3]);
    assert!(std::sync::Arc::ptr_eq(&a.definitions, &b.definitions));
    assert!(std::sync::Arc::ptr_eq(
        &a.effects.instances[0].definition,
        &b.effects.instances[0].definition
    ));
}
