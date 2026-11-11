use super::*;
#[test]
fn dense_candidate_count_cleans_null_before_selector_or_query_output() {
    for kind in ["select_step", "multi_select", "count"] {
        for target in ["UNIT", "HEX"] {
            if kind == "multi_select" && target == "HEX" {
                continue;
            }
            for output in ["result", "_cmf_origin_hex"] {
                let mut envelope = source("action-execution-Min-HOLD");
                envelope.state["execution_stack"] = json!([{
                    "type":kind, "target_type":target, "step_id":"dense-count-cleanup",
                    "prompt":"Choose counted candidate", "output_key":output,
                    "min_selections":1, "max_selections":2,
                    "filters":[{"type":"count_match_filter", "min_count":0, "sub_filters":[]}]
                }]);
                envelope.state["execution_context"] = json!({"_cmf_origin_hex":null});
                let mut worker = NativeSession::compile(envelope).unwrap();
                assert!(worker.objects.is_none());
                let result = worker.advance_prefix(1).unwrap();
                let state = worker.semantic_state().unwrap();
                let origin = state["execution_context"].get("_cmf_origin_hex");
                if kind == "count" && output == "_cmf_origin_hex" {
                    assert!(origin.unwrap().as_i64().unwrap() > 0);
                } else {
                    assert!(origin.is_none(), "{kind} {target} {output}: {state}");
                    assert!(kind == "count" || result.request.is_some());
                }
            }
        }
    }
}
#[test]
fn inert_min_card_templates_import_but_reached_dense_cards_guard_before_pop() {
    for kind in [
        "convert_card_to_item",
        "apply_after_attack_card_text",
        "place_token",
    ] {
        for active in [false, true] {
            let mut envelope = source("action-execution-Min-HOLD");
            envelope.state["execution_stack"] = json!([{"type":kind,"step_id":"dense-min-card","active_if_key":"enabled","card_key":"target_card","hero_id":"hero_min"}]);
            envelope.state["execution_context"] = json!({"enabled":active});
            let mut worker = NativeSession::compile(envelope).unwrap();
            let before = worker.semantic_state().unwrap();
            if active {
                assert!(
                    worker
                        .advance_prefix(1)
                        .unwrap_err()
                        .to_string()
                        .contains("authoritative object bindings")
                );
                assert_eq!(worker.semantic_state().unwrap(), before);
            } else {
                worker.advance_prefix(1).unwrap();
                assert!(worker.stack.is_empty());
            }
        }
    }
}
#[test]
fn inert_wasp_query_templates_import_but_reached_dense_queries_guard_before_pop() {
    for kind in ["check_unit_type", "combine_boolean_context"] {
        for active in [false, true] {
            let mut envelope = source("action-execution-Min-HOLD");
            envelope.state["execution_stack"] = json!([{
                "type":kind, "step_id":"dense-wasp-query", "active_if_key":"enabled",
                "unit_key":"subject", "expected_type":"HERO", "key_a":"left", "key_b":"right",
                "operation":"AND", "output_key":"query_result"
            }]);
            envelope.state["execution_context"] = json!({"enabled":active,"subject":"hero_min"});
            let mut worker = NativeSession::compile(envelope).unwrap();
            let before = worker.semantic_state().unwrap();
            if active {
                assert!(
                    worker
                        .advance_prefix(1)
                        .unwrap_err()
                        .to_string()
                        .contains("authoritative object bindings")
                );
                assert_eq!(worker.semantic_state().unwrap(), before);
            } else {
                worker.advance_prefix(1).unwrap();
                assert!(worker.stack.is_empty());
            }
        }
    }
}
#[test]
fn inert_hanu_templates_import_but_reached_dense_instructions_are_guarded() {
    for kind in [
        "co_directional_drag",
        "set_card_initiative",
        "schedule_action_control",
        "schedule_journey_return",
    ] {
        for enabled in [false, true] {
            let mut envelope = source("action-execution-Min-HOLD");
            envelope.state["execution_stack"] = json!([{
                "type": kind, "active_if_key": "hanu_enabled", "step_id": "fixture/hanu-inert"
            }]);
            envelope.state["execution_context"]["hanu_enabled"] = json!(enabled);
            let mut worker = NativeSession::compile(envelope).unwrap();
            let before = worker.semantic_state().unwrap();
            if enabled {
                assert!(matches!(
                    worker.advance(&ExecutionBudget::default()),
                    Err(EngineError::Unsupported(_))
                ));
                assert_eq!(worker.semantic_state().unwrap(), before);
                assert_eq!(worker.stack.len(), 1);
            } else {
                worker.advance(&ExecutionBudget::default()).unwrap();
                assert!(worker.stack.is_empty());
            }
        }
    }
}

#[test]
fn inert_distance_templates_import_but_reached_dense_distance_is_guarded_before_pop() {
    for active in [false, true] {
        let mut envelope = source("action-execution-Min-HOLD");
        envelope.state["execution_stack"] = json!([{
            "type":"check_distance", "step_id":"dense-distance", "active_if_key":"enabled",
            "unit_a_id":"hero_min", "unit_b_id":"hero_hanu", "operator":"==", "threshold":1,
            "output_key":"distance_result"
        }]);
        envelope.state["execution_context"] = json!({"enabled":active});
        let mut worker = NativeSession::compile(envelope).unwrap();
        let before = worker.semantic_state().unwrap();
        if active {
            assert!(
                matches!(worker.advance(&ExecutionBudget::default()),Err(EngineError::Unsupported(message)) if message.contains("authoritative object bindings"))
            );
            assert_eq!(worker.semantic_state().unwrap(), before);
        } else {
            worker.advance(&ExecutionBudget::default()).unwrap();
            assert!(worker.stack.is_empty());
        }
    }
}
fn source(name: &str) -> StateEnvelope {
    #[derive(serde::Deserialize)]
    struct Entry {
        name: String,
        path: String,
    }
    #[derive(serde::Deserialize)]
    struct Index {
        schema_version: u32,
        cases: Vec<Entry>,
    }
    #[derive(serde::Deserialize)]
    struct Input {
        envelope: StateEnvelope,
    }
    static INDEX: std::sync::OnceLock<Index> = std::sync::OnceLock::new();
    let base = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
    let index = INDEX.get_or_init(|| {
        serde_json::from_reader(std::io::BufReader::new(
            crate::fixture_cache::open(base.join("stack-traces.json")).unwrap(),
        ))
        .unwrap()
    });
    assert_eq!(index.schema_version, 2);
    let entry = index.cases.iter().find(|case| case.name == name).unwrap();
    let input: Input = serde_json::from_reader(std::io::BufReader::new(
        crate::fixture_cache::open(base.join(&entry.path)).unwrap(),
    ))
    .unwrap();
    input.envelope
}

#[test]
fn card_action_workers_run_distinct_secondary_choices_with_shared_programs() {
    let mut root = NativeSession::compile(source("action-execution-Arien-MOVEMENT")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert!(matches!(request.choices, RequestChoices::Action(_)));
    let before = root.semantic_state().unwrap();
    let mut moving = root.clone();
    let mut holding = root.clone();
    let movement = moving.import_selection(&json!("MOVEMENT")).unwrap();
    let hold = holding.import_selection(&json!("HOLD")).unwrap();
    moving.submit(request.id, movement).unwrap();
    holding.submit(request.id, hold).unwrap();
    assert!(
        moving
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .is_some()
    );
    assert!(
        holding
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .is_none()
    );
    assert_eq!(
        holding.semantic_state().unwrap()["execution_context"]["action_done"],
        true
    );
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(&moving.steps, &holding.steps));
    assert!(Arc::ptr_eq(
        &moving.world.definitions,
        &holding.world.definitions
    ));
}
#[test]
fn generated_action_recipe_restores_before_movement_and_never_fakes_primary_text() {
    let mut session = NativeSession::compile(source("action-execution-Arien-MOVEMENT")).unwrap();
    let request = session
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let movement = session.import_selection(&json!("MOVEMENT")).unwrap();
    session.submit(request.id, movement).unwrap();
    session.advance_prefix(1).unwrap();
    let mut restored = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
    let first = session
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let second = restored
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let mut expected = session.request_json(&first);
    let mut actual = restored.request_json(&second);
    expected.as_object_mut().unwrap().remove("id");
    actual.as_object_mut().unwrap().remove("id");
    assert_eq!(expected, actual);
    let destination = match first.choices {
        RequestChoices::Hexes(v) => v[0],
        _ => panic!("movement prompt expected"),
    };
    let selected = serde_json::to_value(destination).unwrap();
    let a = session.import_selection(&selected).unwrap();
    let b = restored.import_selection(&selected).unwrap();
    session.submit(first.id, a).unwrap();
    restored.submit(second.id, b).unwrap();
    session.advance(&ExecutionBudget::default()).unwrap();
    restored.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(
        session.semantic_state().unwrap(),
        restored.semantic_state().unwrap()
    );
    let mut envelope = source("action-execution-Arien-MOVEMENT");
    for definition in &mut envelope.definitions {
        if definition.fields["id"] == "noble_blade" {
            definition
                .fields
                .insert("effect_id".into(), json!("angry_roar"));
        }
    }
    let mut unported = NativeSession::compile(envelope).unwrap();
    let request = unported
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let primary = match &request.choices {
        RequestChoices::Action(menu) => menu
            .options
            .iter()
            .find(|option| option.primary_effect)
            .unwrap()
            .kind
            .name(),
        _ => panic!("action prompt expected"),
    };
    let primary = unported.import_selection(&json!(primary)).unwrap();
    unported.submit(request.id, primary).unwrap();
    assert!(
        matches!(unported.advance(&ExecutionBudget::default()),Err(EngineError::Unsupported(message)) if message.contains("primary card effect"))
    );
}

#[test]
fn unexecuted_primary_effect_catalog_does_not_block_min_actions() {
    let mut worker = NativeSession::compile(source("action-execution-Min-HOLD")).unwrap();
    let request = worker
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let hold = worker.import_selection(&json!("HOLD")).unwrap();
    worker.submit(request.id, hold).unwrap();
    worker.advance(&ExecutionBudget::default()).unwrap();
}
#[test]
fn reached_primary_effect_keeps_the_unsafe_blast_mine_combination_guarded_before_pop() {
    let root = NativeSession::compile(source("action-execution-Min-HOLD")).unwrap();
    assert!(root.primary_blast_guard);
    let indices = root
        .steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| {
            (step.primary_template && matches!(step.instruction, Instruction::CreateEffect(_)))
                .then_some(index)
        })
        .collect::<Vec<_>>();
    assert!(!indices.is_empty());
    for index in indices {
        let mut worker = root.clone();
        if let Some(key) = worker.steps[index].conditions.active {
            worker.context.set(key, Datum::Bool(true));
        }
        if let Some(key) = worker.steps[index].conditions.skip {
            worker.context.set(key, Datum::Null);
        }
        worker.stack = vec![worker.steps[index].initial.clone()];
        let before = worker.semantic_state().unwrap();
        assert!(
            matches!(worker.advance(&ExecutionBudget::default()),Err(EngineError::Unsupported(message)) if message.contains("blast mine dynamically-created effect closure not compiled"))
        );
        assert_eq!(worker.semantic_state().unwrap(), before);
        assert_eq!(worker.stack.len(), 1);
    }
}

#[test]
fn inert_copied_primary_templates_import_but_reached_dense_copy_is_guarded_before_pop() {
    let root = NativeSession::compile(source("action-execution-Min-HOLD")).unwrap();
    let index = root
        .steps
        .iter()
        .position(|step| {
            step.primary_template && matches!(step.instruction, Instruction::LivePerformCard)
        })
        .unwrap();
    for enabled in [false, true] {
        let mut worker = root.clone();
        let key = worker.steps[index].conditions.active.unwrap();
        worker.context.set(key, Datum::Bool(enabled));
        worker.stack = vec![worker.steps[index].initial.clone()];
        let before = worker.semantic_state().unwrap();
        if enabled {
            assert!(
                matches!(worker.advance(&ExecutionBudget::default()), Err(EngineError::Unsupported(message)) if message.contains("copied-card action requires authoritative object bindings"))
            );
            assert_eq!(worker.semantic_state().unwrap(), before);
            assert_eq!(worker.stack.len(), 1);
        } else {
            worker.advance(&ExecutionBudget::default()).unwrap();
            assert!(worker.stack.is_empty());
        }
    }
}

#[test]
fn inert_resource_templates_import_but_reached_dense_theft_stays_guarded_before_pop() {
    let root = NativeSession::compile(source("action-execution-Min-HOLD")).unwrap();
    let index = root
        .steps
        .iter()
        .position(|step| {
            step.primary_template && matches!(step.instruction, Instruction::LiveResource)
        })
        .unwrap();
    for enabled in [false, true] {
        let mut worker = root.clone();
        let key = worker.steps[index].conditions.active.unwrap();
        worker.context.set(key, Datum::Bool(enabled));
        worker.stack = vec![worker.steps[index].initial.clone()];
        let before = worker.semantic_state().unwrap();
        if enabled {
            assert!(
                matches!(worker.advance(&ExecutionBudget::default()),Err(EngineError::Unsupported(message)) if message.contains("resource step requires authoritative object bindings"))
            );
            assert_eq!(worker.semantic_state().unwrap(), before);
            assert_eq!(worker.stack.len(), 1);
        } else {
            worker.advance(&ExecutionBudget::default()).unwrap();
            assert!(worker.stack.is_empty());
        }
    }
}

#[test]
fn inert_card_and_round_queries_import_but_reached_dense_queries_are_guarded_before_pop() {
    let root = NativeSession::compile(source("action-execution-Min-HOLD")).unwrap();
    let indices: Vec<_> = root
        .steps
        .iter()
        .enumerate()
        .filter_map(|(i, step)| {
            (step.primary_template
                && matches!(
                    step.instruction,
                    Instruction::LiveAdjacentEnemies
                        | Instruction::LiveCardCount
                        | Instruction::LiveRoundDefeat
                ))
            .then_some(i)
        })
        .collect();
    assert!(indices.len() >= 3);
    assert!(indices.iter().any(|index| matches!(
        root.steps[*index].instruction,
        Instruction::LiveAdjacentEnemies
    )));
    for index in indices {
        for enabled in [false, true] {
            let mut worker = root.clone();
            let key = Arc::make_mut(&mut worker.symbols).intern("query_enabled");
            Arc::make_mut(&mut worker.steps)[index].conditions.active = Some(key);
            Arc::make_mut(&mut worker.steps)[index].conditions.skip = None;
            worker.context.set(key, Datum::Bool(enabled));
            worker.stack = vec![worker.steps[index].initial.clone()];
            let before = worker.semantic_state().unwrap();
            if enabled {
                assert!(
                    matches!(worker.advance(&ExecutionBudget::default()), Err(EngineError::Unsupported(message)) if message.contains("card and round queries require authoritative object bindings"))
                );
                assert_eq!(worker.semantic_state().unwrap(), before);
                assert_eq!(worker.stack.len(), 1);
            } else {
                worker.advance(&ExecutionBudget::default()).unwrap();
                assert!(worker.stack.is_empty());
            }
        }
    }
}

#[test]
fn inert_spell_lifecycle_templates_import_but_reached_dense_operations_are_guarded_before_pop() {
    for name in [
        "cast_spell",
        "wish_sequence",
        "prepare_spellbook",
        "remove_prepared_spells",
    ] {
        for enabled in [false, true] {
            let mut envelope = source("action-execution-Min-HOLD");
            let mut wire = combat_template(name).unwrap();
            wire["active_if_key"] = json!("spell_enabled");
            envelope.state["execution_stack"] = json!([wire]);
            envelope.state["execution_context"]["spell_enabled"] = json!(enabled);
            let mut worker = NativeSession::compile(envelope).unwrap();
            let before = worker.semantic_state().unwrap();
            if enabled {
                assert!(
                    matches!(worker.advance(&ExecutionBudget::default()),Err(EngineError::Unsupported(message)) if message.contains("spell lifecycle requires authoritative object bindings"))
                );
                assert_eq!(worker.semantic_state().unwrap(), before);
                assert_eq!(worker.stack.len(), 1);
            } else {
                worker.advance(&ExecutionBudget::default()).unwrap();
                assert!(worker.stack.is_empty());
            }
        }
    }
}

#[test]
fn inert_spell_rule_primitives_import_but_reached_dense_queries_and_respawns_are_guarded_before_pop()
 {
    let data: Value = serde_json::from_str(include_str!(
        "../../../../content/catalogs/spell-builders.json"
    ))
    .unwrap();
    for wire in [
        &data["recipes"]["disintegrate"][1]["fields"],
        &data["recipes"]["create_undead"][0]["fields"],
    ] {
        for enabled in [false, true] {
            let mut envelope = source("action-execution-Min-HOLD");
            let mut wire = wire.clone();
            wire["active_if_key"] = json!("spell_rule_enabled");
            envelope.state["execution_stack"] = json!([wire]);
            envelope.state["execution_context"]["spell_rule_enabled"] = json!(enabled);
            let mut worker = NativeSession::compile(envelope).unwrap();
            let before = worker.semantic_state().unwrap();
            if enabled {
                assert!(
                    matches!(worker.advance(&ExecutionBudget::default()),Err(EngineError::Unsupported(message)) if message.contains("authoritative object bindings"))
                );
                assert_eq!(worker.semantic_state().unwrap(), before);
                assert_eq!(worker.stack.len(), 1);
            } else {
                worker.advance(&ExecutionBudget::default()).unwrap();
                assert!(worker.stack.is_empty());
            }
        }
    }
}

#[test]
fn primary_text_distinguishes_hidden_no_op_from_empty_identifier_fallback() {
    for hidden in [false, true] {
        let mut envelope = source("action-execution-Arien-MOVEMENT");
        for definition in &mut envelope.definitions {
            if definition.kind == crate::codec::InstanceKind::Card {
                definition.fields.insert("effect_id".into(), json!(""));
            }
        }
        for instance in &mut envelope.instances {
            if instance.fields.contains_key("is_facedown") {
                instance.fields.insert("is_facedown".into(), json!(hidden));
            }
        }
        let mut text = combat_template("card_text").unwrap();
        text["hero_id"] = json!("hero_arien");
        let mut done = combat_template("context").unwrap();
        done["key"] = json!("action_done");
        done["value"] = json!(true);
        envelope.state["execution_stack"] = json!([done, text]);
        let mut session = NativeSession::compile(envelope).unwrap();
        let result = session.advance(&ExecutionBudget::default());
        if hidden {
            assert!(result.unwrap().request.is_none());
            assert_eq!(
                session.semantic_state().unwrap()["execution_context"]["action_done"],
                true
            );
        } else {
            assert!(result.is_ok());
            assert_eq!(
                session.semantic_state().unwrap()["execution_context"]["attack_is_ranged"],
                false
            );
        }
    }
}
#[test]
fn multi_selection_workers_and_restored_partial_choices_are_independent() {
    let mut root = NativeSession::compile(source("action-execution-multi-maximum")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let before = root.semantic_state().unwrap();
    let mut left = root.clone();
    let mut right = root.clone();
    let a = left.import_selection(&json!("native_token_0")).unwrap();
    let b = right.import_selection(&json!("native_token_1")).unwrap();
    left.submit(request.id, a).unwrap();
    right.submit(request.id, b).unwrap();
    left.advance(&ExecutionBudget::default()).unwrap();
    right.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(
        left.semantic_state().unwrap()["execution_context"]["picked"],
        json!(["native_token_0"])
    );
    assert_eq!(
        right.semantic_state().unwrap()["execution_context"]["picked"],
        json!(["native_token_1"])
    );
    assert_eq!(root.semantic_state().unwrap(), before);
    let mut restored = NativeSession::compile(left.export_envelope().unwrap()).unwrap();
    let request = restored
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let done = restored.import_selection(&json!("DONE")).unwrap();
    restored.submit(request.id, done).unwrap();
    restored.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(
        restored.semantic_state().unwrap()["execution_context"]["picked"],
        json!(["native_token_0"])
    );
    assert_eq!(
        restored.semantic_state().unwrap()["execution_context"]["multi_done"],
        true
    );
}

#[test]
fn clear_restores_between_each_generated_token_removal_without_losing_events() {
    let mut ready = NativeSession::compile(source("action-execution-tokens")).unwrap();
    let mut request = ready
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let clear = ready.import_selection(&json!("CLEAR")).unwrap();
    ready.submit(request.id, clear).unwrap();
    request = ready
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    for i in 0..3 {
        let token = ready
            .import_selection(&json!(format!("native_token_{i}")))
            .unwrap();
        ready.submit(request.id, token).unwrap();
        if i < 2 {
            request = ready
                .advance(&ExecutionBudget::default())
                .unwrap()
                .request
                .unwrap();
        }
    }
    let mut baseline = ready.clone();
    let complete = baseline.advance(&ExecutionBudget::default()).unwrap();
    assert!(complete.request.is_none());
    assert_eq!(complete.events.len(), 3);
    let expected_events = complete
        .events
        .iter()
        .map(|e| baseline.event_json(e))
        .collect::<Vec<_>>();
    let expected_state = baseline.semantic_state().unwrap();
    for cut in 0..12 {
        let mut partial = ready.clone();
        let prefix = partial.advance_prefix(cut).unwrap();
        assert!(prefix.request.is_none());
        let mut events = prefix
            .events
            .iter()
            .map(|e| partial.event_json(e))
            .collect::<Vec<_>>();
        let mut restored = NativeSession::compile(partial.export_envelope().unwrap()).unwrap();
        let suffix = restored.advance(&ExecutionBudget::default()).unwrap();
        assert!(suffix.request.is_none());
        events.extend(suffix.events.iter().map(|e| restored.event_json(e)));
        assert_eq!(events, expected_events, "cut {cut}");
        assert_eq!(
            restored.semantic_state().unwrap(),
            expected_state,
            "cut {cut}"
        );
    }
}

#[test]
fn turn_handoffs_restore_every_instruction_and_prompt() {
    fn run(mut session: NativeSession, restore: bool, movement: bool) -> (Value, Vec<Value>) {
        let mut events = Vec::new();
        for _ in 0..200 {
            let result = session.advance_prefix(1).unwrap();
            events.extend(result.events.iter().map(|e| session.event_json(e)));
            let answer = result.request.as_ref().map(|req| {
                let wire = session.request_json(req);
                let options = wire["options"].as_array().unwrap();
                let find = |id: &str| options.iter().find(|o| o["id"] == id);
                let option = if movement { find("MOVEMENT") } else { None }
                    .or_else(|| find("HOLD"))
                    .or_else(|| find("CONFIRM"))
                    .or_else(|| find("SKIP"))
                    .unwrap_or(&options[0]);
                (
                    session.request_token(req.id),
                    option["metadata"]
                        .get("hex")
                        .unwrap_or(&option["id"])
                        .clone(),
                )
            });
            if restore {
                session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
            }
            if let Some((token, value)) = answer {
                let request = if restore {
                    session.advance_prefix(1).unwrap().request.unwrap()
                } else {
                    result.request.unwrap()
                };
                assert_eq!(session.request_token(request.id), token);
                let selected = session.import_selection(&value).unwrap();
                session.submit(request.id, selected).unwrap();
            }
            if session.stack.is_empty() {
                return (session.semantic_state().unwrap(), events);
            }
        }
        panic!("turn did not terminate");
    }
    for name in [
        "turn-handoff-unique",
        "turn-handoff-cross-team-tie",
        "turn-handoff-movement",
        "turn-respawn-fallback",
        "turn-respawn-token",
        "turn-finalize-piece",
        "turn-handoff-abort",
        "turn-finalize-guess-own",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        let expected = run(root.clone(), false, name.ends_with("movement"));
        assert_eq!(
            run(root, true, name.ends_with("movement")),
            expected,
            "{name}"
        );
    }
}
#[test]
fn turn_workers_keep_cards_counters_and_position_snapshots_private() {
    let root = NativeSession::compile(source("turn-finalize-ordinary")).unwrap();
    let before = root.semantic_state().unwrap();
    let mut a = root.clone();
    let mut b = root.clone();
    let hero = a.symbols.lookup("hero_arien").unwrap();
    let to = a
        .world
        .tile_hexes()
        .iter()
        .copied()
        .find(|h| !a.world.obstacle(*h))
        .unwrap();
    a.world.move_entity(hero, to).unwrap();
    a.world.combat.turn = 2;
    a.advance(&ExecutionBudget::default()).unwrap();
    b.advance(&ExecutionBudget::default()).unwrap();
    let av = a.semantic_state().unwrap();
    let bv = b.semantic_state().unwrap();
    assert_eq!(av["phase"], "PLANNING");
    assert_eq!(av["turn"], 3);
    assert_eq!(bv["turn"], 2);
    assert_ne!(av["last_turn_positions"], bv["last_turn_positions"]);
    assert_eq!(av["last_turn_positions"], av["entity_locations"]);
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(
        a.flow_program.as_ref().unwrap(),
        b.flow_program.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        a.turn_rules.as_ref().unwrap(),
        b.turn_rules.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(&a.world.definitions, &b.world.definitions));
}
#[test]
fn resolved_witching_hour_is_a_compiled_card_hook() {
    let envelope = source("turn-finalize-ordinary");
    let mut symbols = Symbols::default();
    let mut plain = NativeWorld::compile(&envelope, &mut symbols).unwrap();
    let hero = symbols.lookup("hero_arien").unwrap();
    plain.resolve_current_card(hero, Some(1)).unwrap();
    assert!(!plain.has_uncompiled_auras);
    assert_eq!(plain.computed_stat(hero, Stat::Initiative, 3).unwrap(), 3);
    let mut changed = envelope;
    changed
        .definitions
        .iter_mut()
        .find(|d| d.fields.get("effect_id") == Some(&json!("noble_blade")))
        .unwrap()
        .fields
        .insert("effect_id".into(), json!("witching_hour"));
    let mut symbols = Symbols::default();
    let mut guarded = NativeWorld::compile(&changed, &mut symbols).unwrap();
    let hero = symbols.lookup("hero_arien").unwrap();
    assert!(!guarded.has_uncompiled_auras);
    guarded.resolve_current_card(hero, Some(1)).unwrap();
    assert!(!guarded.has_uncompiled_auras);
    assert_eq!(guarded.computed_stat(hero, Stat::Initiative, 3).unwrap(), 3);
    assert_eq!(
        crate::auras::CardAura::compile("future_unported_aura", true),
        crate::auras::CardAura::Uncompiled
    );
}
#[test]
fn active_movement_aura_freezes_permission_and_restores_every_instruction() {
    fn run(mut session: NativeSession, restore: bool) -> (Value, Vec<Value>) {
        let mut events = Vec::new();
        for _ in 0..40 {
            let result = session.advance_prefix(1).unwrap();
            events.extend(result.events.iter().map(|e| session.event_json(e)));
            let answer = result.request.as_ref().map(|r| {
                let RequestChoices::Hexes(hexes) = &r.choices else {
                    panic!("hexes");
                };
                let chosen = hexes
                    .iter()
                    .find(|h| h.coordinates() == (2, 0, -2))
                    .unwrap_or(&hexes[0]);
                serde_json::to_value(chosen).unwrap()
            });
            if restore {
                session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
            }
            if let Some(value) = answer {
                let req = if restore {
                    session.advance_prefix(1).unwrap().request.unwrap()
                } else {
                    result.request.unwrap()
                };
                let value = session.import_selection(&value).unwrap();
                session.submit(req.id, value).unwrap();
                if restore {
                    session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
                }
            }
            if session.stack.is_empty() {
                return (session.semantic_state().unwrap(), events);
            }
        }
        panic!("aura movement did not finish");
    }
    for name in [
        "aura-movement-ordinary",
        "aura-movement-straight",
        "aura-movement-full-distance",
        "aura-movement-computed-range",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        let expected = run(root.clone(), false);
        assert_eq!(run(root, true), expected, "{name}");
        assert_eq!(
            expected.0["entity_locations"]["hero_xargatha"],
            json!({"q":2,"r":0,"s":-2})
        );
    }
    let mut root = NativeSession::compile(source("aura-movement-ordinary")).unwrap();
    root.advance_prefix(1).unwrap();
    let mut paused = root.export_envelope().unwrap();
    // The movement-start aura is frozen in both generated child instructions.
    assert!(
        paused.state["execution_stack"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["pass_through_obstacles"] == true)
    );
    for team in paused.state["teams"].as_object_mut().unwrap().values_mut() {
        for hero in team["heroes"].as_array_mut().unwrap() {
            if hero["id"] == "hero_xargatha" {
                hero["level"] = json!(7);
            }
        }
    }
    let root = NativeSession::compile(paused).unwrap();
    let state = run(root, true).0;
    assert_eq!(
        state["entity_locations"]["hero_xargatha"],
        json!({"q":2,"r":0,"s":-2})
    );
}

#[test]
fn aura_movement_workers_share_programs_but_not_positions_or_adjacency_counts() {
    let mut root = NativeSession::compile(source("aura-movement-computed-range")).unwrap();
    let hero = root.symbols.lookup("hero_xargatha").unwrap();
    let enemy = root.symbols.lookup("hero_brogan").unwrap();
    assert_eq!(
        root.world.computed_stat(hero, Stat::Movement, 1).unwrap(),
        2
    );
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let before = root.semantic_state().unwrap();
    let mut a = root.clone();
    let mut b = root.clone();
    a.world.remove_entity(enemy).unwrap();
    assert_eq!(a.world.computed_stat(hero, Stat::Movement, 1).unwrap(), 1);
    assert_eq!(b.world.computed_stat(hero, Stat::Movement, 1).unwrap(), 2);
    let to = crate::geometry::Hex::new(2, 0, -2).unwrap();
    a.submit(request.id, native_hex(to, a.coordinates)).unwrap();
    a.advance(&ExecutionBudget::default()).unwrap();
    // Removing an adjacent enemy after the action starts does not shrink its frozen range.
    assert_eq!(a.world.position(hero), Some(to));
    let stay = crate::geometry::Hex::new(0, 0, 0).unwrap();
    b.submit(request.id, native_hex(stay, b.coordinates))
        .unwrap();
    assert!(
        b.advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .is_none()
    );
    assert_ne!(a.world.position(hero), b.world.position(hero));
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(&a.steps, &b.steps));
    assert!(Arc::ptr_eq(&a.world.definitions, &b.world.definitions));
}

#[test]
fn conditional_stats_follow_live_private_context_and_explicit_cards() {
    let root = NativeSession::compile(source("aura-conditional-changing-context-trace")).unwrap();
    let hero = root.symbols.lookup("hero_sabina").unwrap();
    let key = root.symbols.lookup("performing_card_id").unwrap();
    let basic = root.symbols.lookup("point_blank_shot").unwrap();
    let current = root.world.hero(hero).unwrap().current.unwrap();
    let other =
        root.world.definitions.cards[root.world.card(current).unwrap().definition].printed_id;
    let basic_card = root.world.card_for_hero_printed(hero, basic).unwrap();
    assert!(
        root.world
            .computed_stat_for_card(hero, Stat::Attack, 1, Some(crate::cards::CardId(u32::MAX)))
            .is_err()
    );
    let before = root.semantic_state().unwrap();
    let mut a = root.clone();
    let mut b = root.clone();
    a.context.set(key, Datum::Text(basic));
    b.context.set(key, Datum::Text(other));
    assert_eq!(
        a.world
            .computed_stat_in_context(hero, Stat::Attack, 1, &a.context)
            .unwrap(),
        3
    );
    assert_eq!(
        b.world
            .computed_stat_in_context(hero, Stat::Attack, 1, &b.context)
            .unwrap(),
        1
    );
    assert_eq!(a.world.computed_stat(hero, Stat::Attack, 1).unwrap(), 1);
    assert_eq!(
        a.world
            .computed_stat_for_card(hero, Stat::Attack, 1, Some(current))
            .unwrap(),
        1
    );
    assert_eq!(
        b.world.card_stats(hero, basic_card).unwrap().primary_value,
        4
    );
    a.context.set(key, Datum::Text(other));
    assert_eq!(
        a.world
            .computed_stat_in_context(hero, Stat::Range, 1, &a.context)
            .unwrap(),
        1
    );
    a.context.set(key, Datum::Text(basic));
    assert_eq!(
        a.world
            .computed_stat_in_context(hero, Stat::Range, 1, &a.context)
            .unwrap(),
        3
    );
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(&a.world.definitions, &b.world.definitions));
    assert!(Arc::ptr_eq(&a.steps, &b.steps));
}

fn run_instruction_restores(mut session: NativeSession, restore: bool) -> (Value, Vec<Value>) {
    let mut events = Vec::new();
    for _ in 0..150 {
        let result = session.advance_prefix(1).unwrap();
        events.extend(result.events.iter().map(|e| session.event_json(e)));
        let answer = result.request.as_ref().map(|req| {
            let wire = session.request_json(req);
            let options = wire["options"].as_array().unwrap();
            let option = options
                .iter()
                .find(|o| o["id"] == "ATTACK")
                .unwrap_or(&options[0]);
            option["metadata"]
                .get("hex")
                .unwrap_or(&option["id"])
                .clone()
        });
        if restore {
            session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
        }
        if let Some(value) = answer {
            let req = if restore {
                session.advance_prefix(1).unwrap().request.unwrap()
            } else {
                result.request.unwrap()
            };
            let value = session.import_selection(&value).unwrap();
            session.submit(req.id, value).unwrap();
            if restore {
                session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
            }
        }
        if session.stack.is_empty() {
            return (session.semantic_state().unwrap(), events);
        }
    }
    panic!("conditional continuation did not finish");
}

#[test]
fn nested_card_stats_adjacency_and_discard_or_defeat_restore_every_instruction() {
    for name in [
        "aura-conditional-changing-context-trace",
        "aura-conditional-primary-basic-trace",
        "aura-conditional-primary-ctx-nonbasic-trace",
        "discard-or-defeat-hand",
        "discard-or-defeat-one-card",
        "discard-or-defeat-empty",
        "discard-or-defeat-offboard",
        "discard-or-defeat-missing",
        "discard-or-defeat-falsy",
        "discard-or-defeat-unknown",
        "discard-or-defeat-minion",
        "discard-or-defeat-killer-literal",
        "discard-or-defeat-killer-key",
        "discard-or-defeat-killer-fallback",
        "discard-or-defeat-no-actor",
        "discard-or-defeat-multi-owner",
        "discard-or-defeat-multi-piece",
        "discard-or-defeat-multi-offboard",
        "discard-or-defeat-multi-empty",
        "discard-or-defeat-immunity-source",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}

#[test]
fn passive_continuations_restore_at_every_instruction() {
    for name in [
        "passive-enemy",
        "passive-empty",
        "passive-friendly",
        "passive-minion",
        "passive-missing",
        "passive-falsy",
        "passive-offboard",
        "passive-unknown",
        "passive-piece",
        "passive-owner",
        "passive-level-seven",
        "passive-facedown",
        "passive-regular",
        "passive-unresolved",
        "passive-alias",
        "passive-spent",
        "passive-override",
        "passive-wrong-trigger",
        "passive-offer-yes",
        "passive-offer-no",
        "passive-offer-skip",
        "passive-offer-low-level",
        "passive-offer-missing-card",
        "passive-offer-facedown",
        "passive-offer-no-owner",
        "passive-offer-wrong-trigger",
        "passive-mark",
        "passive-mark-facedown",
        "passive-mark-missing",
        "passive-mark-override-actor",
        "passive-push",
        "passive-push-empty",
        "passive-push-twice",
        "passive-push-blocked",
        "passive-primary",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
fn passive_workers_keep_usage_and_context_private() {
    let root = NativeSession::compile(source("passive-enemy")).unwrap();
    let before = root.semantic_state().unwrap();
    let mut a = root.clone();
    let mut b = root.clone();
    assert!(Arc::ptr_eq(&a.passive_rules, &b.passive_rules));
    assert!(Arc::ptr_eq(&a.steps, &b.steps));
    let key = b.symbols.lookup("push_victim_id").unwrap();
    b.context.remove(key);
    assert!(
        b.advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .is_none()
    );
    let request = a
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let wire = a.request_json(&request);
    let value = a.import_selection(&wire["options"][0]["id"]).unwrap();
    a.submit(request.id, value).unwrap();
    assert!(
        a.advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .is_none()
    );
    let hero = a.symbols.lookup("hero_sabina").unwrap();
    let card = a.world.hero(hero).unwrap().ultimate.unwrap();
    assert_eq!(a.world.card(card).unwrap().passive_uses, 1);
    assert_eq!(b.world.card(card).unwrap().passive_uses, 0);
    assert_eq!(root.semantic_state().unwrap(), before);
}

#[test]
fn optional_passive_rechecks_live_card_before_acceptance() {
    let mut root = NativeSession::compile(source("passive-offer-yes")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let before = root.semantic_state().unwrap();
    let hero = root.symbols.lookup("hero_sabina").unwrap();
    let card = root.world.hero(hero).unwrap().ultimate.unwrap();
    for remove in [false, true] {
        let mut worker = root.clone();
        if remove {
            let index = worker.world.hero_index(hero).unwrap();
            worker.world.heroes[index].ultimate = None;
        } else {
            worker.world.card_mut(card).unwrap().facedown = true;
        }
        worker.submit(request.id, Datum::Text(worker.yes)).unwrap();
        assert!(
            worker
                .advance(&ExecutionBudget::default())
                .unwrap()
                .request
                .is_none()
        );
        assert_eq!(worker.world.card(card).unwrap().passive_uses, 0);
    }
    assert_eq!(root.semantic_state().unwrap(), before);
}
#[test]
fn unported_passive_providers_remain_explicit_and_do_not_mutate_the_root() {
    let root = NativeSession::compile(source("passive-enemy")).unwrap();
    let before = root.semantic_state().unwrap();
    let hero = root.symbols.lookup("hero_sabina").unwrap();
    let card = root.world.hero(hero).unwrap().ultimate.unwrap();
    let mut worker = root.clone();
    let definition = worker.world.card(card).unwrap().definition;
    let effect = worker.symbols.lookup("battle_fury").unwrap();
    Arc::make_mut(&mut worker.world.definitions).cards[definition].effect = effect;
    let trigger = worker.symbols.lookup("after_card_discard").unwrap();
    assert!(
        worker
            .passive_rules
            .scan(&worker.world, Some(hero), None, trigger)
            .unwrap_err()
            .contains("eligible passive")
    );
    assert_eq!(worker.world.card(card).unwrap().passive_uses, 0);
    assert_eq!(root.semantic_state().unwrap(), before);
}

#[test]
fn tactical_primaries_restore_at_every_instruction() {
    for name in [
        "primary-back_to_back-ordinary",
        "primary-back_to_back-items",
        "primary-listen_up-ordinary",
        "primary-listen_up-items",
        "primary-roger_roger-ordinary",
        "primary-roger_roger-items",
        "primary-ready_and_waiting-ordinary",
        "primary-ready_and_waiting-items",
        "primary-troop_movement-ordinary",
        "primary-troop_movement-items",
        "primary-marching_orders-ordinary",
        "primary-marching_orders-items",
        "primary-path_to_victory-ordinary",
        "primary-path_to_victory-items",
        "primary-close_support-ordinary",
        "primary-close_support-items",
        "primary-covering_fire-ordinary",
        "primary-covering_fire-items",
        "primary-shootout-ordinary",
        "primary-shootout-items",
        "primary-bullet_hell-ordinary",
        "primary-bullet_hell-items",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
fn repeat_continuations_restore_at_every_instruction() {
    for name in [
        "repeat-once",
        "repeat-twice",
        "repeat-decline",
        "repeat-skip",
        "repeat-once-then-no",
        "repeat-nested",
        "repeat-already-one",
        "repeat-negative-done",
        "repeat-zero-max",
        "repeat-negative-max",
        "repeat-done",
        "repeat-no-actor",
        "repeat-offboard",
        "repeat-unknown-actor",
        "repeat-piece",
        "repeat-lane-push",
        "repeat-lane-checked",
        "repeat-lane-empty",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restoration matrix; requires complete named case family"
)]
fn standalone_mine_choices_restore_at_every_instruction() {
    for variant in [
        "plain",
        "one-dud",
        "one-blast",
        "mixed",
        "friendly",
        "unowned",
        "unknown-owner",
        "supply-only",
        "entity-priority",
        "pool-mismatch",
        "terrain-mine",
        "map-gap",
        "offboard-start",
        "no-actor",
        "actor-other",
        "actor-empty",
        "barrier",
        "empty-denial",
        "split",
        "isolation",
        "topology-dormant",
        "bound-owner",
        "controlled",
    ] {
        for mode in ["first", "last", "invalid", "numeric"] {
            let name = format!("mine-choice-{variant}-{mode}");
            let root = NativeSession::compile(source(&name)).unwrap();
            assert_eq!(
                run_instruction_restores(root.clone(), true),
                run_instruction_restores(root, false),
                "{name}"
            );
        }
    }
}
#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restoration matrix; requires complete named case family"
)]
fn ordered_mine_trigger_chains_restore_tokens_discards_and_rollback_flags() {
    for variant in [
        "dud",
        "blast",
        "mixed",
        "reverse",
        "duplicate-blast",
        "offboard-blast",
        "unknown",
        "hero-id",
        "empty-literal",
        "context-list",
        "literal-victim",
        "missing-victim",
        "empty-hand",
        "actor-other",
        "actor-empty",
        "dud-effects",
        "offboard-dud-effects",
    ] {
        let name = format!("mine-trigger-{variant}");
        let root = NativeSession::compile(source(&name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restoration matrix; requires complete named case family"
)]
fn automatic_mine_movement_restores_every_instruction_and_generated_snapshot() {
    for kind in ["dud", "blast"] {
        for route in ["corridor", "choice"] {
            for root_type in ["sequence", "direct"] {
                let name = format!("mine-move-{kind}-{route}-{root_type}-first");
                let root = NativeSession::compile(source(&name)).unwrap();
                assert_eq!(
                    run_instruction_restores(root.clone(), true),
                    run_instruction_restores(root, false),
                    "{name}"
                );
            }
        }
    }
    for variant in [
        "piece",
        "actor-other",
        "actor-none",
        "actor-empty",
        "literal",
        "literal-requeue",
        "custom-key",
        "straight",
        "prechosen",
        "prechosen-empty",
        "prechosen-unknown",
        "dud-effects",
        "controlled",
        "second-move",
        "two-straight",
        "two-prechosen",
        "sequence-select",
        "supply-only",
        "pool-mismatch",
        "skip",
    ] {
        let name = format!("mine-move-{variant}");
        let root = NativeSession::compile(source(&name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
fn generated_mine_requeue_identity_is_identical_after_restore() {
    let mut direct = NativeSession::compile(source("mine-move-dud-choice-sequence-first")).unwrap();
    direct.advance_prefix(1).unwrap();
    let mut restored = NativeSession::compile(direct.export_envelope().unwrap()).unwrap();
    assert_eq!(
        direct.semantic_state().unwrap(),
        restored.semantic_state().unwrap()
    );
    direct.advance_prefix(1).unwrap();
    restored.advance_prefix(1).unwrap();
    assert_eq!(
        direct.semantic_state().unwrap(),
        restored.semantic_state().unwrap()
    );
    let a = direct.advance_prefix(1).unwrap().request.unwrap();
    let b = restored.advance_prefix(1).unwrap().request.unwrap();
    assert_eq!(direct.request_json(&a), restored.request_json(&b));
}
#[test]
fn automatic_mine_choice_workers_keep_movement_and_discards_private() {
    let mut root = NativeSession::compile(source("mine-move-blast-choice-direct-first")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let RequestChoices::Mines(options) = &request.choices else {
        panic!("mine choices")
    };
    let hit = options.iter().position(|p| !p.mines.is_empty()).unwrap();
    let avoid = options.iter().position(|p| p.mines.is_empty()).unwrap();
    let before = root.semantic_state().unwrap();
    let moves = root.resolution_choices(request.owner).unwrap();
    let mut workers = [root.clone(), root.clone()];
    for (worker, index) in workers.iter_mut().zip([hit, avoid]) {
        let DecisionMove::Input { selection, .. } = &moves[index] else {
            panic!("mine choices must be input decisions");
        };
        assert_eq!(worker.selection_json(selection), json!(index.to_string()));
        worker.submit(request.id, selection.clone()).unwrap();
        // This completes the move, with only the hit worker awaiting discard.
    }
    let hit_result = workers[0].advance(&ExecutionBudget::default()).unwrap();
    let avoid_result = workers[1].advance(&ExecutionBudget::default()).unwrap();
    assert!(hit_result.request.is_some());
    assert!(avoid_result.request.is_none());
    let mine = root.symbols.lookup("mine_a").unwrap();
    assert!(workers[0].world.position(mine).is_none());
    assert!(workers[1].world.position(mine).is_some());
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(&workers[0].steps, &workers[1].steps));
    assert!(Arc::ptr_eq(
        &workers[0].world.definitions,
        &workers[1].world.definitions
    ));
}
#[test]
fn mine_choice_workers_keep_different_sets_private() {
    let mut root = NativeSession::compile(source("mine-choice-mixed-first")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let RequestChoices::Mines(options) = &request.choices else {
        panic!("mine choices")
    };
    assert!(options.len() > 1);
    let before = root.semantic_state().unwrap();
    let mut first = root.clone();
    let mut last = root.clone();
    let a = first.import_selection(&json!("0")).unwrap();
    let b = last
        .import_selection(&json!((options.len() - 1).to_string()))
        .unwrap();
    first.submit(request.id, a).unwrap();
    last.submit(request.id, b).unwrap();
    first.advance(&ExecutionBudget::default()).unwrap();
    last.advance(&ExecutionBudget::default()).unwrap();
    let key = root.symbols.lookup("triggered_mine_ids").unwrap();
    assert_ne!(first.context.get(key), last.context.get(key));
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(&first.steps, &last.steps));
    assert!(Arc::ptr_eq(
        &first.world.definitions,
        &last.world.definitions
    ));
}
#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restoration matrix; requires complete named case family"
)]
fn effect_discard_steps_restore_at_every_instruction() {
    for variant in [
        "plain",
        "current",
        "current-facedown",
        "facedown",
        "duplicate",
        "missing-card",
        "inactive",
        "old-active",
        "wrong-source",
        "empty-card",
        "off-scope",
        "multiple",
        "linked-expiry",
        "empty-hand",
        "no-basic",
        "basic",
        "non-basic",
        "immune",
        "friendly-immunity",
        "no-immunity-source",
        "irrelevant-provider",
    ] {
        let name = format!("discard-effect-{variant}-first");
        let root = NativeSession::compile(source(&name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
    for variant in [
        "plain",
        "current",
        "immune",
        "linked-expiry",
        "irrelevant-provider",
    ] {
        for mode in ["move", "trigger"] {
            let name = format!("discard-effect-blast-{variant}-{mode}-first");
            let root = NativeSession::compile(source(&name)).unwrap();
            assert_eq!(
                run_instruction_restores(root.clone(), true),
                run_instruction_restores(root, false),
                "{name}"
            );
        }
    }
}
#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restoration matrix; requires complete named case family"
)]
fn selected_shield_expiry_survives_restore_without_finishing_steps() {
    for variant in ["plain", "current", "linked-expiry", "basic", "non-basic"] {
        let mut direct =
            NativeSession::compile(source(&format!("discard-effect-{variant}-last"))).unwrap();
        direct.advance_prefix(1).unwrap();
        let mut restored = NativeSession::compile(direct.export_envelope().unwrap()).unwrap();
        let a = direct.advance_prefix(1).unwrap().request.unwrap();
        let b = restored.advance_prefix(1).unwrap().request.unwrap();
        assert_eq!(direct.request_json(&a), restored.request_json(&b));
        let shield = direct.symbols.lookup("stone_carapace").unwrap();
        assert!(
            direct.request_json(&a)["options"]
                .as_array()
                .unwrap()
                .iter()
                .any(|o| o["id"] == "stone_carapace")
        );
        direct.submit(a.id, Datum::Text(shield)).unwrap();
        restored.submit(b.id, Datum::Text(shield)).unwrap();
        direct.advance_prefix(1).unwrap();
        restored.advance_prefix(1).unwrap();
        restored = NativeSession::compile(restored.export_envelope().unwrap()).unwrap();
        let a = direct.advance(&ExecutionBudget::default()).unwrap();
        let b = restored.advance(&ExecutionBudget::default()).unwrap();
        assert!(a.request.is_none() && b.request.is_none());
        assert_eq!(
            direct.semantic_state().unwrap(),
            restored.semantic_state().unwrap()
        );
        assert!(
            direct
                .world
                .effects
                .instances
                .iter()
                .all(|e| e.card != Some(shield))
        );
        assert!(
            direct
                .symbols
                .lookup("should_not_finish")
                .is_none_or(|key| direct.context.get(key).is_none())
        );
        let card = direct.world.card_by_printed(shield).unwrap();
        assert!(!direct.world.card(card).unwrap().active);
    }
}
#[test]
fn shield_and_hand_discard_workers_keep_cards_and_effects_private() {
    let mut root = NativeSession::compile(source("discard-effect-plain-last")).unwrap();
    let req = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let before = root.semantic_state().unwrap();
    let shield = root.symbols.lookup("stone_carapace").unwrap();
    let hand = root.request_json(&req)["options"][0]["id"].clone();
    let mut a = root.clone();
    let mut b = root.clone();
    a.submit(req.id, Datum::Text(shield)).unwrap();
    let hand = b.import_selection(&hand).unwrap();
    b.submit(req.id, hand).unwrap();
    a.advance(&ExecutionBudget::default()).unwrap();
    b.advance(&ExecutionBudget::default()).unwrap();
    assert!(
        a.world
            .effects
            .instances
            .iter()
            .all(|e| e.card != Some(shield))
    );
    assert!(
        b.world
            .effects
            .instances
            .iter()
            .any(|e| e.card == Some(shield))
    );
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(&a.steps, &b.steps));
    assert!(Arc::ptr_eq(&a.world.definitions, &b.world.definitions));
}
#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restoration matrix; requires complete named case family"
)]
fn granted_and_ordinary_wild_hunt_restore_every_instruction() {
    for variant in [
        "plain",
        "offboard",
        "inactive",
        "wrong-source",
        "missing-card",
        "unknown-grant",
        "empty-grant",
        "no-grant",
        "used",
        "negative-uses",
        "facedown-carrier",
        "duplicate-provider",
        "scope-ignored",
        "old-active",
        "empty-provider-id",
        "no-actor",
        "mine",
        "mine-linked-provider",
        "actions-only-cap",
        "movement-cap",
        "ordinary",
        "ordinary-low-level",
        "ordinary-facedown",
        "ordinary-dedup",
    ] {
        for entry in ["scan", "direct"] {
            let name = format!("provider-{variant}-{entry}-yes");
            let root = NativeSession::compile(source(&name)).unwrap();
            assert_eq!(
                run_instruction_restores(root.clone(), true),
                run_instruction_restores(root, false),
                "{name}"
            );
        }
    }
}
#[test]
fn provider_rechecks_live_identity_before_accepted_offer_executes() {
    let mut root = NativeSession::compile(source("provider-plain-scan-yes")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let before = root.semantic_state().unwrap();
    let hero = root.symbols.lookup("hero_mrak").unwrap();
    let position = root.world.position(hero);
    for variant in 0..5 {
        let mut worker = root.clone();
        match variant {
            0 => worker.world.effects.instances.clear(),
            1 => worker.world.effects.instances[0].active = false,
            2 => {
                worker.world.effects.instances[0].source =
                    worker.symbols.lookup("hero_min").unwrap()
            }
            3 => worker.world.effects.instances[0].card = None,
            _ => worker.world.effects.instances[0].granted_passive = None,
        }
        worker.submit(request.id, Datum::Text(worker.yes)).unwrap();
        assert!(
            worker
                .advance(&ExecutionBudget::default())
                .unwrap()
                .request
                .is_none()
        );
        assert_eq!(worker.world.position(hero), position);
        assert!(
            worker
                .world
                .effects
                .instances
                .iter()
                .all(|e| e.passive_uses == 0)
        );
    }
    assert_eq!(root.semantic_state().unwrap(), before);
}
#[test]
fn provider_counter_is_private_and_does_not_increment_the_foreign_card() {
    let mut root = NativeSession::compile(source("provider-plain-scan-yes")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let before = root.semantic_state().unwrap();
    let carrier = root.world.effects.instances[0].card.unwrap();
    let physical = root.world.card_by_printed(carrier).unwrap();
    let mut accepted = root.clone();
    let mut declined = root.clone();
    accepted
        .submit(request.id, Datum::Text(accepted.yes))
        .unwrap();
    let no = declined.import_selection(&json!("NO")).unwrap();
    declined.submit(request.id, no).unwrap();
    declined.advance(&ExecutionBudget::default()).unwrap();
    let target = accepted
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let value = accepted.request_json(&target)["options"][0]["metadata"]["hex"].clone();
    let value = accepted.import_selection(&value).unwrap();
    accepted.submit(target.id, value).unwrap();
    accepted.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(accepted.world.effects.instances[0].passive_uses, 1);
    assert_eq!(declined.world.effects.instances[0].passive_uses, 0);
    assert_eq!(accepted.world.card(physical).unwrap().passive_uses, 7);
    let restored = NativeSession::compile(accepted.export_envelope().unwrap()).unwrap();
    assert_eq!(
        accepted.semantic_state().unwrap(),
        restored.semantic_state().unwrap()
    );
    assert_eq!(restored.world.effects.instances[0].passive_uses, 1);
    assert!(Arc::ptr_eq(
        &accepted.world.effects.instances[0].definition,
        &declined.world.effects.instances[0].definition
    ));
    assert!(Arc::ptr_eq(
        &accepted.passive_bodies,
        &declined.passive_bodies
    ));
    assert_eq!(root.semantic_state().unwrap(), before);
}
#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restoration matrix; requires complete named case family"
)]
fn provider_marking_matches_rows_even_when_inactive_or_carrier_is_missing() {
    for variant in [
        "plain",
        "inactive",
        "wrong-source",
        "missing-card",
        "empty-provider-id",
        "no-actor",
        "used",
        "negative-uses",
        "ordinary",
        "ordinary-facedown",
    ] {
        let name = format!("provider-mark-{variant}");
        let root = NativeSession::compile(source(&name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
fn effect_combat_protection_and_granted_gun_restore_every_instruction() {
    for name in [
        "combat-effect-protection-card-False",
        "combat-effect-protection-two-skip-False",
        "combat-effect-protection-two-invalid-False",
        "combat-effect-protection-all-decline-False",
        "combat-effect-protection-totem-priority-False",
        "combat-effect-protection-totem-offboard-False",
        "combat-effect-protection-card-linked-False",
        "combat-effect-protection-tried-True",
        "combat-effect-protection-pending-legacy",
        "combat-effect-protection-pending-selection",
        "combat-effect-protection-heavy-True",
        "combat-effect-bounty-twice",
        "combat-effect-bounty-last-coin",
        "combat-effect-bounty-multiple",
        "combat-effect-bounty-split",
        "combat-effect-bounty-isolated",
        "combat-effect-expiry-source",
        "combat-effect-expiry-owned-card",
        "combat-effect-expiry-foreign-card",
        "combat-effect-expiry-empty-token",
        "combat-effect-negative--16-None",
        "combat-effect-negative--16--17",
        "combat-effect-force-empty",
        "combat-effect-force-immune",
        "combat-effect-gun-hand",
        "combat-effect-gun-empty",
        "combat-effect-gun-victim-carrier",
        "combat-effect-create-minion_protection",
        "combat-effect-create-minion_defeat_bounty",
        "combat-effect-terminal",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restore scan needs the verified canonical corpus"
)]
fn effect_reaction_and_illusion_traces_restore_every_instruction() {
    let base = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
    let index: Value = serde_json::from_reader(std::io::BufReader::new(
        crate::fixture_cache::open(base.join("stack-traces.json")).unwrap(),
    ))
    .unwrap();
    let names = index["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|case| case["name"].as_str())
        .filter(|name| name.starts_with("reaction-effect-"))
        .collect::<Vec<_>>();
    assert!(!names.is_empty());
    for name in names {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
fn illusion_defense_uses_each_workers_live_actor_and_restores_it() {
    let root = NativeSession::compile(source("reaction-effect-illusion-plain-True")).unwrap();
    let before = root.semantic_state().unwrap();
    let mut same_actor = root.clone();
    let mut different_actor = root.clone();
    different_actor.planning.actor = Some(different_actor.symbols.lookup("hero_hanu").unwrap());
    let a = same_actor
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let b = different_actor
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert_eq!(
        same_actor.request_json(&a)["context"]["minion_modifier"],
        -2
    );
    assert_eq!(
        different_actor.request_json(&b)["context"]["minion_modifier"],
        0
    );
    let mut restored = NativeSession::compile(different_actor.export_envelope().unwrap()).unwrap();
    let req = restored.advance_prefix(1).unwrap().request.unwrap();
    assert_eq!(
        restored.request_json(&req),
        different_actor.request_json(&b)
    );
    assert_eq!(root.semantic_state().unwrap(), before);
}
#[test]
fn shield_defense_and_pass_workers_keep_cards_and_linked_effects_private() {
    let mut root = NativeSession::compile(source("reaction-effect-shield-plain-shield")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let before = root.semantic_state().unwrap();
    let mut shield = root.clone();
    let mut pass = root.clone();
    let selection = shield.import_selection(&json!("stone_carapace")).unwrap();
    shield.submit(request.id, selection).unwrap();
    let selection = pass.import_selection(&json!("PASS")).unwrap();
    pass.submit(request.id, selection).unwrap();
    // Observe the discard before the passed attack goes on to defeat the
    // hero and correctly expire its card-owned shield effect.
    shield.advance_prefix(2).unwrap();
    pass.advance_prefix(1).unwrap();
    let id = shield.symbols.lookup("shield").unwrap();
    assert!(
        shield
            .world
            .effects
            .instances
            .iter()
            .all(|effect| effect.id != id)
    );
    assert!(
        pass.world
            .effects
            .instances
            .iter()
            .any(|effect| effect.id == id)
    );
    assert_eq!(root.semantic_state().unwrap(), before);
}
#[test]
fn expired_shield_is_rejected_when_its_waiting_reaction_is_submitted() {
    let mut session =
        NativeSession::compile(source("reaction-effect-shield-plain-shield")).unwrap();
    let request = session
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    session.world.effects.instances.remove(0);
    let value = session.import_selection(&json!("stone_carapace")).unwrap();
    session.submit(request.id, value).unwrap();
    let next = session
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert_ne!(request.id, next.id);
    assert!(
        session.request_json(&next)["options"]
            .as_array()
            .unwrap()
            .iter()
            .all(|option| option["id"] != "stone_carapace")
    );
}
#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restore scan needs the verified canonical corpus"
)]
fn effect_targeting_pre_actions_and_projectiles_restore_every_instruction() {
    let base = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
    let index: Value = serde_json::from_reader(std::io::BufReader::new(
        crate::fixture_cache::open(base.join("stack-traces.json")).unwrap(),
    ))
    .unwrap();
    let names = index["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .filter(|name| name.starts_with("targeting-"))
        .collect::<Vec<_>>();
    assert!(!names.is_empty());
    for name in names {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
fn disruptor_requeue_and_consumption_remain_private_while_discard_input_waits() {
    let mut root = NativeSession::compile(source("targeting-pre-discard-multiple-first")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let wire = root.request_json(&request);
    let before = root.semantic_state().unwrap();
    let mut first = root.clone();
    let mut last = NativeSession::compile(root.export_envelope().unwrap()).unwrap();
    let restored_request = last.advance_prefix(1).unwrap().request.unwrap();
    assert_eq!(wire, last.request_json(&restored_request));
    let first_value = first.import_selection(&wire["options"][0]["id"]).unwrap();
    let last_value = last
        .import_selection(&wire["options"].as_array().unwrap().last().unwrap()["id"])
        .unwrap();
    first.submit(request.id, first_value).unwrap();
    last.submit(restored_request.id, last_value).unwrap();
    assert!(
        first
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .is_some()
    );
    assert!(
        last.advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .is_some()
    );
    let hero = root.symbols.lookup("hero_mrak").unwrap();
    assert_ne!(
        first.world.hero(hero).unwrap().hand,
        last.world.hero(hero).unwrap().hand
    );
    assert_eq!(root.semantic_state().unwrap(), before);
    let second = root.symbols.lookup("disrupt-two").unwrap();
    assert!(
        root.world
            .effects
            .instances
            .iter()
            .any(|effect| effect.id == second)
    );
    assert!(
        first
            .world
            .effects
            .instances
            .iter()
            .all(|effect| effect.id != second)
    );
}
#[test]
fn bounty_counters_are_private_and_export_the_decremented_value() {
    let root = NativeSession::compile(source("combat-effect-bounty-twice")).unwrap();
    let before = root.semantic_state().unwrap();
    let mut paid = root.clone();
    let mut exhausted = root.clone();
    exhausted.world.effects.instances[0].remaining_bounty = Some(0);
    paid.advance(&ExecutionBudget::default()).unwrap();
    exhausted.advance(&ExecutionBudget::default()).unwrap();
    let hero = root.symbols.lookup("hero_swift").unwrap();
    assert_eq!(paid.world.hero(hero).unwrap().gold, 2);
    assert_eq!(exhausted.world.hero(hero).unwrap().gold, 0);
    assert_eq!(paid.world.effects.instances[0].remaining_bounty, Some(0));
    assert_eq!(
        paid.semantic_state().unwrap()["active_effects"][0]["max_value"],
        0
    );
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(
        &paid.world.definitions,
        &exhausted.world.definitions
    ));
}
#[test]
fn protection_rechecks_the_live_provider_before_accepting_a_waiting_card() {
    let mut root = NativeSession::compile(source("combat-effect-protection-card-False")).unwrap();
    let req = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let selection = root.request_json(&req)["options"][0]["id"].clone();
    let before = root.semantic_state().unwrap();
    let mut accepted = root.clone();
    let mut expired = root.clone();
    expired.world.effects.instances.remove(0);
    for session in [&mut accepted, &mut expired] {
        let value = session.import_selection(&selection).unwrap();
        session.submit(req.id, value).unwrap();
        session.advance(&ExecutionBudget::default()).unwrap();
    }
    let minion = root
        .symbols
        .lookup(
            before["execution_stack"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["minion_id"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
    assert!(accepted.world.position(minion).is_some());
    assert!(expired.world.position(minion).is_none());
    assert_eq!(root.semantic_state().unwrap(), before);
}
#[test]
fn forced_filtered_shield_selection_keeps_its_original_allowed_hand_snapshot() {
    let mut session = NativeSession::compile(source("discard-effect-non-basic-last")).unwrap();
    session.advance_prefix(1).unwrap();
    let hero = session.symbols.lookup("hero_mrak").unwrap();
    let index = session.world.hero_index(hero).unwrap();
    let before = session.world.heroes[index].hand.clone();
    let extra = session.world.heroes[index]
        .deck
        .iter()
        .copied()
        .find(|id| {
            !before.contains(id)
                && session.world.card(*id).unwrap().lifecycle == crate::world::CardLifecycle::Deck
                && !session.world.definitions.cards[session.world.card(*id).unwrap().definition]
                    .basic
        })
        .unwrap();
    let printed =
        session.world.definitions.cards[session.world.card(extra).unwrap().definition].printed_id;
    session.world.heroes[index].hand.push(extra);
    let req = session.advance_prefix(1).unwrap().request.unwrap();
    let wire = session.request_json(&req);
    assert!(
        wire["options"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["id"] != session.symbols.name(printed))
    );
    let mut restored = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
    let restored_req = restored.advance_prefix(1).unwrap().request.unwrap();
    assert_eq!(wire, restored.request_json(&restored_req));
}
#[test]
fn blast_discard_dynamic_effect_closure_remains_explicit() {
    let mut active = source("mine-trigger-dud-effects");
    let step = active.state["execution_stack"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|s| s["type"] == "trigger_mine")
        .unwrap();
    step["mine_ids"] = json!(["mine_b"]);
    assert!(NativeSession::compile(active).is_ok());
    let mut after_create = source("mine-trigger-blast");
    let created = source("effect-step-plain");
    let create = created.state["execution_stack"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["type"] == "create_effect")
        .unwrap()
        .clone();
    after_create.state["execution_stack"]
        .as_array_mut()
        .unwrap()
        .push(create);
    assert!(
        matches!(NativeSession::compile(after_create),Err(EngineError::Unsupported(message)) if message.contains("blast mine dynamically-created effect"))
    );
}
#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restoration matrix; requires complete named case family"
)]
fn effect_movement_sequences_restore_every_instruction() {
    for variant in [
        "base-plain",
        "base-blocked",
        "base-bound-piece",
        "base-topology-split",
        "base-actor-immunity",
        "barrier",
        "barrier-outside",
        "empty",
        "map-gap",
        "restriction-cap-negative",
        "restriction-actions-only",
        "aura-active",
        "aura-inactive",
        "aura-no-grant",
        "aura-outside",
        "aura-immune",
        "aura-bound-piece",
    ] {
        for mode in ["normal", "straight", "full"] {
            let name = format!("movement-sequence-{variant}-{mode}");
            let root = NativeSession::compile(source(&name)).unwrap();
            assert_eq!(
                run_instruction_restores(root.clone(), true),
                run_instruction_restores(root, false),
                "{name}"
            );
        }
    }
}
#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restoration matrix; requires complete named case family"
)]
fn controlled_prompts_restore_original_actor_metadata_and_request_identity() {
    for variant in [
        "active",
        "inactive-cardless",
        "expired-active",
        "future-active",
        "wrong-card",
        "wrong-origin",
        "no-owner",
        "other-owner",
        "other-actor",
        "different-performed-card",
        "first-controller",
        "source-offboard",
        "unknown-controller",
        "self-controller",
        "no-current-card",
        "bound-owner",
    ] {
        for kind in ["number", "discard-number", "hex", "confirm"] {
            let name = format!("controlled-{variant}-{kind}");
            let root = NativeSession::compile(source(&name)).unwrap();
            assert_eq!(
                run_instruction_restores(root.clone(), true),
                run_instruction_restores(root, false),
                "{name}"
            );
        }
    }
}
#[test]
fn movement_zone_permission_is_frozen_at_expansion_and_survives_restore() {
    let mut root = NativeSession::compile(source("movement-sequence-aura-active-normal")).unwrap();
    let actor = root.symbols.lookup("hero_brogan").unwrap();
    let origin = root.symbols.lookup("hero_arien").unwrap();
    assert!(root.world.movement_zone_passes(actor).unwrap());
    root.advance_prefix(1).unwrap();
    root.world
        .move_entity(origin, crate::geometry::Hex::new(-3, 0, 3).unwrap())
        .unwrap();
    assert!(!root.world.movement_zone_passes(actor).unwrap());
    let envelope = root.export_envelope().unwrap();
    let frames = envelope.state["execution_stack"].as_array().unwrap();
    let movement = frames.iter().find(|s| s["type"] == "move_unit").unwrap();
    assert_eq!(movement["pass_through_obstacles"], true);
    assert_eq!(movement["range_val"], 3);
    let restored = NativeSession::compile(envelope).unwrap();
    assert_eq!(
        run_instruction_restores(root, false),
        run_instruction_restores(restored, true)
    );
}
#[test]
fn controlled_decision_workers_keep_controller_and_actor_separate() {
    let root = NativeSession::compile(source("controlled-active-discard-number")).unwrap();
    let before = root.semantic_state().unwrap();
    let mut controlled = root.clone();
    let mut own = root.clone();
    own.planning.owner = own.symbols.lookup("hero_brogan");
    let request = controlled
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let original = own
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert_eq!(controlled.symbols.name(request.owner), "hero_hanu");
    assert_eq!(own.symbols.name(original.owner), "hero_arien");
    let wire = controlled.request_json(&request);
    assert_eq!(wire["context"]["controlled_hero_id"], "hero_arien");
    assert_eq!(wire["decision_context"]["actor_id"], "hero_arien");
    assert_eq!(wire["options"], own.request_json(&original)["options"]);
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(
        &controlled.world.definitions,
        &own.world.definitions
    ));
    let mut restored = NativeSession::compile(controlled.export_envelope().unwrap()).unwrap();
    let again = restored
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert_eq!(restored.request_json(&again), wire);
}
#[test]
fn effect_fast_travel_filter_compiles_and_keeps_stack_order() {
    let mut envelope = source("movement-sequence-base-plain-normal");
    let filter = crate::templates::instructions().unwrap()["templates"]["travel_filter"].clone();
    envelope.state["execution_stack"] = json!([
        {"type":"select_step","target_type":"HEX","prompt":"Unported travel","output_key":"choice","filters":[filter],"step_id":"fixture/unsafe-effect-travel"},
        {"type":"set_context_flag","key":"before","value":true,"step_id":"fixture/unsafe-before"}
    ]);
    let mut session = NativeSession::compile(envelope).unwrap();
    session.advance_prefix(1).unwrap();
    assert_eq!(
        session.semantic_state().unwrap()["execution_context"]["before"],
        true
    );
    session.advance_prefix(1).unwrap();
}
#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restoration matrix; requires complete named case family"
)]
fn effect_aware_movement_restores_at_every_instruction() {
    for name in [
        "base-plain",
        "base-blocked",
        "base-offboard",
        "base-actor-immunity",
        "base-type-topology_split",
        "base-type-topology_isolation",
        "barrier",
        "barrier-outside",
        "barrier-piece-origin",
        "barrier-inactive",
        "empty",
        "empty-scope-ignored",
        "restriction-cap-negative",
        "restriction-cap-none",
        "restriction-actions-only",
        "petrify",
        "map-gap",
    ] {
        for mode in ["mandatory", "optional", "action", "straight"] {
            let name = format!("terrain-move-{name}-{mode}");
            let root = NativeSession::compile(source(&name)).unwrap();
            assert_eq!(
                run_instruction_restores(root.clone(), true),
                run_instruction_restores(root, false),
                "{name}"
            );
        }
    }
}
#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restoration matrix; requires complete named case family"
)]
fn active_repeat_validation_restores_at_every_instruction() {
    for variant in [
        "blocked",
        "plain",
        "offboard",
        "bound-piece",
        "source-token",
        "actor-immunity",
        "attack-shield",
        "duration-NEXT_TURN-current",
        "prerequisite-order",
        "bound-owner",
        "one-piece-free",
        "both-pieces-blocked",
        "no-pieces",
        "cap-negative",
        "scope-zone-all_units",
    ] {
        let name = format!("action-rule-repeat-{variant}");
        let root = NativeSession::compile(source(&name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
fn repeat_workers_keep_counts_and_nested_copies_private() {
    let mut root = NativeSession::compile(source("repeat-nested")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let before = root.semantic_state().unwrap();
    let mut accepted = root.clone();
    let mut declined = root.clone();
    assert!(Arc::ptr_eq(&accepted.steps, &declined.steps));
    accepted
        .submit(request.id, Datum::Text(accepted.yes))
        .unwrap();
    let no = declined.import_selection(&json!("NO")).unwrap();
    declined.submit(request.id, no).unwrap();
    assert!(
        declined
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .is_none()
    );
    for _ in 0..10 {
        let next = accepted.advance(&ExecutionBudget::default()).unwrap();
        if let Some(request) = next.request {
            accepted
                .submit(request.id, Datum::Text(accepted.yes))
                .unwrap();
        }
        if accepted.stack.is_empty() {
            break;
        }
    }
    assert!(accepted.stack.is_empty());
    let key = root.symbols.lookup("repeat_count").unwrap();
    assert_eq!(accepted.context.get(key), Some(&Datum::Int(2)));
    assert_eq!(declined.context.get(key), Some(&Datum::Int(0)));
    assert_eq!(root.semantic_state().unwrap(), before);
}
#[test]
fn repeat_validation_guards_aggregate_aliases() {
    let root = NativeSession::compile(source("repeat-validation-true")).unwrap();
    let before = root.semantic_state().unwrap();
    let mut worker = root.clone();
    let key = worker.symbols.lookup("previous").unwrap();
    worker.context.set(key, Datum::List(vec![Datum::Int(1)]));
    assert!(
        matches!(worker.advance(&ExecutionBudget::default()),Err(EngineError::Unsupported(message)) if message.contains("aggregate alias"))
    );
    assert_eq!(root.semantic_state().unwrap(), before);
    let mut worker = root.clone();
    worker.world.has_effects = true;
    assert!(!worker.world.can_repeat_action(None).unwrap());
    assert!(
        worker
            .world
            .can_repeat_action(worker.planning.actor)
            .unwrap()
    );
}

#[test]
fn conditional_card_queries_restore_at_every_instruction() {
    for name in [
        "conditional-primary-quickdraw-attack-current",
        "conditional-primary-quickdraw-attack-resolved",
        "conditional-primary-quickdraw-piece-attack",
        "conditional-primary-gunslinger-attack-current",
        "conditional-primary-gunslinger-attack-resolved",
        "conditional-primary-gunslinger-piece-attack",
        "conditional-primary-dead_shot-attack-current",
        "conditional-primary-dead_shot-attack-resolved",
        "conditional-primary-dead_shot-piece-attack",
        "conditional-primary-shield_bash-attack-current",
        "conditional-primary-shield_bash-attack-resolved",
        "conditional-primary-shield_bash-piece-attack",
        "conditional-primary-counterattack-attack-current",
        "conditional-primary-counterattack-attack-resolved",
        "conditional-primary-counterattack-piece-attack",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
fn effect_creation_and_frozen_payloads_restore_at_each_instruction() {
    for name in [
        "effect-step-prefix",
        "effect-step-freeze-colors",
        "effect-step-duplicate",
        "effect-step-duplicate-dormant",
        "effect-step-token-piece",
        "effect-step-nested-color",
        "effect-step-id-collision",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
fn created_effects_do_not_enable_unported_gameplay_or_controller_requests() {
    let mut envelope = source("effect-step-plain");
    let mut root = NativeSession::compile(envelope.clone()).unwrap();
    root.advance(&ExecutionBudget::default()).unwrap();
    let before = root.semantic_state().unwrap();
    envelope = root.export_envelope().unwrap();
    envelope.state["execution_stack"] = json!([{"type":"select","target_type":"NUMBER","options":[1],"output_key":"answer","prompt":"Choose","step_id":"fixture/effect-unsafe-input"}]);
    assert!(
        matches!(NativeSession::compile(envelope),Err(EngineError::Unsupported(message)) if message.contains("active-effect"))
    );
    assert_eq!(root.semantic_state().unwrap(), before);
}
#[test]
fn effect_step_workers_freeze_different_colors_without_mutating_templates() {
    let root = NativeSession::compile(source("effect-step-freeze-colors")).unwrap();
    let before = root.semantic_state().unwrap();
    let mut a = root.clone();
    let mut b = root.clone();
    let key = b.symbols.lookup("color").unwrap();
    let red = b.symbols.lookup("RED").unwrap();
    b.context.set(key, Datum::Text(red));
    a.advance_prefix(1).unwrap();
    b.advance_prefix(1).unwrap();
    assert!(Arc::ptr_eq(&a.steps, &b.steps));
    assert_ne!(
        a.world.effects.export(&a.symbols),
        b.world.effects.export(&b.symbols)
    );
    assert_eq!(root.semantic_state().unwrap(), before);
}
#[test]
fn ordinary_retrieval_restores_at_each_instruction() {
    for name in [
        "retrieval-current",
        "retrieval-played",
        "retrieval-all",
        "retrieval-hero-key-piece",
        "retrieval-global-shadow",
        "retrieval-played-discard-alias",
        "discard-expiry-global-shadow",
        "discard-expiry-enraged",
        "retrieval-primary-steady_advance-ordinary",
        "retrieval-primary-unwavering_resolve-ordinary",
        "retrieval-primary-devoted_followers-ordinary",
        "retrieval-primary-fresh_converts-ordinary",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
fn ordinary_retrieval_workers_have_private_collections_and_expiry() {
    let root = NativeSession::compile(source("retrieval-all")).unwrap();
    let before = root.semantic_state().unwrap();
    let actor = root.planning.actor.unwrap();
    let cards = root.world.hero(actor).unwrap().discard.clone();
    assert_eq!(cards.len(), 2);
    let mut a = root.clone();
    let mut b = root.clone();
    let printed = b.world.definitions.cards[b.world.card(cards[0]).unwrap().definition].printed_id;
    let events = b.world.retrieve_card(actor, Some(printed), false).unwrap();
    assert_eq!(events.len(), 1);
    a.advance(&ExecutionBudget::default()).unwrap();
    assert!(a.world.hero(actor).unwrap().discard.is_empty());
    assert_eq!(b.world.hero(actor).unwrap().discard, vec![cards[1]]);
    assert!(!a.world.card(cards[1]).unwrap().active);
    assert!(b.world.card(cards[1]).unwrap().active);
    assert!(Arc::ptr_eq(&a.steps, &b.steps));
    assert!(Arc::ptr_eq(&a.world.definitions, &b.world.definitions));
    assert_eq!(root.semantic_state().unwrap(), before);
}
#[test]
fn retrieval_rejects_unported_expiry_and_partial_failure_before_mutation() {
    let root = NativeSession::compile(source("retrieval-all")).unwrap();
    let actor = root.planning.actor.unwrap();
    let mut worker = root.clone();
    worker.world.has_effects = true;
    let before = worker.semantic_state().unwrap();
    assert!(matches!(
        worker.world.retrieve_card(actor, None, true),
        Err(crate::cards::CardError::Unsupported(_))
    ));
    assert_eq!(worker.semantic_state().unwrap(), before);
    let mut worker = root.clone();
    let card = worker.world.hero(actor).unwrap().discard[0];
    worker.world.hero_row_mut(actor).unwrap().discard.push(card);
    let before = worker.semantic_state().unwrap();
    assert!(matches!(
        worker.world.retrieve_card(actor, None, true),
        Err(crate::cards::CardError::Unsupported(_))
    ));
    assert_eq!(worker.semantic_state().unwrap(), before);
}

#[test]
fn queries_restore_order_and_scalar_results_at_instruction_boundaries() {
    for name in [
        "query-collect-order",
        "query-played-current",
        "query-immunity-on",
        "query-hex-range",
        "query-piece-owner",
        "query-collect-immune-token",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
fn query_workers_read_private_live_cards_slots_and_occupancy() {
    for name in ["query-played-current", "query-played-resolved"] {
        let root = NativeSession::compile(source(name)).unwrap();
        let before = root.semantic_state().unwrap();
        let mut a = root.clone();
        let mut b = root.clone();
        assert!(Arc::ptr_eq(&a.steps, &b.steps));
        assert!(Arc::ptr_eq(&a.world.definitions, &b.world.definitions));
        if name.ends_with("current") {
            let hero = b.symbols.lookup("hero_brogan").unwrap();
            let card = b.world.hero(hero).unwrap().current.unwrap();
            b.world.card_mut(card).unwrap().facedown = true;
        } else {
            b.world.combat.turn = 2;
        }
        a.advance(&ExecutionBudget::default()).unwrap();
        b.advance(&ExecutionBudget::default()).unwrap();
        let output = root.symbols.lookup("result").unwrap();
        assert_eq!(a.context.get(output), Some(&Datum::Int(1)));
        assert_eq!(b.context.get(output), Some(&Datum::Int(0)));
        assert_eq!(root.semantic_state().unwrap(), before);
    }
    let root = NativeSession::compile(source("query-collect")).unwrap();
    let before = root.semantic_state().unwrap();
    let mut a = root.clone();
    let mut b = root.clone();
    let hero = b.symbols.lookup("hero_arien").unwrap();
    b.world.remove_entity(hero).unwrap();
    a.advance(&ExecutionBudget::default()).unwrap();
    b.advance(&ExecutionBudget::default()).unwrap();
    let output = root.symbols.lookup("result").unwrap();
    let (Some(Datum::List(all)), Some(Datum::List(less))) =
        (a.context.get(output), b.context.get(output))
    else {
        panic!("expected collected IDs")
    };
    assert_eq!(all.len(), less.len() + 1);
    assert_eq!(root.semantic_state().unwrap(), before);
}
#[test]
fn query_boundaries_reject_invalid_flags_filters_and_nonfinite_comparisons() {
    for field in ["skip_self_filter", "skip_immunity_filter"] {
        let mut invalid = source("query-all");
        invalid.state["execution_stack"][0][field] = json!([]);
        assert!(matches!(
            NativeSession::compile(invalid),
            Err(EngineError::Unsupported(_))
        ));
    }
    for (field, value) in [
        ("action_type", "UNKNOWN_ACTION"),
        ("card_color", "UNKNOWN_COLOR"),
    ] {
        let mut invalid = source("query-played-current");
        invalid.state["execution_stack"][0]["filters"][0][field] = json!(value);
        assert!(matches!(
            NativeSession::compile(invalid),
            Err(EngineError::Unsupported(_))
        ));
    }
    assert!(
        crate::queries::Comparison::Ge
            .evaluate(Some(&Datum::Float(f64::NAN)), 0)
            .unwrap_err()
            .contains("nonfinite")
    );
}

#[test]
fn conditional_stat_hook_exposes_one_eligible_push_passive() {
    let root = NativeSession::compile(source("aura-conditional-changing-context-trace")).unwrap();
    let hero = root.symbols.lookup("hero_sabina").unwrap();
    let trigger = root.symbols.lookup("after_push").unwrap();
    let before = root.semantic_state().unwrap();
    assert!(!root.world.has_uncompiled_auras);
    assert!(
        root.passive_rules
            .scan(&root.world, Some(hero), None, trigger)
            .unwrap()
            .len()
            == 1
    );
    assert_eq!(root.semantic_state().unwrap(), before);
}

#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restore scan needs the verified canonical corpus"
)]
fn effect_aware_cleanup_upgrades_and_reset_restore_every_instruction() {
    let base = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
    let index: Value = serde_json::from_reader(std::io::BufReader::new(
        crate::fixture_cache::open(base.join("stack-traces.json")).unwrap(),
    ))
    .unwrap();
    let names = index["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .filter(|name| name.starts_with("round-effect-"))
        .collect::<Vec<_>>();
    assert!(
        names.len() >= 216,
        "retain durations, links, tokens, upgrades and maps"
    );
    for name in names {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_round_instruction_restores(root.clone(), true),
            run_round_instruction_restores(root, false),
            "{name}"
        );
    }
}

#[test]
fn round_boundary_planning_commands_require_complete_effect_and_continuation_closures() {
    for (name, persistent) in [
        ("round-effect-regression-round-cleanup-ordinary", true),
        ("round-effect-cleanup-THIS_ROUND-True-None", false),
    ] {
        let mut root = NativeSession::compile(source(name)).unwrap();
        root.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(root.planning.phase, Phase::Planning);
        assert_eq!(root.world.has_effects, persistent);
        let before = root.semantic_state().unwrap();
        let hero = root.world.heroes[0].id;
        let card = root.world.heroes[0].hand[0];
        for command in 0..4 {
            let result = match command {
                0 => root.commit_card(hero, card),
                1 => root.uncommit_card(hero),
                2 => root.pass_planning(hero),
                _ => root.finish_planning(hero),
            };
            assert!(matches!(result, Err(EngineError::Unsupported(_))));
            assert_eq!(root.semantic_state().unwrap(), before);
        }
        let mut restored = NativeSession::compile(root.export_envelope().unwrap()).unwrap();
        assert_eq!(restored.semantic_state().unwrap(), before);
        let hero = restored.world.heroes[0].id;
        let card = restored.world.heroes[0].hand[0];
        if persistent {
            assert!(matches!(
                restored.commit_card(hero, card),
                Err(EngineError::Unsupported(_))
            ));
            assert_eq!(restored.semantic_state().unwrap(), before);
        } else {
            // A fresh boundary with no effects compiles full ordinary flow.
            restored.commit_card(hero, card).unwrap();
        }
    }
}

#[test]
fn ordinary_round_cleanup_advances_into_planning() {
    let mut root = NativeSession::compile(source("turn-advance-round-end")).unwrap();
    let result = root.advance(&ExecutionBudget::default()).unwrap();
    assert!(result.request.is_none());
    let state = root.semantic_state().unwrap();
    assert_eq!(state["round"], 2);
    assert_eq!(root.world.combat.round, 2);
    assert_eq!(state["turn"], 1);
    assert_eq!(state["phase"], "PLANNING");
}

fn upgrade_answer(session: &NativeSession, request: &NativeRequest, side: usize) -> Datum {
    let RequestChoices::Upgrades(players) = &request.choices else {
        panic!("upgrade prompt expected");
    };
    let player = players
        .iter()
        .find(|p| p.remaining > 0 && !p.options.is_empty())
        .unwrap();
    let id = player.options[0].pair[side];
    let printed =
        session.world.definitions.cards[session.world.card(id).unwrap().definition].printed_id;
    Datum::Object(vec![
        (
            session.symbols.lookup("hero_id").unwrap(),
            Datum::Text(player.hero),
        ),
        (
            session.symbols.lookup("card_id").unwrap(),
            Datum::Text(printed),
        ),
    ])
}

fn run_round_instruction_restores(
    mut session: NativeSession,
    restore: bool,
) -> (Value, Vec<Value>) {
    let mut events = Vec::new();
    for _ in 0..100 {
        let result = session.advance_prefix(1).unwrap();
        events.extend(result.events.iter().map(|e| session.event_json(e)));
        let answer = result.request.as_ref().and_then(|r| {
            let RequestChoices::Upgrades(players) = &r.choices else {
                panic!("upgrade prompt expected");
            };
            players
                .iter()
                .any(|p| p.remaining > 0 && !p.options.is_empty())
                .then(|| upgrade_answer(&session, r, 0).to_json(&session.symbols))
        });
        let paused = result.request.is_some() && answer.is_none();
        if restore {
            session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
        }
        if paused {
            let request = if restore {
                session.advance_prefix(1).unwrap().request.unwrap()
            } else {
                result.request.unwrap()
            };
            return (
                json!({"state": session.semantic_state().unwrap(),
                    "request": session.request_json(&request)}),
                events,
            );
        }
        if let Some(value) = answer {
            let request = if restore {
                session.advance_prefix(1).unwrap().request.unwrap()
            } else {
                result.request.unwrap()
            };
            // Symbol IDs are private to each compiled session.
            let value = session.import_selection(&value).unwrap();
            session.submit(request.id, value).unwrap();
            if restore {
                session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
            }
        }
        if session.stack.is_empty() {
            return (session.semantic_state().unwrap(), events);
        }
    }
    panic!("round did not finish");
}

#[test]
fn round_cleanup_upgrades_and_ultimate_unlocks_restore_at_every_instruction() {
    for name in [
        "round-cleanup-played-discard-current",
        "round-cleanup-markers",
        "round-token-duplicate-pool",
        "round-gold-28-batch-True",
        "round-cleanup-sparse-items",
        "round-ultimate-Ursafar-unlock-six",
        "round-ultimate-Takahide-unlock-six",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_round_instruction_restores(root.clone(), true),
            run_round_instruction_restores(root, false),
            "{name}"
        );
    }
}

#[test]
fn invalid_upgrade_batch_is_atomic_and_can_be_retried() {
    let mut root = NativeSession::compile(source("round-gold-3-batch-False")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let RequestChoices::Upgrades(players) = &request.choices else {
        panic!("upgrade");
    };
    let p = &players[0];
    assert_eq!(p.remaining, 2);
    let printed =
        |id| root.world.definitions.cards[root.world.card(id).unwrap().definition].printed_id;
    let make = |cards: Vec<Symbol>| {
        Datum::Object(vec![
            (root.symbols.lookup("hero_id").unwrap(), Datum::Text(p.hero)),
            (
                root.symbols.lookup("card_ids").unwrap(),
                Datum::List(cards.into_iter().map(Datum::Text).collect()),
            ),
        ])
    };
    let invalid = make(p.options[0].pair.iter().copied().map(printed).collect());
    let valid = make(vec![
        printed(p.options[0].pair[0]),
        printed(p.options[1].pair[0]),
    ]);
    let before = root.semantic_state().unwrap();
    let wire = root.request_json(&request);
    assert!(matches!(
        root.submit(request.id, invalid),
        Err(EngineError::InvalidInput(_))
    ));
    assert_eq!(root.semantic_state().unwrap(), before);
    assert_eq!(root.request_json(root.last_request.as_ref().unwrap()), wire);
    root.submit(request.id, valid).unwrap();
    assert!(
        root.advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .is_none()
    );
    let state = root.semantic_state().unwrap();
    assert_eq!(state["round"], 2);
    assert_eq!(state["phase"], "PLANNING");
    assert_eq!(state["pending_upgrades"], json!({}));
    assert_eq!(state["upgrade_reveal_snapshot"], json!({}));
}

#[test]
fn upgrade_workers_share_rules_but_keep_cards_items_and_reveal_state_private() {
    for name in [
        "round-gold-3-batch-False",
        "round-effect-regression-round-gold-3-batch-False",
    ] {
        let mut root = NativeSession::compile(source(name)).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let before = root.semantic_state().unwrap();
        let mut left = root.clone();
        let mut right = root.clone();
        let a = upgrade_answer(&left, &request, 0);
        let b = upgrade_answer(&right, &request, 1);
        left.submit(request.id, a).unwrap();
        right.submit(request.id, b).unwrap();
        left.advance(&ExecutionBudget::default()).unwrap();
        right.advance(&ExecutionBudget::default()).unwrap();
        assert_ne!(left.world.heroes[0].hand, right.world.heroes[0].hand);
        assert_ne!(left.world.heroes[0].items, right.world.heroes[0].items);
        assert_eq!(root.semantic_state().unwrap(), before);
        assert!(Arc::ptr_eq(
            left.round_rules.as_ref().unwrap(),
            right.round_rules.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(
            left.flow_program.as_ref().unwrap(),
            right.flow_program.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(
            &left.world.definitions,
            &right.world.definitions
        ));
    }
}

#[test]
fn planning_command_branches_restore_without_sharing_card_commits() {
    let mut root = NativeSession::compile(source("game-double")).unwrap();
    let hero = root.symbols.lookup("hero_emmitt").unwrap();
    let initial_hand = root.world.hero(hero).unwrap().hand.len();
    let card = root.world.hero(hero).unwrap().hand[0];
    root.commit_card(hero, card).unwrap();
    let before = root.semantic_state().unwrap();
    let mut left = root.clone();
    let mut right = NativeSession::compile(root.export_envelope().unwrap()).unwrap();
    let right_hero = right.symbols.lookup("hero_emmitt").unwrap();
    let second = left.world.hero(hero).unwrap().hand[0];
    left.commit_card(hero, second).unwrap();
    right.finish_planning(right_hero).unwrap();
    assert_eq!(root.semantic_state().unwrap(), before);
    assert_ne!(
        left.semantic_state().unwrap()["pending_second_cards"],
        right.semantic_state().unwrap()["pending_second_cards"]
    );
    left = NativeSession::compile(left.export_envelope().unwrap()).unwrap();
    right = NativeSession::compile(right.export_envelope().unwrap()).unwrap();
    let left_hero = left.symbols.lookup("hero_emmitt").unwrap();
    let right_hero = right.symbols.lookup("hero_emmitt").unwrap();
    left.uncommit_card(left_hero).unwrap();
    right.uncommit_card(right_hero).unwrap();
    // Undo removes the second commit before the first, matching the phase API.
    assert_eq!(
        left.world.hero(left_hero).unwrap().hand.len(),
        initial_hand - 1
    );
    assert_eq!(
        right.world.hero(right_hero).unwrap().hand.len(),
        initial_hand
    );
    left.uncommit_card(left_hero).unwrap();
    assert_eq!(left.world.hero(left_hero).unwrap().hand.len(), initial_hand);
    for session in [&left, &right] {
        let h = session
            .world
            .hero(session.symbols.lookup("hero_emmitt").unwrap())
            .unwrap();
        assert!(h.hand.iter().all(|id| h.deck.contains(id)));
    }
}

#[test]
fn battle_restores_every_instruction_and_team_prompt() {
    fn run(mut session: NativeSession, restore: bool) -> (Value, Vec<Value>) {
        let mut events = Vec::new();
        for _ in 0..150 {
            let result = session.advance_prefix(1).unwrap();
            events.extend(result.events.iter().map(|e| session.event_json(e)));
            let answer = result.request.as_ref().map(|req| {
                let wire = session.request_json(req);
                let option = &wire["options"][0];
                (
                    session.request_token(req.id),
                    option["metadata"]
                        .get("hex")
                        .unwrap_or(&option["id"])
                        .clone(),
                )
            });
            if restore {
                session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
            }
            if let Some((token, value)) = answer {
                let request = if restore {
                    session.advance_prefix(1).unwrap().request.unwrap()
                } else {
                    result.request.unwrap()
                };
                assert_eq!(session.request_token(request.id), token);
                let selected = session.import_selection(&value).unwrap();
                session.submit(request.id, selected).unwrap();
            }
            if session.stack.is_empty() {
                return (session.semantic_state().unwrap(), events);
            }
        }
        panic!("battle did not terminate");
    }
    for name in [
        "battle-spawn-blocked-two",
        "battle-displace-same-origin",
        "battle-casualties-force-all",
        "battle-casualties-choice",
        "battle-multi-both-thrones",
        "battle-multi-last-red",
        "battle-red-push",
        "battle-minions-army",
        "battle-minions-all-lanes",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        let expected = run(root.clone(), false);
        let actual = run(root, true);
        assert_eq!(actual, expected, "{name}");
    }
}
#[test]
fn battle_workers_keep_waves_zones_and_victory_private() {
    let root = NativeSession::compile(source("battle-red-push")).unwrap();
    let before = root.semantic_state().unwrap();
    let mut final_wave = root.clone();
    let mut ordinary = root.clone();
    final_wave.world.waves[0].1 = 1;
    final_wave.advance(&ExecutionBudget::default()).unwrap();
    ordinary.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(final_wave.planning.phase, Phase::GameOver);
    assert_eq!(ordinary.planning.phase, Phase::Resolution);
    assert_eq!(final_wave.world.waves[0].1, 0);
    assert_eq!(ordinary.world.waves[0].1, 4);
    assert_ne!(final_wave.world.battle_zones, ordinary.world.battle_zones);
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(
        root.battle_rules.as_ref().unwrap(),
        ordinary.battle_rules.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        root.battle_program.as_ref().unwrap(),
        final_wave.battle_program.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(
        &ordinary.world.definitions,
        &final_wave.world.definitions
    ));
}
#[test]
fn simultaneous_pushes_freeze_targets_and_flip_waves_once() {
    let mut root = NativeSession::compile(source("battle-multi-same-direction")).unwrap();
    root.advance_prefix(1).unwrap();
    let state = root.semantic_state().unwrap();
    let pushes = state["execution_stack"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["type"] == "lane_push")
        .collect::<Vec<_>>();
    assert_eq!(pushes.len(), 2);
    for push in pushes {
        assert_eq!(push["skip_wave_counter"], true);
        assert!(push["target_zone_id"].as_str().unwrap().ends_with("-3"));
    }
    assert!(root.world.waves.iter().all(|(_, n)| *n == 4));
    let mut restored = NativeSession::compile(root.export_envelope().unwrap()).unwrap();
    root.advance(&ExecutionBudget::default()).unwrap();
    restored.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(
        root.semantic_state().unwrap(),
        restored.semantic_state().unwrap()
    );
    assert!(root.world.waves.iter().all(|(_, n)| *n == 4));
}

#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restore scan needs the verified canonical corpus"
)]
fn effect_aware_battles_restore_every_instruction() {
    let base = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
    let index: Value = serde_json::from_reader(std::io::BufReader::new(
        crate::fixture_cache::open(base.join("stack-traces.json")).unwrap(),
    ))
    .unwrap();
    let names = index["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .filter(|name| name.starts_with("battle-effect-"))
        .collect::<Vec<_>>();
    assert!(
        names.len() >= 322,
        "retain exclusion, push and displacement cases"
    );
    for name in names {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}

#[test]
fn effect_aware_battle_casualty_choices_are_private_to_workers() {
    let mut root = NativeSession::compile(source("battle-effect-exclusion-ordinary")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let RequestChoices::Units { ref values, .. } = request.choices else {
        panic!("casualty choice expected");
    };
    assert!(values.len() > 1);
    let before = root.semantic_state().unwrap();
    let mut a = root.clone();
    let mut b = root.clone();
    a.submit(request.id, Datum::Text(values[0].0)).unwrap();
    b.submit(request.id, Datum::Text(values.last().unwrap().0))
        .unwrap();
    a.advance(&ExecutionBudget::default()).unwrap();
    b.advance(&ExecutionBudget::default()).unwrap();
    assert_ne!(
        a.semantic_state().unwrap()["entity_locations"],
        b.semantic_state().unwrap()["entity_locations"]
    );
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(
        a.battle_rules.as_ref().unwrap(),
        b.battle_rules.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(&a.world.definitions, &b.world.definitions));
}

#[test]
fn minion_return_restores_every_generated_boundary() {
    for name in [
        "return-prefix",
        "return-prefix-fallback",
        "return-two-teams",
        "return-processed-first",
        "return-blocked-then-legal",
    ] {
        let mut root = NativeSession::compile(source(name)).unwrap();
        let mut expected = root.clone();
        let mut expected_events = Vec::new();
        loop {
            let result = expected.advance_prefix(1).unwrap();
            expected_events.extend(result.events.iter().map(|e| expected.event_json(e)));
            if let Some(req) = result.request {
                let RequestChoices::Hexes(hexes) = req.choices else {
                    panic!("return destination");
                };
                expected
                    .submit(req.id, native_hex(hexes[0], expected.coordinates))
                    .unwrap();
            }
            if expected.stack.is_empty() {
                break;
            }
        }
        let mut actual_events = Vec::new();
        for _ in 0..30 {
            let result = root.advance_prefix(1).unwrap();
            actual_events.extend(result.events.iter().map(|e| root.event_json(e)));
            root = NativeSession::compile(root.export_envelope().unwrap()).unwrap();
            if let Some(req) = result.request {
                // Regenerate the imported request before accepting its saved token.
                let resumed = root
                    .advance(&ExecutionBudget::default())
                    .unwrap()
                    .request
                    .unwrap();
                let RequestChoices::Hexes(hexes) = resumed.choices else {
                    panic!("return destination");
                };
                assert_eq!(root.request_token(resumed.id), root.request_token(req.id));
                root.submit(resumed.id, native_hex(hexes[0], root.coordinates))
                    .unwrap();
            }
            if root.stack.is_empty() {
                break;
            }
        }
        assert!(root.stack.is_empty(), "{name}");
        assert_eq!(actual_events, expected_events, "{name}");
        assert_eq!(
            root.semantic_state().unwrap(),
            expected.semantic_state().unwrap(),
            "{name}"
        );
    }
}

#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restore scan needs the verified canonical corpus"
)]
fn effect_aware_minion_returns_restore_every_instruction() {
    let base = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
    let index: Value = serde_json::from_reader(std::io::BufReader::new(
        crate::fixture_cache::open(base.join("stack-traces.json")).unwrap(),
    ))
    .unwrap();
    let names = index["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["name"].as_str())
        .filter(|name| name.starts_with("return-effect-"))
        .collect::<Vec<_>>();
    assert!(
        names.len() >= 234,
        "effect return corpus must retain terrain and cap coverage"
    );
    for name in names {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}

#[test]
fn minion_return_choices_are_private_to_workers() {
    for name in ["return-choice", "return-effect-cap-0-hero_arien-False"] {
        let mut root = NativeSession::compile(source(name)).unwrap();
        let req = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let RequestChoices::Hexes(ref hexes) = req.choices else {
            panic!("return destination");
        };
        assert!(hexes.len() > 1);
        let before = root.semantic_state().unwrap();
        let mut a = root.clone();
        let mut b = root.clone();
        a.submit(req.id, native_hex(hexes[0], a.coordinates))
            .unwrap();
        b.submit(req.id, native_hex(*hexes.last().unwrap(), b.coordinates))
            .unwrap();
        a.advance(&ExecutionBudget::default()).unwrap();
        b.advance(&ExecutionBudget::default()).unwrap();
        assert_ne!(
            a.semantic_state().unwrap()["entity_locations"],
            b.semantic_state().unwrap()["entity_locations"]
        );
        assert_eq!(root.semantic_state().unwrap(), before);
        assert!(Arc::ptr_eq(&a.steps, &b.steps));
        assert!(Arc::ptr_eq(&a.return_rules, &b.return_rules));
        assert!(Arc::ptr_eq(&a.world.definitions, &b.world.definitions));
    }
}

#[test]
fn minion_return_rejects_uncompiled_aura_restrictions_before_relocation() {
    let mut root = NativeSession::compile(source("return-walk")).unwrap();
    root.world.has_uncompiled_auras = true;
    let locations = root.world.locations.clone();
    assert!(matches!(
        root.advance(&ExecutionBudget::default()),
        Err(EngineError::Unsupported(_))
    ));
    assert_eq!(root.world.locations, locations);
}

#[test]
fn primary_workers_keep_victim_choices_and_compiled_recipes_independent() {
    let mut root = NativeSession::compile(source("primary-noble_blade-ordinary")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let choices = match &request.choices {
        RequestChoices::Units { values, .. } => {
            values.iter().map(|(id, _)| *id).collect::<Vec<_>>()
        }
        _ => panic!("unit choices expected"),
    };
    assert!(choices.len() >= 2);
    let before = root.semantic_state().unwrap();
    let mut left = root.clone();
    let mut right = root.clone();
    left.submit(request.id, Datum::Text(choices[0])).unwrap();
    right.submit(request.id, Datum::Text(choices[1])).unwrap();
    left.advance(&ExecutionBudget::default()).unwrap();
    right.advance(&ExecutionBudget::default()).unwrap();
    assert_ne!(
        left.semantic_state().unwrap()["execution_context"]["victim_id"],
        right.semantic_state().unwrap()["execution_context"]["victim_id"]
    );
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(&left.steps, &right.steps));
    assert!(Arc::ptr_eq(
        &left.world.definitions,
        &right.world.definitions
    ));
}
#[test]
#[cfg_attr(
    not(feature = "full-parity"),
    ignore = "full-parity: exhaustive restoration matrix; requires complete named case family"
)]
fn primary_effects_restore_every_instruction_with_frozen_stats_and_exact_events() {
    fn finish(mut session: NativeSession, restore: bool) -> (Value, Vec<Value>) {
        let mut events = Vec::new();
        for _ in 0..100 {
            if restore {
                session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
            }
            let result = session.advance_prefix(1).unwrap();
            events.extend(result.events.iter().map(|event| session.event_json(event)));
            if let Some(request) = result.request {
                let value = if request.can_skip {
                    json!("SKIP")
                } else {
                    match &request.choices {
                        RequestChoices::Units { values, .. } => {
                            json!(session.symbols.name(values[0].0))
                        }
                        RequestChoices::Hexes(values) => serde_json::to_value(values[0]).unwrap(),
                        RequestChoices::Defense(_) => json!("PASS"),
                        RequestChoices::Numbers { values, .. } => json!(values.last().unwrap()),
                        _ => panic!("unexpected primary request"),
                    }
                };
                let selection = session.import_selection(&value).unwrap();
                session.submit(request.id, selection).unwrap();
            }
            if session.stack.is_empty() {
                return (session.semantic_state().unwrap(), events);
            }
        }
        panic!("primary continuation did not finish")
    }
    for effect in [
        "noble_blade",
        "onslaught",
        "leaping_strike",
        "poisoned_dagger",
        "liquid_leap",
        "arcane_whirlpool",
        "brutal_jab",
        "blink_strike",
        "mad_dash",
        "rogue_wave",
    ] {
        let root = NativeSession::compile(source(&format!("primary-{effect}-items"))).unwrap();
        let expected = finish(root.clone(), false);
        let restored = finish(root, true);
        assert_eq!(restored, expected, "{effect}");
        assert!(!restored.0.to_string().contains(crate::primary::HERO));
        assert!(!restored.0.to_string().contains(crate::primary::CARD));
    }
}
#[test]
fn invalid_loaded_map_placement_is_a_rule_error_before_world_mutation() {
    let mut envelope = source("primary-primitive-place-literal");
    envelope.state["execution_context"]["dest"] = json!({"q":1000,"r":-1000,"s":0});
    let mut session = NativeSession::compile(envelope).unwrap();
    let before = session.world.position(session.planning.actor.unwrap());
    assert!(matches!(
        session.advance(&ExecutionBudget::default()),
        Err(EngineError::Rule(_))
    ));
    assert_eq!(
        session.world.position(session.planning.actor.unwrap()),
        before
    );
}

#[test]
fn push_workers_keep_positions_collision_and_victim_context_private() {
    let root = NativeSession::compile(source("push-ordinary")).unwrap();
    let before = root.semantic_state().unwrap();
    let mut left = root.clone();
    let mut right = root.clone();
    let distance = left.symbols.lookup("distance").unwrap();
    left.context.set(distance, Datum::Int(1));
    right.context.set(distance, Datum::Int(3));
    let a = left.advance_prefix(1).unwrap();
    let b = right.advance_prefix(1).unwrap();
    let target = left.symbols.lookup("hero_brogan").unwrap();
    assert_ne!(left.world.position(target), right.world.position(target));
    assert_eq!(left.event_json(&a.events[0])["metadata"]["distance"], 1);
    assert_eq!(right.event_json(&b.events[0])["metadata"]["distance"], 3);
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(&left.steps, &right.steps));
    assert!(Arc::ptr_eq(
        &left.world.definitions,
        &right.world.definitions
    ));
}
#[test]
fn primary_ranged_operands_are_frozen_before_export_and_restore() {
    for (variant, ranged) in [("melee", false), ("ranged", true)] {
        let mut original =
            NativeSession::compile(source(&format!("primary-rogue_wave-{variant}"))).unwrap();
        original.advance_prefix(1).unwrap();
        let exported = original.export_envelope().unwrap();
        assert_eq!(
            exported.state["execution_stack"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["is_ranged"],
            ranged
        );
        let mut restored = NativeSession::compile(exported).unwrap();
        restored.advance_prefix(1).unwrap();
        assert_eq!(
            restored.semantic_state().unwrap()["execution_context"]["attack_is_ranged"],
            ranged
        );
    }
}
#[test]
fn primary_defense_marker_workers_keep_actor_flags_and_markers_private() {
    let root = NativeSession::compile(source("defense-effect-atomic-close_call")).unwrap();
    let before = root.semantic_state().unwrap();
    let mut worker = root.clone();
    worker.advance_prefix(3).unwrap();
    let halfway = worker.semantic_state().unwrap();
    assert_eq!(halfway["current_actor_id"], "hero_bain");
    assert_eq!(halfway["execution_context"]["auto_block"], true);
    assert_eq!(halfway["markers"]["bounty"]["target_id"], "hero_brogan");
    let event = worker.advance_prefix(1).unwrap();
    assert_eq!(event.events.len(), 1);
    assert!(matches!(
        event.events[0],
        NativeEvent::Marker(MarkerEvent::Placed { .. })
    ));
    assert_eq!(
        worker.semantic_state().unwrap()["markers"]["bounty"]["target_id"],
        "hero_bain"
    );
    worker.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(
        worker.semantic_state().unwrap()["current_actor_id"],
        "hero_brogan"
    );
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(
        &root.world.definitions,
        &worker.world.definitions
    ));
    assert!(Arc::ptr_eq(&root.reaction_rules, &worker.reaction_rules));
}
#[test]
fn defense_effect_generated_flags_markers_and_actor_restoration_survive_export() {
    for name in [
        "defense-effect-atomic-aspiring_duelist",
        "defense-effect-atomic-close_call",
        "defense-effect-atomic-narrow_escape",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        let mut complete = root.clone();
        complete.advance(&ExecutionBudget::default()).unwrap();
        let expected = complete.semantic_state().unwrap();
        for steps in 0..=6 {
            let mut partial = root.clone();
            partial.advance_prefix(steps).unwrap();
            let mut restored = NativeSession::compile(partial.export_envelope().unwrap()).unwrap();
            assert_eq!(
                partial.semantic_state().unwrap(),
                restored.semantic_state().unwrap()
            );
            restored.advance(&ExecutionBudget::default()).unwrap();
            assert_eq!(restored.semantic_state().unwrap(), expected);
        }
    }
}
#[test]
fn projectile_on_block_target_and_discard_prompts_execute_the_custom_body() {
    let mut envelope = source("defense-effect-reflect_projectiles-text-True");
    envelope.state["execution_context"]["block_succeeded"] = json!(true);
    let mut step = combat_template("on_block").unwrap();
    step["step_id"] = json!("fixture/projectile-on-block");
    envelope.state["execution_stack"] = json!([step]);
    let mut session = NativeSession::compile(envelope).unwrap();
    let wasp = session.symbols.lookup("hero_wasp").unwrap();
    let victim = session.symbols.lookup("hero_brogan").unwrap();
    let origin = session.world.position(wasp).unwrap();
    let near = session
        .world
        .tile_hexes()
        .iter()
        .copied()
        .find(|h| origin.distance(*h) <= 3 && !session.world.obstacle(*h))
        .unwrap();
    session.world.move_entity(victim, near).unwrap();
    let target = session
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert_eq!(target.owner, wasp);
    let RequestChoices::Units { values, .. } = &target.choices else {
        panic!("expected custom target choice");
    };
    assert!(values.iter().any(|(id, _)| *id == victim));
    session.submit(target.id, Datum::Text(victim)).unwrap();
    let discard = session
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert_eq!(discard.owner, victim);
    let RequestChoices::CardIds { values, .. } = &discard.choices else {
        panic!("expected forced discard");
    };
    let chosen = values[0];
    let printed = session.symbols.name(chosen).to_owned();
    session.submit(discard.id, Datum::Text(chosen)).unwrap();
    session.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(
        session.semantic_state().unwrap()["turn_discard_log"]["hero_brogan"],
        json!([printed])
    );
    assert_eq!(session.planning.actor, Some(victim));
}
#[test]
fn card_selection_workers_keep_choices_and_metadata_private() {
    let mut root = NativeSession::compile(source("card-select-hand")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let RequestChoices::CardIds { values, .. } = &request.choices else {
        panic!("expected card choice");
    };
    let before = root.semantic_state().unwrap();
    let mut left = root.clone();
    let mut right = root.clone();
    left.submit(request.id, Datum::Text(values[0])).unwrap();
    right.submit(request.id, Datum::Text(values[1])).unwrap();
    left.advance(&ExecutionBudget::default()).unwrap();
    right.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(root.semantic_state().unwrap(), before);
    assert_ne!(
        left.semantic_state().unwrap()["execution_context"]["chosen"],
        right.semantic_state().unwrap()["execution_context"]["chosen"]
    );
    assert_ne!(
        left.semantic_state().unwrap()["execution_context"]["chosen_color"],
        right.semantic_state().unwrap()["execution_context"]["chosen_color"]
    );
    assert!(Arc::ptr_eq(&left.steps, &right.steps));
}
#[test]
fn forced_discard_snapshots_victim_and_restores_before_selection_and_discard() {
    let mut session = NativeSession::compile(source("card-select-force-key")).unwrap();
    session.advance_prefix(1).unwrap();
    let victim_key = session.symbols.lookup("victim").unwrap();
    let different = session.symbols.lookup("hero_arien").unwrap();
    session.context.set(victim_key, Datum::Text(different));
    let mut restored = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
    let request = restored
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert_eq!(restored.symbols.name(request.owner), "hero_brogan");
    let RequestChoices::CardIds { values, .. } = &request.choices else {
        panic!("expected captured victim's cards");
    };
    let chosen = values[0];
    let printed = restored.symbols.name(chosen).to_owned();
    restored.submit(request.id, Datum::Text(chosen)).unwrap();
    restored.advance_prefix(1).unwrap();
    let mut final_step = NativeSession::compile(restored.export_envelope().unwrap()).unwrap();
    final_step.advance(&ExecutionBudget::default()).unwrap();
    let state = final_step.semantic_state().unwrap();
    assert_eq!(state["turn_discard_log"]["hero_brogan"], json!([printed]));
    assert_eq!(state["execution_context"]["victim"], "hero_arien");
    assert_eq!(
        state["execution_context"]["discarded_card_owner_id"],
        "hero_brogan"
    );
}
#[test]
fn attack_workers_choose_different_targets_without_sharing_mutations() {
    let mut root = NativeSession::compile(source("attack-minion")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let before = root.semantic_state().unwrap();
    let mut minion = root.clone();
    let mut hero = root.clone();
    let enemy = root.symbols.lookup("enemy").unwrap();
    let blue = root.symbols.lookup("blue").unwrap();
    minion.submit(request.id, Datum::Text(enemy)).unwrap();
    hero.submit(request.id, Datum::Text(blue)).unwrap();
    minion.advance(&ExecutionBudget::default()).unwrap();
    let defense = hero
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert!(matches!(defense.choices, RequestChoices::Defense(_)));
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(minion.world.position(enemy).is_none());
    assert!(hero.world.position(enemy).is_some());
    assert_eq!(hero.world.position(blue), root.world.position(blue));
    assert!(Arc::ptr_eq(
        &minion.world.definitions,
        &hero.world.definitions
    ));
}
#[test]
fn attack_bindings_and_optional_followups_restore_between_steps_and_prompts() {
    for name in ["attack-minion", "attack-optional-minion", "attack-bonuses"] {
        let envelope = source(name);
        let mut uninterrupted = NativeSession::compile(envelope.clone()).unwrap();
        let request = uninterrupted
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let target = uninterrupted.symbols.lookup("enemy").unwrap();
        uninterrupted
            .submit(request.id, Datum::Text(target))
            .unwrap();
        uninterrupted.advance(&ExecutionBudget::default()).unwrap();
        let expected = uninterrupted.semantic_state().unwrap();
        let mut partial = NativeSession::compile(envelope).unwrap();
        assert_eq!(
            partial
                .advance(&ExecutionBudget {
                    max_steps: 1,
                    ..Default::default()
                })
                .unwrap_err(),
            EngineError::StepLimit
        );
        let mut restored = NativeSession::compile(partial.export_envelope().unwrap()).unwrap();
        assert_eq!(
            partial.semantic_state().unwrap(),
            restored.semantic_state().unwrap()
        );
        let prompt = restored
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let selection = restored.import_selection(&json!("enemy")).unwrap();
        restored.submit(prompt.id, selection).unwrap();
        restored.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(restored.semantic_state().unwrap(), expected);
    }
    let mut attack = NativeSession::compile(source("attack-preselected-hero")).unwrap();
    let request = attack
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let mut restored = NativeSession::compile(attack.export_envelope().unwrap()).unwrap();
    let pass = attack.symbols.lookup("PASS").unwrap();
    attack.submit(request.id, Datum::Text(pass)).unwrap();
    let restored_pass = restored.import_selection(&json!("PASS")).unwrap();
    let token = restored.import_request_token(&attack.request_token(request.id));
    restored.submit(token, restored_pass).unwrap();
    attack.advance(&ExecutionBudget::default()).unwrap();
    restored.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(
        attack.semantic_state().unwrap(),
        restored.semantic_state().unwrap()
    );
}
#[test]
fn defense_workers_keep_card_knowledge_history_and_action_state_private() {
    let mut root = NativeSession::compile(source("reaction-secondary-combat")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let RequestChoices::Defense(menu) = &request.choices else {
        panic!("expected defense menu");
    };
    let id = menu.cards.iter().find(|c| !c.primary).unwrap().id;
    let before = root.semantic_state().unwrap();
    let mut defense = root.clone();
    let mut pass = root.clone();
    defense.submit(request.id, Datum::Text(id)).unwrap();
    let pass_id = pass.symbols.lookup("PASS").unwrap();
    pass.submit(request.id, Datum::Text(pass_id)).unwrap();
    defense.advance(&ExecutionBudget::default()).unwrap();
    pass.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(root.semantic_state().unwrap(), before);
    let state = defense.semantic_state().unwrap();
    assert!(
        state["public_revealed_cards"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["card_id"] == root.symbols.name(id))
    );
    assert_eq!(
        state["turn_discard_log"][root.symbols.name(menu.owner)],
        json!([root.symbols.name(id)])
    );
    assert_eq!(state["execution_context"]["current_action_type"], "ATTACK");
    assert!(
        pass.semantic_state().unwrap()["entity_locations"]
            .get(root.symbols.name(menu.owner))
            .is_none()
    );
    assert!(Arc::ptr_eq(
        &defense.world.definitions,
        &pass.world.definitions
    ));
}
#[test]
fn defense_discard_and_passive_bindings_restore_between_generated_steps() {
    let mut root = NativeSession::compile(source("reaction-secondary-combat")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let RequestChoices::Defense(menu) = &request.choices else {
        panic!("expected defense menu");
    };
    let id = menu.cards.iter().find(|c| !c.primary).unwrap().id;
    root.submit(request.id, Datum::Text(id)).unwrap();
    let mut complete = root.clone();
    complete.advance(&ExecutionBudget::default()).unwrap();
    let expected = complete.semantic_state().unwrap();
    for count in [1, 2] {
        let mut interrupted = root.clone();
        assert_eq!(
            interrupted
                .advance(&ExecutionBudget {
                    max_steps: count,
                    ..Default::default()
                })
                .unwrap_err(),
            EngineError::StepLimit
        );
        let mut restored = NativeSession::compile(interrupted.export_envelope().unwrap()).unwrap();
        assert_eq!(
            restored.semantic_state().unwrap(),
            interrupted.semantic_state().unwrap()
        );
        restored.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(restored.semantic_state().unwrap(), expected);
    }
}
#[test]
fn combat_workers_keep_life_gold_markers_and_occupancy_private() {
    let root = NativeSession::compile(source("combat-markers-razzle_piece_1")).unwrap();
    let before = root.semantic_state().unwrap();
    let mut worker = root.clone();
    worker.advance(&ExecutionBudget::default()).unwrap();
    let after = worker.semantic_state().unwrap();
    assert_eq!(root.semantic_state().unwrap(), before);
    assert_eq!(
        after["teams"]["RED"]["life_counters"].as_i64().unwrap(),
        before["teams"]["RED"]["life_counters"].as_i64().unwrap() - 2
    );
    assert!(after["markers"]["bounty"]["target_id"].is_null());
    assert!(after["entity_locations"].get("razzle_piece_1").is_none());
    assert!(after["entity_locations"].get("razzle_piece_2").is_none());
    assert_eq!(
        after["teams"]["RED"]["heroes"][0]["gold"].as_i64().unwrap(),
        before["teams"]["RED"]["heroes"][0]["gold"]
            .as_i64()
            .unwrap()
            + 1
    );
    assert!(Arc::ptr_eq(
        &worker.world.definitions,
        &root.world.definitions
    ));
    assert!(Arc::ptr_eq(&worker.steps, &root.steps));
}
#[test]
fn generated_defeat_removal_and_game_over_restore_at_step_boundaries() {
    for name in ["combat-hit", "combat-annihilation-1", "combat-card-alias"] {
        let source = source(name);
        let mut complete = NativeSession::compile(source.clone()).unwrap();
        complete.advance(&ExecutionBudget::default()).unwrap();
        let expected = complete.semantic_state().unwrap();
        let boundaries: &[usize] = if name == "combat-card-alias" {
            &[1]
        } else {
            &[1, 2]
        };
        for &count in boundaries {
            let mut interrupted = NativeSession::compile(source.clone()).unwrap();
            assert_eq!(
                interrupted
                    .advance(&ExecutionBudget {
                        max_steps: count,
                        ..Default::default()
                    })
                    .unwrap_err(),
                EngineError::StepLimit
            );
            let envelope = interrupted.export_envelope().unwrap();
            let before = interrupted.semantic_state().unwrap();
            let mut restored = NativeSession::compile(envelope).unwrap();
            assert_eq!(before, restored.semantic_state().unwrap());
            interrupted.advance(&ExecutionBudget::default()).unwrap();
            restored.advance(&ExecutionBudget::default()).unwrap();
            assert_eq!(restored.semantic_state().unwrap(), expected);
            assert_eq!(interrupted.semantic_state().unwrap(), expected);
        }
    }
}
#[test]
fn restored_identifier_counters_fail_explicitly_instead_of_overflowing() {
    let mut envelope = source("travel-sequence");
    envelope.state["execution_stack"][0]["step_id"] =
        json!(format!("native-step-{}", u64::MAX - 1));
    let mut session = NativeSession::compile(envelope).unwrap();
    assert!(
        matches!(session.advance(&ExecutionBudget::default()),Err(EngineError::Unsupported(message)) if message.contains("step generation counter exhausted"))
    );
    let mut envelope = source("number-input-resume");
    envelope.state["execution_stack"][0]["pending_request_id"] =
        json!(format!("native-request-{}", u64::MAX - 1));
    let mut session = NativeSession::compile(envelope).unwrap();
    let request = session
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    session.submit(request.id, Datum::Null).unwrap();
    assert!(
        matches!(session.advance(&ExecutionBudget::default()),Err(EngineError::Unsupported(message)) if message.contains("request generation counter exhausted"))
    );
}
#[test]
fn travel_workers_relocate_independently_and_emit_placement_not_movement() {
    let mut root = NativeSession::compile(source("travel-sequence")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let RequestChoices::Hexes(choices) = request.choices else {
        panic!("expected destination choice");
    };
    let before = root.semantic_state().unwrap();
    let mut left = root.clone();
    let mut right = root.clone();
    left.submit(request.id, native_hex(choices[0], root.coordinates))
        .unwrap();
    right
        .submit(request.id, native_hex(choices[1], root.coordinates))
        .unwrap();
    let events = left.advance(&ExecutionBudget::default()).unwrap().events;
    assert!(matches!(
        events.as_slice(),
        [NativeEvent::UnitPlaced { .. }]
    ));
    right.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(root.semantic_state().unwrap(), before);
    let actor = root.planning.actor.unwrap();
    assert_eq!(left.world.position(actor), Some(choices[0]));
    assert_eq!(right.world.position(actor), Some(choices[1]));
    assert!(Arc::ptr_eq(
        &left.world.definitions,
        &right.world.definitions
    ));
}
#[test]
fn compact_travel_continuation_restores_real_card_aliases_and_request_identity() {
    // Use a registered full game, rather than a cardless board.
    let mut root = NativeSession::compile(source("travel-forgotten_island-change-False")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let envelope = root.export_envelope().unwrap();
    envelope.validate().unwrap();
    let mut restored = NativeSession::compile(envelope).unwrap();
    let original_hero = root.world.hero(root.planning.actor.unwrap()).unwrap();
    let restored_hero = restored
        .world
        .hero(restored.planning.actor.unwrap())
        .unwrap();
    assert_eq!(original_hero.hand, restored_hero.hand);
    assert_eq!(original_hero.deck, restored_hero.deck);
    assert!(
        restored_hero
            .hand
            .iter()
            .all(|id| restored_hero.deck.contains(id))
    );
    let resumed = restored
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert_eq!(root.request_json(&request), restored.request_json(&resumed));
    let RequestChoices::Hexes(choices) = request.choices else {
        panic!("expected destination choice");
    };
    root.submit(request.id, native_hex(choices[0], root.coordinates))
        .unwrap();
    restored
        .submit(resumed.id, native_hex(choices[0], restored.coordinates))
        .unwrap();
    root.advance(&ExecutionBudget::default()).unwrap();
    restored.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(
        root.semantic_state().unwrap(),
        restored.semantic_state().unwrap()
    );
}
#[test]
fn acting_piece_binding_is_private_to_each_search_worker() {
    let mut root = NativeSession::compile(source("unit-choose-piece-invalid")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let RequestChoices::Units { values, .. } = &request.choices else {
        panic!("expected piece choice");
    };
    let mut left = root.clone();
    let mut right = root.clone();
    left.submit(request.id, Datum::Text(values[0].0)).unwrap();
    right.submit(request.id, Datum::Text(values[1].0)).unwrap();
    left.advance(&ExecutionBudget::default()).unwrap();
    right.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(root.world.acting_piece, None);
    assert_eq!(left.world.acting_piece, Some(values[0].0));
    assert_eq!(right.world.acting_piece, Some(values[1].0));
    assert!(Arc::ptr_eq(
        &left.world.definitions,
        &right.world.definitions
    ));
    assert_ne!(
        left.semantic_state().unwrap(),
        right.semantic_state().unwrap()
    );
}
#[test]
fn heavy_immunity_tracks_each_workers_support_position() {
    let root = NativeSession::compile(source("unit-heavy-immune")).unwrap();
    let heavy = root.symbols.lookup("heavy").unwrap();
    let support = root.symbols.lookup("support").unwrap();
    let actor = root.planning.actor;
    assert!(root.world.ordinary_immune(heavy, actor));
    let mut worker = root.clone();
    worker
        .world
        .move_entity(support, Hex::new(4, -4, 0).unwrap())
        .unwrap();
    assert!(!worker.world.ordinary_immune(heavy, actor));
    assert!(root.world.ordinary_immune(heavy, actor));
}
#[test]
fn retrieval_workers_can_keep_different_cards_without_mutating_root() {
    let mut root = NativeSession::compile(source("retrieve-unresolved-first")).unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let RequestChoices::Cards(choices) = &request.choices else {
        panic!("expected cards");
    };
    let before = root.semantic_state().unwrap();
    let mut left = root.clone();
    let mut right = root.clone();
    left.submit(request.id, Datum::Text(choices[0].0)).unwrap();
    right.submit(request.id, Datum::Text(choices[1].0)).unwrap();
    assert_eq!(
        left.advance(&ExecutionBudget::default())
            .unwrap()
            .events
            .len(),
        1
    );
    assert_eq!(
        right
            .advance(&ExecutionBudget::default())
            .unwrap()
            .events
            .len(),
        1
    );
    assert_eq!(root.semantic_state().unwrap(), before);
    assert_ne!(
        left.semantic_state().unwrap(),
        right.semantic_state().unwrap()
    );
    assert!(Arc::ptr_eq(
        &left.world.definitions,
        &right.world.definitions
    ));
    let hero = root
        .world
        .heroes
        .iter()
        .find(|h| root.symbols.name(h.id) == "hero_emmitt")
        .unwrap()
        .id;
    assert_ne!(
        left.world.hero(hero).unwrap().current,
        right.world.hero(hero).unwrap().current
    );
}
#[test]
fn independent_workers_share_only_immutable_program_data() {
    let mut root = NativeSession::compile(source("number-input-resume")).unwrap();
    root.advance(&ExecutionBudget::default()).unwrap();
    let before = root.semantic_state().unwrap();
    let mut child = root.clone();
    assert!(Arc::ptr_eq(&root.source, &child.source));
    assert!(Arc::ptr_eq(&root.steps, &child.steps));
    let token = child.last_request.as_ref().unwrap().id;
    child.submit(token, Datum::Int(2)).unwrap();
    child.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(root.semantic_state().unwrap(), before);
    assert_ne!(child.semantic_state().unwrap(), before);
    assert_eq!(root.last_request.as_ref().unwrap().id, token);
}
#[test]
fn spatial_workers_share_definitions_but_never_positions_or_occupancy() {
    let mut root = NativeSession::compile(source("movement-resume-events")).unwrap();
    root.advance(&ExecutionBudget::default()).unwrap();
    let before = root.semantic_state().unwrap();
    let mut child = root.clone();
    assert!(Arc::ptr_eq(
        &root.world.definitions,
        &child.world.definitions
    ));
    let request = child.last_request.as_ref().unwrap().id;
    child.submit(request, Datum::Int(1)).unwrap();
    let moved = child.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(moved.events.len(), 1);
    assert_eq!(root.semantic_state().unwrap(), before);
    assert_ne!(child.semantic_state().unwrap(), before);
    let symbol = child.symbols.lookup("hero_arien").unwrap();
    assert_ne!(root.world.position(symbol), child.world.position(symbol));
}
#[test]
fn generated_movement_continuations_restore_without_losing_bindings_or_identity() {
    let mut session = NativeSession::compile(source("macro-default")).unwrap();
    session.advance(&ExecutionBudget::default()).unwrap();
    let mut envelope = source("macro-default");
    envelope.state = session.semantic_state().unwrap();
    let mut restored = NativeSession::compile(envelope).unwrap();
    assert!(restored.next_step >= 2);
    let request = restored
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert_eq!(
        restored.request_token(request.id),
        session.request_token(session.last_request.as_ref().unwrap().id)
    );
    let choice = serde_json::json!({"q":1,"r":-1,"s":0});
    let value = restored.import_selection(&choice).unwrap();
    restored.submit(request.id, value).unwrap();
    restored.advance(&ExecutionBudget::default()).unwrap();
    let value = session.import_selection(&choice).unwrap();
    session
        .submit(session.last_request.as_ref().unwrap().id, value)
        .unwrap();
    session.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(
        restored.semantic_state().unwrap(),
        session.semantic_state().unwrap()
    );
}
#[test]
fn deadlines_and_cancellation_preserve_waiting_state() {
    let mut session = NativeSession::compile(source("number-input-resume")).unwrap();
    session.advance(&ExecutionBudget::default()).unwrap();
    let before = session.semantic_state().unwrap();
    let token = session.last_request.as_ref().unwrap().id;
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        session
            .advance(&ExecutionBudget {
                cancelled: Some(&cancelled),
                ..Default::default()
            })
            .unwrap_err(),
        EngineError::Cancelled
    );
    assert_eq!(session.last_request.as_ref().unwrap().id, token);
    assert_eq!(session.semantic_state().unwrap(), before);
    assert_eq!(
        session
            .advance(&ExecutionBudget {
                deadline: Some(Instant::now()),
                ..Default::default()
            })
            .unwrap_err(),
        EngineError::Deadline
    );
    assert_eq!(session.semantic_state().unwrap(), before);
}
#[test]
fn unsupported_nested_instruction_rejects_whole_root() {
    let mut source = source("foreach-paused-copies");
    source.state["execution_stack"][1]["steps_template"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"damage"}));
    assert!(matches!(
        NativeSession::compile(source),
        Err(EngineError::Unsupported(_))
    ));
}
#[test]
fn consumed_request_ids_do_not_reappear_after_restore_without_a_pending_frame() {
    let mut root = NativeSession::compile(source("number-input-resume")).unwrap();
    let first = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let old = root.request_token(first.id);
    root.submit(first.id, Datum::Int(2)).unwrap();
    let consumed = root.advance_prefix(1).unwrap();
    assert!(consumed.request.is_none());
    assert!(root.stack.iter().all(|f| f.request.is_none()));
    let encoded = serde_json::to_value(root.export_envelope().unwrap()).unwrap();
    let envelope = serde_json::from_value(encoded).unwrap();
    let mut restored = NativeSession::compile(envelope).unwrap();
    let next = restored
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let next_name = restored.request_token(next.id);
    assert_ne!(
        next_name, old,
        "a completed prompt must not identify a later prompt"
    );
    let before = restored.semantic_state().unwrap();
    let stale = restored.import_request_token(&old);
    assert!(matches!(
        restored.submit(stale, Datum::Text(restored.yes)),
        Err(EngineError::InvalidInput(_))
    ));
    assert_eq!(restored.semantic_state().unwrap(), before);
    let uninterrupted = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert_eq!(root.request_token(uninterrupted.id), next_name);
    restored.submit(next.id, Datum::Text(restored.yes)).unwrap();
}

#[test]
fn request_wire_identity_is_exact_for_generated_and_imported_prompts() {
    let mut session = NativeSession::compile(source("number-input-resume")).unwrap();
    let request = session
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert_eq!(session.request_token(request.id), "native-request-1");
    let before = session.semantic_state().unwrap();
    for alias in [
        "native-request-01",
        "native-request-+1",
        "native-request-0001",
    ] {
        let token = session.import_request_token(alias);
        assert!(
            matches!(
                session.submit(token, Datum::Int(2)),
                Err(EngineError::InvalidInput(_))
            ),
            "{alias}"
        );
        assert_eq!(session.semantic_state().unwrap(), before);
    }
    let mut envelope = session.export_envelope().unwrap();
    let frame = envelope.state["execution_stack"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap();
    frame["pending_request_id"] = json!("native-request-01");
    let mut imported = NativeSession::compile(envelope).unwrap();
    let request = imported
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert_eq!(imported.request_token(request.id), "native-request-01");
    let alias = imported.import_request_token("native-request-1");
    assert!(matches!(
        imported.submit(alias, Datum::Int(2)),
        Err(EngineError::InvalidInput(_))
    ));
    let exact = imported.import_request_token("native-request-01");
    imported.submit(exact, Datum::Int(2)).unwrap();
}
#[test]
fn completed_generated_steps_keep_identity_history_outside_game_state() {
    let mut root = NativeSession::compile(source("travel-sequence")).unwrap();
    for _ in 0..20 {
        let result = root.advance(&ExecutionBudget::default()).unwrap();
        if let Some(request) = result.request {
            let wire = root.request_json(&request);
            let value = wire["options"][0]["metadata"]["hex"].clone();
            let value = root.import_selection(&value).unwrap();
            root.submit(request.id, value).unwrap();
        }
        if root.stack.is_empty() {
            break;
        }
    }
    assert!(root.stack.is_empty());
    assert!(root.next_step > 0 && root.next_request > 0);
    let semantic = root.semantic_state().unwrap();
    assert!(semantic.get("native_resume").is_none());
    let encoded = serde_json::to_value(root.export_envelope().unwrap()).unwrap();
    let envelope = serde_json::from_value(encoded).unwrap();
    let mut restored = NativeSession::compile(envelope).unwrap();
    assert_eq!(restored.semantic_state().unwrap(), semantic);
    assert_eq!(restored.next_step, root.next_step);
    assert_eq!(restored.next_request, root.next_request);
    assert!(
        restored
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .is_none()
    );
    assert_eq!(
        restored.export_envelope().unwrap().native_resume,
        root.export_envelope().unwrap().native_resume
    );
}
#[test]
fn resume_rejects_regressing_and_exhausted_identity_history() {
    let mut root = NativeSession::compile(source("travel-sequence")).unwrap();
    root.advance(&ExecutionBudget::default()).unwrap();
    let envelope = root.export_envelope().unwrap();
    for request_counter in [false, true] {
        let mut bad = envelope.clone();
        let metadata = bad.native_resume.as_mut().unwrap();
        if request_counter {
            metadata.request_high_water = 0;
        } else {
            metadata.step_high_water = 0;
        }
        assert!(
            matches!(NativeSession::compile(bad), Err(EngineError::Unsupported(message)) if message.contains("precede live"))
        );
    }
    for request_counter in [false, true] {
        let mut bad = envelope.clone();
        let metadata = bad.native_resume.as_mut().unwrap();
        if request_counter {
            metadata.request_high_water = u64::MAX;
        } else {
            metadata.step_high_water = u64::MAX;
        }
        assert!(
            matches!(NativeSession::compile(bad), Err(EngineError::Unsupported(message)) if message.contains("counter exhausted"))
        );
    }
}
#[test]
fn generated_template_id_prefixes_reserve_their_generation() {
    let mut envelope = source("travel-sequence");
    envelope.state["execution_stack"][1]["step_id"] = json!("native-step-200-template-3");
    let mut root = NativeSession::compile(envelope).unwrap();
    assert_eq!(root.next_step, 200);
    root.advance_prefix(1).unwrap();
    let wire = root.semantic_state().unwrap();
    let names = wire["execution_stack"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["step_id"].as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"native-step-201") && names.contains(&"native-step-202"));
}

#[test]
fn imported_requests_work_and_rejected_choice_gets_fresh_identity() {
    let mut source = source("number-input-resume");
    let frames = source.state["execution_stack"].as_array_mut().unwrap();
    // Drop automatic prefix and import the paused number step directly.
    frames.pop();
    frames.pop();
    frames.last_mut().unwrap()["pending_request_id"] = json!("native-request-1");
    let mut session = NativeSession::compile(source).unwrap();
    let first = session
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let imported = session.import_request_token("native-request-1");
    assert_eq!(imported, first.id);
    session.submit(imported, Datum::Null).unwrap();
    let next = session
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert_ne!(
        session.request_token(first.id),
        session.request_token(next.id)
    );
    assert!(matches!(
        session.submit(imported, Datum::Int(2)),
        Err(EngineError::InvalidInput(_))
    ));
}
#[test]
fn automatic_step_limit_is_real_and_does_not_count_fake_progress() {
    let mut session = NativeSession::compile(source("number-input-resume")).unwrap();
    assert_eq!(
        session
            .advance(&ExecutionBudget {
                max_steps: 1,
                ..Default::default()
            })
            .unwrap_err(),
        EngineError::StepLimit
    );
    let state = session.semantic_state().unwrap();
    assert_eq!(state["execution_context"]["enabled"], true);
    assert!(state["execution_context"].get("count").is_none());
}
#[test]
fn token_linked_effect_removal_restores_at_instruction_boundaries() {
    for name in [
        "token-effect-prefix",
        "token-effect-create-remove",
        "token-effect-duplicate-id",
        "token-effect-offboard",
        "token-effect-card-flags",
        "token-effect-barrier-only",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}

#[test]
fn token_removal_keeps_other_workers_and_unlinked_same_id_effects() {
    let root = NativeSession::compile(source("token-effect-duplicate-id")).unwrap();
    let before = root.semantic_state().unwrap();
    let mut placed = root.clone();
    let mut offboard = root.clone();
    let token = root.symbols.lookup("effect_token").unwrap();
    offboard.world.remove_entity(token).unwrap();
    let before_offboard = offboard.world.effects.instances.len();
    let a = placed.advance(&ExecutionBudget::default()).unwrap();
    let b = offboard.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(placed.world.effects.instances.len(), before_offboard - 1);
    assert_eq!(offboard.world.effects.instances.len(), before_offboard);
    assert!(b.events.is_empty());
    assert!(a.events.iter().any(|event| matches!(
        event,
        NativeEvent::TokenRemoved {
            effects_removed: 1,
            ..
        }
    )));
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(&root.steps, &placed.steps));
    assert!(Arc::ptr_eq(
        &root.world.definitions,
        &placed.world.definitions
    ));
}
#[test]
fn effect_topology_adjacency_restores_at_every_instruction() {
    for name in [
        "effect-adjacency-isolation-in",
        "effect-adjacency-isolation-out",
        "effect-adjacency-shield",
        "effect-adjacency-shield-except",
        "effect-adjacency-prefix",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
fn recursive_reference_immunity_topology_is_explicitly_unsupported() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/effect-errors.json");
    let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    for case in fixture["cases"].as_array().unwrap() {
        assert_eq!(case["reference_error"], "RecursionError");
        let root =
            NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                .unwrap();
        let before = root.semantic_state().unwrap();
        let mut worker = root.clone();
        assert!(
            matches!(worker.advance(&ExecutionBudget::default()),Err(EngineError::Unsupported(message)) if message.contains("recursive topology/immunity"))
        );
        assert_eq!(root.semantic_state().unwrap(), before);
    }
}
#[test]
fn created_effect_stats_use_live_typed_payload_after_restore() {
    let mut source = source("effect-step-type-area_stat_modifier");
    let step = &mut source.state["execution_stack"][0];
    step["stat_type"] = json!("ATTACK");
    step["stat_value"] = json!(3);
    step["scope"]["shape"] = json!("global");
    let mut root = NativeSession::compile(source).unwrap();
    let actor = root.symbols.lookup("hero_arien").unwrap();
    let before = root.world.computed_stat(actor, Stat::Attack, 4).unwrap();
    root.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(
        root.world.computed_stat(actor, Stat::Attack, 4).unwrap(),
        before + 3
    );
    let restored = NativeSession::compile(root.export_envelope().unwrap()).unwrap();
    assert_eq!(
        restored
            .world
            .computed_stat(actor, Stat::Attack, 4)
            .unwrap(),
        before + 3
    );
}
#[test]
fn effect_placement_and_swap_restore_at_every_instruction() {
    for name in [
        "displacement-place-blocked",
        "displacement-swap-swap-second",
        "displacement-place-mandatory",
        "displacement-place-live-origin",
        "displacement-swap-live-piece-origin",
        "displacement-place-target-token-empty-actor",
        "displacement-place-target-owner",
        "displacement-swap-same-swap",
    ] {
        let root = NativeSession::compile(source(name)).unwrap();
        assert_eq!(
            run_instruction_restores(root.clone(), true),
            run_instruction_restores(root, false),
            "{name}"
        );
    }
}
#[test]
fn placement_workers_use_private_live_effect_origins() {
    let root = NativeSession::compile(source("displacement-place-blocked")).unwrap();
    let before = root.semantic_state().unwrap();
    let mut a = root.clone();
    let mut b = root.clone();
    let origin = root.symbols.lookup("hero_arien").unwrap();
    let target = root.symbols.lookup("hero_brogan").unwrap();
    b.world
        .move_entity(origin, Hex::new(-3, 0, 3).unwrap())
        .unwrap();
    let blocked = a.advance(&ExecutionBudget::default()).unwrap();
    let allowed = b.advance(&ExecutionBudget::default()).unwrap();
    assert!(blocked.events.is_empty());
    assert!(
        allowed
            .events
            .iter()
            .any(|e| matches!(e,NativeEvent::UnitPlaced{unit,..} if *unit==target))
    );
    assert_ne!(a.world.position(target), b.world.position(target));
    assert_eq!(root.semantic_state().unwrap(), before);
    assert!(Arc::ptr_eq(&a.steps, &b.steps));
}
#[test]
fn blocked_swap_checks_both_units_before_mutating_occupancy() {
    let mut session = NativeSession::compile(source("displacement-swap-swap-second")).unwrap();
    let a = session.symbols.lookup("hero_brogan").unwrap();
    let b = session.symbols.lookup("hero_hanu").unwrap();
    let before = (session.world.position(a), session.world.position(b));
    let result = session.advance(&ExecutionBudget::default()).unwrap();
    assert!(result.events.is_empty());
    assert_eq!(
        (session.world.position(a), session.world.position(b)),
        before
    );
    assert_eq!(session.world.occupant(before.0.unwrap()), Some(a));
    assert_eq!(session.world.occupant(before.1.unwrap()), Some(b));
}
