//! Frozen Python EffectManager lifecycle; payload execution is tested separately.
use goa2_core::{
    codec::StateEnvelope,
    data::{Registers, Symbols},
    diagnostics::effect_trace,
    effects::{CreationContext, EffectCreateSpec},
    world::NativeWorld,
};
use serde_json::Value;
use std::sync::{Arc, OnceLock};
#[path = "../../../test-support/cases.rs"]
mod fixtures;
fn path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/effects.json")
}
fn case(name: &str) -> &'static Value {
    static NAMED: OnceLock<std::collections::HashMap<String, Value>> = OnceLock::new();
    NAMED
        .get_or_init(|| {
            fixtures::selected_inputs(
                &path(),
                "cases",
                &[
                    "all-types-ordered-lifecycle",
                    "creation-origin-radius-False-None",
                    "creation-card-hand-False",
                    "creation-dedup-dormant",
                ],
            )
        })
        .get(name)
        .unwrap()
}
#[test]
fn effect_lifecycle_matches_python_full_state_and_finishing_order() {
    let checked = fixtures::visit_array(&path(), "cases", |case| {
        let envelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let got = effect_trace(envelope, case["operations"].as_array().unwrap().clone()).unwrap();
        assert_eq!(got["records"], case["expected"], "{}", case["name"]);
    });
    assert_eq!(checked, 123);
}
#[test]
fn effects_share_immutable_payloads_but_keep_activation_and_removal_private() {
    let case = case("all-types-ordered-lifecycle");
    let envelope: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    let mut symbols = Symbols::default();
    let root = NativeWorld::compile(&envelope, &mut symbols).unwrap();
    let mut a = root.clone();
    let mut b = root.clone();
    assert!(Arc::ptr_eq(
        &a.effects.instances[0].definition,
        &b.effects.instances[0].definition
    ));
    let id = a.effects.instances[0].id;
    let before = root.effects.export(&symbols);
    a.effects.activate_id(id, false);
    b.remove_effects(&[id]);
    assert_eq!(root.effects.export(&symbols), before);
    assert_ne!(a.effects.export(&symbols), before);
    assert_eq!(b.effects.instances.len() + 1, root.effects.instances.len());
    let mut restored = envelope.clone();
    a.export_cards(&mut restored, &symbols);
    a.export_spatial(&mut restored.state, &symbols);
    let copy = NativeWorld::compile(&restored, &mut symbols).unwrap();
    assert_eq!(copy.effects.export(&symbols), a.effects.export(&symbols));
}

#[test]
fn effect_creation_workers_share_templates_but_not_counters_or_origins() {
    let case = case("creation-origin-radius-False-None");
    let envelope: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
    let mut symbols = Symbols::default();
    let root = NativeWorld::compile(&envelope, &mut symbols).unwrap();
    let spec = EffectCreateSpec::compile(&case["operations"][0]["effect"], &mut symbols).unwrap();
    let registers = Registers::new(&symbols);
    let context = CreationContext {
        round: 3,
        turn: 2,
        actor: None,
        registers: &registers,
    };
    let mut a = root.clone();
    let mut b = root.clone();
    let mut sa = symbols.clone();
    let mut sb = symbols.clone();
    let ia = a.create_effect(&spec, &context, &mut sa).unwrap();
    let ib = b
        .create_effect(
            &spec,
            &CreationContext {
                round: 4,
                turn: 1,
                ..context
            },
            &mut sb,
        )
        .unwrap();
    assert_eq!((ia, ib), (0, 0));
    assert!(Arc::ptr_eq(
        &a.effects.instances[0].definition,
        &b.effects.instances[0].definition
    ));
    assert_eq!(a.effects.instances[0].created_round, 3);
    assert_eq!(b.effects.instances[0].created_round, 4);
    let piece = case["envelope"]["state"]["acting_piece_id"]
        .as_str()
        .unwrap();
    assert_eq!(sa.name(a.effects.instances[0].scope_origin.unwrap()), piece);
    a.effects.instances[0].scope_origin = None;
    assert!(b.effects.instances[0].scope_origin.is_some());
    b.remove_effects(&[b.effects.instances[0].id]);
    assert_eq!(a.effects.instances.len(), 1);
    assert!(b.effects.instances.is_empty());
    assert!(root.effects.instances.is_empty());
    let mut exported = envelope.clone();
    a.export_cards(&mut exported, &sa);
    a.export_spatial(&mut exported.state, &sa);
    assert_eq!(exported.state["next_entity_id"], 118);
    let mut restored = NativeWorld::compile(&exported, &mut sa).unwrap();
    assert_eq!(restored.effects.export(&sa), a.effects.export(&sa));
    restored.create_effect(&spec, &context, &mut sa).unwrap();
    let mut next = exported.clone();
    restored.export_spatial(&mut next.state, &sa);
    assert_eq!(next.state["next_entity_id"], 119);
    let mut unchanged = envelope.clone();
    root.export_spatial(&mut unchanged.state, &symbols);
    assert_eq!(unchanged.state["next_entity_id"], 117);
}
#[test]
fn exhausted_creation_counter_rejects_before_mutation_but_dedup_is_still_a_no_op() {
    for (name, duplicate) in [
        ("creation-card-hand-False", false),
        ("creation-dedup-dormant", true),
    ] {
        let case = case(name);
        let mut envelope: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        envelope.state["next_entity_id"] = serde_json::json!(i64::MAX);
        let mut symbols = Symbols::default();
        let mut world = NativeWorld::compile(&envelope, &mut symbols).unwrap();
        let spec =
            EffectCreateSpec::compile(&case["operations"][0]["effect"], &mut symbols).unwrap();
        let registers = Registers::new(&symbols);
        let context = CreationContext {
            round: 3,
            turn: 2,
            actor: None,
            registers: &registers,
        };
        let mut before = envelope.clone();
        world.export_cards(&mut before, &symbols);
        world.export_spatial(&mut before.state, &symbols);
        let result = world.create_effect(&spec, &context, &mut symbols);
        if duplicate {
            assert_eq!(result.unwrap(), 0);
        } else {
            assert!(result.unwrap_err().contains("counter exhausted"));
        }
        let mut after = envelope.clone();
        world.export_cards(&mut after, &symbols);
        world.export_spatial(&mut after.state, &symbols);
        assert_eq!(after.materialize().unwrap(), before.materialize().unwrap());
    }
}
