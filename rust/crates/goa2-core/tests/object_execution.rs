//! Live source identity, restore, worker isolation and execution boundaries.
use goa2_core::{
    codec::StateEnvelope,
    data::Symbols,
    interpreter::{EngineError, ExecutionBudget, NativeSession},
    world::NativeWorld,
};
use serde_json::{Value, json};
use std::{sync::atomic::AtomicBool, time::Instant};

fn cases() -> Vec<Value> {
    let base = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
    let index: Value =
        serde_json::from_reader(std::fs::File::open(base.join("object-execution.json")).unwrap())
            .unwrap();
    index["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            serde_json::from_reader(
                std::fs::File::open(base.join(entry["path"].as_str().unwrap())).unwrap(),
            )
            .unwrap()
        })
        .collect()
}
fn case(name: &str) -> Value {
    let base = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
    let index: Value =
        serde_json::from_reader(std::fs::File::open(base.join("object-execution.json")).unwrap())
            .unwrap();
    let entry = index["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap();
    serde_json::from_reader(
        std::fs::File::open(base.join(entry["path"].as_str().unwrap())).unwrap(),
    )
    .unwrap()
}
fn source(name: &str) -> StateEnvelope {
    serde_json::from_value(case(name)["envelope"].clone()).unwrap()
}
fn apply(session: &mut NativeSession, operation: &Value) -> Value {
    let mut request = Value::Null;
    let mut events = Vec::new();
    let mut error = Value::Null;
    match operation["kind"].as_str().unwrap() {
        "advance" | "advance_prefix" => {
            let result = if operation["kind"] == "advance_prefix" {
                session.advance_prefix(operation["steps"].as_u64().unwrap() as usize)
            } else {
                session.advance(&ExecutionBudget::default())
            }
            .unwrap();
            request = result
                .request
                .as_ref()
                .map_or(Value::Null, |request| session.request_json(request));
            events = result
                .events
                .iter()
                .map(|event| session.event_json(event))
                .collect();
        }
        "end_turn" => {
            session.end_turn().unwrap();
        }
        "after_cards_played" => {
            session.queue_after_cards_played().unwrap();
        }
        "expire_turn_effects" => {
            session.expire_turn_effects().unwrap();
        }
        "expire_effects" => {
            let duration =
                goa2_core::effects::Duration::parse(operation["duration"].as_str().unwrap())
                    .unwrap();
            session.expire_effects(duration).unwrap();
        }
        "remove_effects" => {
            let mut ids = Vec::new();
            for value in operation["ids"].as_array().unwrap() {
                let goa2_core::data::Datum::Text(id) = session.import_selection(value).unwrap()
                else {
                    unreachable!()
                };
                ids.push(id);
            }
            session.remove_effects(&ids).unwrap();
        }
        "queue_steps" => {
            let path: Vec<goa2_core::codec::ObjectPathPart> =
                serde_json::from_value(operation["path"].clone()).unwrap();
            session.queue_steps(&path).unwrap();
        }
        "push_action_context" => {
            let mut values = Vec::new();
            for key in ["action_type", "card", "owner"] {
                let goa2_core::data::Datum::Text(value) =
                    session.import_selection(&operation[key]).unwrap()
                else {
                    panic!("scope argument must be text");
                };
                values.push(value);
            }
            session
                .push_action_context(values[0], values[1], values[2])
                .unwrap();
        }
        "input" => {
            let token = match operation["request_id"].as_str() {
                Some("current") | None => session.last_request.as_ref().unwrap().id,
                Some(token) => session.import_request_token(token),
            };
            let selection = session.import_selection(&operation["selection"]).unwrap();
            match session.submit(token, selection) {
                Ok(()) => {}
                Err(EngineError::InvalidInput(_)) => error = json!("invalid_input"),
                Err(error) => panic!("{error}"),
            }
        }
        _ => panic!("unknown test operation"),
    }
    let envelope = session.export_envelope().unwrap();
    json!({"semantic_state":envelope.materialize().unwrap(), "context_keys":envelope.state["execution_context"].as_object().unwrap().keys().collect::<Vec<_>>(), "instance_bindings":envelope.instance_bindings().unwrap(), "object_bindings":envelope.object_bindings,
        "request":request, "events":events, "error":error})
}

#[test]
fn every_shared_execution_boundary_restores_values_aliases_and_request_generations() {
    let mut operations = 0;
    for case in cases() {
        let mut session =
            NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                .unwrap();
        for op in case["operations"].as_array().unwrap() {
            let envelope = session.export_envelope().unwrap();
            let mut restored = NativeSession::compile(envelope.clone()).unwrap();
            // last_request is a transport convenience, not persisted semantics.
            restored.last_request = session.last_request.clone().map(|mut request| {
                request.id = restored.import_request_token(&session.request_token(request.id));
                request
            });
            assert_eq!(restored.export_envelope().unwrap().state, envelope.state);
            assert_eq!(
                restored.export_envelope().unwrap().object_bindings,
                envelope.object_bindings
            );
            assert!(
                apply(&mut session, op) == apply(&mut restored, op),
                "{}: {op}",
                case["name"]
            );
            operations += 1;
        }
    }
    assert_eq!(operations, 512);
}

#[test]
fn shared_pending_step_clones_are_private_and_second_execution_requests_again() {
    let mut root = NativeSession::compile(source("object-execution-shared-pending-step")).unwrap();
    root.advance(&ExecutionBudget::default()).unwrap();
    let before = root.export_envelope().unwrap();
    let token = root.last_request.as_ref().unwrap().id;
    let mut a = root.clone();
    let mut b = root.clone();
    for (worker, choice) in [(&mut a, 1), (&mut b, 2)] {
        let selection = worker.import_selection(&json!(choice)).unwrap();
        worker.submit(token, selection).unwrap();
        worker.advance_prefix(1).unwrap();
        let envelope = worker.export_envelope().unwrap();
        let state = envelope.materialize().unwrap();
        assert_eq!(state["execution_context"]["chosen"], choice);
        for step in [
            &state["execution_context"]["left"],
            &state["execution_context"]["right"],
            &state["active_effects"][0]["finishing_steps"][0],
            &state["active_effects"][1]["finishing_steps"][0],
        ] {
            assert!(step["pending_input"].is_null());
            assert!(step["pending_request_id"].is_null());
        }
        let next = worker
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        assert_ne!(worker.request_token(next.id), root.request_token(token));
    }
    assert_ne!(a.semantic_state().unwrap(), b.semantic_state().unwrap());
    assert_eq!(root.export_envelope().unwrap().state, before.state);
    assert_eq!(
        root.export_envelope().unwrap().object_bindings,
        before.object_bindings
    );
}

#[test]
fn cancelled_expired_and_zero_prefix_leave_live_object_state_untouched() {
    let mut session =
        NativeSession::compile(source("object-execution-shared-pending-step")).unwrap();
    let before = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
    let cancelled = AtomicBool::new(true);
    assert!(matches!(
        session.advance(&ExecutionBudget {
            cancelled: Some(&cancelled),
            ..ExecutionBudget::default()
        }),
        Err(EngineError::Cancelled)
    ));
    assert!(matches!(
        session.advance(&ExecutionBudget {
            deadline: Some(Instant::now()),
            ..ExecutionBudget::default()
        }),
        Err(EngineError::Deadline)
    ));
    assert!(matches!(
        session.advance(&ExecutionBudget {
            max_steps: 0,
            ..ExecutionBudget::default()
        }),
        Err(EngineError::StepLimit)
    ));
    assert_eq!(session.advance_prefix(0).unwrap().steps, 0);
    assert_eq!(
        serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
        before
    );
}

#[test]
fn shared_world_and_phase_mutations_still_require_dense_bindings() {
    let source = source("object-execution-context-list");
    assert!(
        NativeWorld::compile(&source, &mut Symbols::default())
            .unwrap_err()
            .contains("shared-object")
    );
    let mut phase = source.clone();
    phase.state["phase"] = json!("SETUP");
    assert!(
        NativeSession::compile(phase)
            .unwrap_err()
            .to_string()
            .contains("phase flow")
    );
    let mut movement = source;
    movement.state["execution_stack"][3]["type"] = json!("move_unit");
    assert!(
        NativeSession::compile(movement)
            .unwrap_err()
            .to_string()
            .contains("instruction family")
    );
}

#[test]
fn appending_through_context_cannot_silently_mutate_unbound_world_lists() {
    use goa2_core::codec::ObjectPathPart::Key;
    let mut source = source("object-execution-context-list");
    let path = vec![Key("execution_context".into()), Key("history".into())];
    let hand = vec![
        Key("teams".into()),
        Key("BLUE".into()),
        Key("heroes".into()),
        goa2_core::codec::ObjectPathPart::Index(0),
        Key("hand".into()),
    ];
    source.state["execution_context"]["history"] =
        source.state["teams"]["BLUE"]["heroes"][0]["hand"].clone();
    for binding in &mut source.object_bindings {
        binding.paths.retain(|candidate| candidate != &path);
    }
    source
        .object_bindings
        .iter_mut()
        .find(|binding| binding.paths.contains(&hand))
        .unwrap()
        .paths
        .push(path);
    source.validate().unwrap();
    let before = source.state["teams"].clone();
    let mut session = NativeSession::compile(source).unwrap();
    session.advance_prefix(1).unwrap();
    let error = session.advance(&ExecutionBudget::default()).unwrap_err();
    assert!(error.to_string().contains("unbound world/step operand"));
    assert!(session.export_envelope().unwrap().state["teams"] == before);
}

#[test]
fn context_dictionary_itself_cannot_alias_an_unbound_world_dictionary() {
    use goa2_core::codec::ObjectPathPart::Key;
    let mut source = source("object-execution-add-list");
    source.state["execution_context"] = source.state["turn_discard_log"].clone();
    for binding in &mut source.object_bindings {
        binding
            .paths
            .retain(|path| path.first() != Some(&Key("execution_context".into())));
    }
    source
        .object_bindings
        .retain(|binding| !binding.paths.is_empty());
    source
        .object_bindings
        .iter_mut()
        .find(|binding| {
            binding
                .paths
                .contains(&vec![Key("turn_discard_log".into())])
        })
        .unwrap()
        .paths
        .push(vec![Key("execution_context".into())]);
    source.validate().unwrap();
    assert!(
        NativeSession::compile(source)
            .unwrap_err()
            .to_string()
            .contains("execution context aliases")
    );
}

#[test]
fn loop_template_copies_keep_physical_allocations_private_between_workers() {
    let mut root = NativeSession::compile(source("object-execution-loop-deep-card")).unwrap();
    root.advance_prefix(1).unwrap();
    let before = root.export_envelope().unwrap();
    let mut a = root.clone();
    let mut b = root.clone();
    a.advance(&ExecutionBudget::default()).unwrap();
    b.advance_prefix(1).unwrap();
    let completed = a.export_envelope().unwrap();
    assert!(completed.instances.len() > before.instances.len());
    assert_eq!(
        b.export_envelope().unwrap().instances.len(),
        before.instances.len()
    );
    assert_eq!(
        root.export_envelope().unwrap().instances.len(),
        before.instances.len()
    );
    assert!(root.export_envelope().unwrap().state == before.state);
    assert!(completed.state["teams"] == before.state["teams"]);
    assert_eq!(
        completed.state["execution_context"]["seen"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[test]
fn live_loop_budget_stops_at_a_real_boundary_and_resumes_remaining_iterations() {
    let case = case("object-execution-loop-budget");
    let mut session =
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap();
    assert!(matches!(
        session.advance(&ExecutionBudget::default()),
        Err(EngineError::StepLimit)
    ));
    let boundary = session.export_envelope().unwrap();
    assert!(boundary.materialize().unwrap() == case["expected"][0]["semantic_state"]);
    assert_eq!(
        boundary.state["execution_context"]["stored"]["current_index"],
        500
    );
    let resumed = session.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(resumed.steps, 200);
    assert!(session.semantic_state().unwrap() == case["expected"][1]["semantic_state"]);
}

#[test]
fn loop_input_choices_are_private_and_each_iteration_issues_its_own_request() {
    let mut root = NativeSession::compile(source("object-execution-loop-input")).unwrap();
    let first = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let before = root.export_envelope().unwrap();
    let mut a = root.clone();
    let mut b = root.clone();
    for (worker, choice) in [(&mut a, 1), (&mut b, 2)] {
        let value = worker.import_selection(&json!(choice)).unwrap();
        worker.submit(first.id, value).unwrap();
        let next = worker
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        assert_ne!(
            worker.request_token(first.id),
            worker.request_token(next.id)
        );
        assert_eq!(
            worker.semantic_state().unwrap()["execution_context"]["seen"],
            json!([choice])
        );
    }
    assert!(root.export_envelope().unwrap().state == before.state);
    assert_ne!(a.semantic_state().unwrap(), b.semantic_state().unwrap());
}

#[test]
fn action_scope_push_restore_and_snapshot_rows_are_private_between_workers() {
    let root =
        NativeSession::compile(source("object-execution-scope-presence-all-fields")).unwrap();
    let before = root.export_envelope().unwrap();
    let mut a = root.clone();
    let mut b = root.clone();
    for (worker, card) in [(&mut a, "fork/a"), (&mut b, "fork/b")] {
        let mut values = Vec::new();
        for value in ["SKILL", card, "hero_hanu"] {
            let goa2_core::data::Datum::Text(value) =
                worker.import_selection(&json!(value)).unwrap()
            else {
                unreachable!()
            };
            values.push(value);
        }
        worker
            .push_action_context(values[0], values[1], values[2])
            .unwrap();
        assert_eq!(
            worker.semantic_state().unwrap()["execution_context"]["current_card_id"],
            card
        );
    }
    assert_ne!(a.semantic_state().unwrap(), b.semantic_state().unwrap());
    assert!(root.export_envelope().unwrap().state == before.state);
    a.advance(&ExecutionBudget::default()).unwrap();
    assert!(
        a.semantic_state().unwrap()["execution_context"]
            == before.materialize().unwrap()["execution_context"]
    );
    assert_eq!(
        b.semantic_state().unwrap()["execution_context"]["current_card_id"],
        "fork/b"
    );
}

#[test]
fn mandatory_abort_preserves_other_hero_boundary_before_restoring_outer_scope() {
    let case = case("object-execution-scope-abort-other-boundary");
    let mut session =
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap();
    session.advance_prefix(1).unwrap();
    let state = session.semantic_state().unwrap();
    assert!(state == case["expected"][0]["semantic_state"]);
    assert_eq!(
        state["execution_stack"].as_array().unwrap().last().unwrap()["other_hero_action"],
        true
    );
    assert_eq!(
        state["execution_context"]["action_context_stack"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    session.advance(&ExecutionBudget::default()).unwrap();
    assert!(session.semantic_state().unwrap() == case["expected"][1]["semantic_state"]);
}

#[test]
fn ordinary_and_arena_actor_and_owner_rules_match_the_same_source_boundaries() {
    for case in cases().into_iter().filter(|case| {
        case["name"]
            .as_str()
            .unwrap()
            .starts_with("object-execution-scope-owner-")
            || (case["name"]
                .as_str()
                .unwrap()
                .starts_with("object-execution-scope-actor-")
                && !case["name"]
                    .as_str()
                    .unwrap()
                    .starts_with("object-execution-scope-actor-swap-"))
    }) {
        let mut source: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        source.object_bindings.clear();
        let mut ordinary = NativeSession::compile(source).unwrap();
        let request = ordinary
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        assert_eq!(
            ordinary.request_json(&request)["player_id"],
            case["expected"][0]["request"]["player_id"],
            "{}",
            case["name"]
        );
        let state = ordinary.semantic_state().unwrap();
        assert_eq!(
            state["current_actor_id"],
            case["expected"][0]["semantic_state"]["current_actor_id"]
        );
        assert!(
            state["execution_context"]
                == case["expected"][0]["semantic_state"]["execution_context"]
        );
    }
}

#[test]
fn action_scope_helper_rejects_unbound_sessions_without_changing_rule_state() {
    let mut source = source("object-execution-scope-presence-all-fields");
    source.object_bindings.clear();
    source.state["execution_stack"] = json!([]);
    let mut session = NativeSession::compile(source).unwrap();
    let before = session.semantic_state().unwrap();
    let mut values = Vec::new();
    for value in ["SKILL", "nested", "hero_hanu"] {
        let goa2_core::data::Datum::Text(value) = session.import_selection(&json!(value)).unwrap()
        else {
            unreachable!()
        };
        values.push(value);
    }
    assert!(
        session
            .push_action_context(values[0], values[1], values[2])
            .unwrap_err()
            .to_string()
            .contains("live object bindings")
    );
    assert!(session.semantic_state().unwrap() == before);
}

#[test]
fn natural_expiry_returns_original_finishing_lists_even_for_repeated_effect_objects() {
    let mut session =
        NativeSession::compile(source("object-execution-effects-expired-finishing-True")).unwrap();
    let finishing = session
        .expire_effects(goa2_core::effects::Duration::ThisRound)
        .unwrap();
    assert_eq!(finishing.len(), 2);
    assert_eq!(finishing[0].1, finishing[1].1);
    let state = session.semantic_state().unwrap();
    assert!(state["active_effects"].as_array().unwrap().is_empty());
    assert_eq!(
        state["execution_context"]["old_effects"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(state["execution_context"]["card"]["is_active_base"], false);
    session.queue_finishing_steps(finishing[0].1).unwrap();
    session.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(
        session.semantic_state().unwrap()["execution_context"]["after_expiry"],
        true
    );
}

#[test]
fn card_scalar_mutation_and_copy_snapshots_remain_private_between_workers() {
    let mut root =
        NativeSession::compile(source("object-execution-effects-physical-copy-snapshot")).unwrap();
    root.advance_prefix(1).unwrap();
    let before = root.export_envelope().unwrap();
    let mut a = root.clone();
    let mut b = root.clone();
    let remove = |worker: &mut NativeSession, id: &str| {
        let goa2_core::data::Datum::Text(id) = worker.import_selection(&json!(id)).unwrap() else {
            unreachable!()
        };
        worker.remove_effects(&[id]).unwrap();
    };
    remove(&mut a, "first");
    a.advance_prefix(3).unwrap();
    remove(&mut a, "second");
    a.advance(&ExecutionBudget::default()).unwrap();
    let state = a.semantic_state().unwrap();
    assert_eq!(state["execution_context"]["card"]["is_active_base"], false);
    assert_eq!(
        state["execution_context"]["copies"][0]["is_active_base"],
        false
    );
    assert_eq!(
        state["execution_context"]["copies"][1]["is_active_base"],
        true
    );
    assert!(root.export_envelope().unwrap().state == before.state);
    b.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(
        b.semantic_state().unwrap()["active_effects"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn stored_finishing_input_remains_shared_after_the_effect_is_removed() {
    use goa2_core::codec::ObjectPathPart::{Index, Key};
    let mut session =
        NativeSession::compile(source("object-execution-effects-live-finishing-RESOLUTION"))
            .unwrap();
    session
        .queue_steps(&[
            Key("active_effects".into()),
            Index(0),
            Key("finishing_steps".into()),
        ])
        .unwrap();
    let request = session
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let goa2_core::data::Datum::Text(id) = session.import_selection(&json!("selected")).unwrap()
    else {
        unreachable!()
    };
    session.remove_effects(&[id]).unwrap();
    let state = session.semantic_state().unwrap();
    assert!(state["active_effects"].as_array().unwrap().is_empty());
    assert_eq!(
        state["execution_context"]["saved_effect"]["finishing_steps"][0]["pending_request_id"],
        session.request_token(request.id)
    );
    let choice = session.import_selection(&json!(1)).unwrap();
    session.submit(request.id, choice).unwrap();
    session.advance(&ExecutionBudget::default()).unwrap();
    assert!(
        session.semantic_state().unwrap()["execution_context"]["saved_effect"]["finishing_steps"]
            [0]["pending_request_id"]
            .is_null()
    );
}

#[test]
fn unsupported_finishing_programs_do_not_block_expiry_but_reject_queue_before_mutation() {
    let mut session =
        NativeSession::compile(source("object-execution-effects-expire-unknown-payload")).unwrap();
    let finishing = session
        .expire_effects(goa2_core::effects::Duration::ThisRound)
        .unwrap();
    assert_eq!(finishing.len(), 1);
    let before = session.semantic_state().unwrap();
    assert!(
        session
            .queue_finishing_steps(finishing[0].1)
            .unwrap_err()
            .to_string()
            .contains("not compiled")
    );
    assert!(session.semantic_state().unwrap() == before);
}

#[test]
fn ordinary_finishing_abort_markers_match_the_source_boundary_behavior() {
    for name in [
        "object-execution-effects-finishing-marker-plain",
        "object-execution-effects-finishing-marker-skipped-marker",
    ] {
        let case = case(name);
        let mut source: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        source.object_bindings.clear();
        let mut session = NativeSession::compile(source).unwrap();
        session.advance(&ExecutionBudget::default()).unwrap();
        assert!(
            session.semantic_state().unwrap()
                == case["expected"].as_array().unwrap().last().unwrap()["semantic_state"]
        );
    }
}

#[test]
fn turn_expiry_collection_is_read_only_and_retains_repeated_finishing_list_handles() {
    let mut session =
        NativeSession::compile(source("object-execution-scheduled-turn-expiry-same-object"))
            .unwrap();
    let before = session.export_envelope().unwrap();
    let expiry = session.collect_turn_expiry().unwrap();
    assert_eq!(expiry.ids.len(), 2);
    assert_eq!(expiry.ids[0], expiry.ids[1]);
    assert_eq!(expiry.finishing.len(), 2);
    assert_eq!(expiry.finishing[0].1, expiry.finishing[1].1);
    assert_eq!(session.export_envelope().unwrap().state, before.state);
    assert_eq!(
        session.export_envelope().unwrap().object_bindings,
        before.object_bindings
    );
    session.remove_effects(&expiry.ids).unwrap();
    for (_, list) in expiry.finishing {
        session.queue_finishing_steps(list).unwrap();
    }
    session.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(
        session.semantic_state().unwrap()["execution_context"]["count"],
        2
    );
}

#[test]
fn after_reveal_removes_only_trigger_rows_even_when_ids_collide() {
    let mut session = NativeSession::compile(source(
        "object-execution-scheduled-after-cards-duplicate-id",
    ))
    .unwrap();
    session.queue_after_cards_played().unwrap();
    let state = session.semantic_state().unwrap();
    assert_eq!(state["active_effects"].as_array().unwrap().len(), 1);
    assert_eq!(state["active_effects"][0]["effect_type"], "delayed_trigger");
    assert_eq!(
        state["execution_context"]["old_effects"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(state["execution_context"]["card"]["is_active_base"], true);
    session.advance(&ExecutionBudget::default()).unwrap();
    assert_eq!(
        session.semantic_state().unwrap()["execution_context"]["count"],
        1
    );
}

#[test]
fn generated_finishing_wrappers_and_stored_input_are_private_across_search_workers() {
    let root =
        NativeSession::compile(source("object-execution-scheduled-after-cards-input")).unwrap();
    let before = root.export_envelope().unwrap();
    let mut left = root.clone();
    let mut right = root.clone();
    for (session, choice) in [(&mut left, 1), (&mut right, 2)] {
        session.queue_after_cards_played().unwrap();
        let request = session
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let state = session.semantic_state().unwrap();
        assert_eq!(state["current_actor_id"], "hero_arien");
        assert_eq!(
            state["execution_context"]["trigger"]["finishing_steps"][0]["pending_request_id"],
            session.request_token(request.id)
        );
        let value = session.import_selection(&json!(choice)).unwrap();
        session.submit(request.id, value).unwrap();
        session.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(
            session.semantic_state().unwrap()["execution_context"]["selected"],
            choice
        );
    }
    assert_eq!(root.export_envelope().unwrap().state, before.state);
    assert_eq!(
        root.export_envelope().unwrap().object_bindings,
        before.object_bindings
    );
    assert_ne!(
        left.semantic_state().unwrap(),
        right.semantic_state().unwrap()
    );
}

#[test]
fn unavailable_due_payload_rejects_scheduling_before_removal_or_step_allocation() {
    let mut envelope = source("object-execution-effects-expire-unknown-payload");
    envelope.state["active_effects"][0]["effect_type"] = json!("after_cards_played_trigger");
    envelope.state["active_effects"][0]["created_at_round"] = envelope.state["round"].clone();
    envelope.state["active_effects"][0]["created_at_turn"] =
        json!(envelope.state["turn"].as_i64().unwrap() - 1);
    // This retained model is the same source object; update both serialized paths.
    let effect = envelope.state["active_effects"][0].clone();
    envelope.state["execution_context"]["old_effects"][0] = effect.clone();
    if envelope.state["execution_context"]
        .get("saved_effect")
        .is_some()
    {
        envelope.state["execution_context"]["saved_effect"] = effect;
    }
    let mut session = NativeSession::compile(envelope).unwrap();
    let before = session.export_envelope().unwrap();
    assert!(matches!(
        session.queue_after_cards_played(),
        Err(EngineError::Unsupported(_))
    ));
    let after = session.export_envelope().unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.object_bindings, before.object_bindings);
    assert_eq!(
        serde_json::to_value(after).unwrap(),
        serde_json::to_value(before).unwrap()
    );
}

#[test]
fn turn_finishing_keeps_rule_v9_effects_live_until_the_deferred_advance() {
    let mut session =
        NativeSession::compile(source("object-execution-turn-boundary-version-9-turn-1")).unwrap();
    session.end_turn().unwrap();
    let first = session.semantic_state().unwrap();
    assert_eq!(first["turn"], 1);
    assert_eq!(first["active_effects"].as_array().unwrap().len(), 1);
    assert_eq!(first["execution_context"]["card"]["is_active_base"], true);
    session.advance_prefix(3).unwrap();
    assert_eq!(session.semantic_state().unwrap()["turn"], 1);
    session.advance_prefix(1).unwrap();
    let final_state = session.semantic_state().unwrap();
    assert_eq!(final_state["phase"], "PLANNING");
    assert_eq!(final_state["turn"], 2);
    assert!(final_state["active_effects"].as_array().unwrap().is_empty());
    assert_eq!(
        final_state["execution_context"]["card"]["is_active_base"],
        false
    );
    assert_eq!(final_state["execution_context"]["count"], 1);
    assert!(
        final_state["turn_discard_log"]
            .as_object()
            .unwrap()
            .is_empty()
    );
    assert!(
        !final_state["execution_context"]["old_log"]
            .as_object()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn repeated_auto_pass_resolves_live_turn_slots_and_retains_the_original_played_list() {
    let mut session = NativeSession::compile(source(
        "object-execution-turn-boundary-auto-pass-all-current",
    ))
    .unwrap();
    session.end_turn().unwrap();
    let envelope = session.export_envelope().unwrap();
    let state = envelope.materialize().unwrap();
    assert_eq!(state["phase"], "CLEANUP");
    assert_eq!(state["turn"], 4);
    assert_eq!(
        state["execution_context"]["played"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(state["execution_context"]["played"][1]["state"], "RESOLVED");
    assert_eq!(state["execution_context"]["current"]["state"], "RESOLVED");
    assert!(
        envelope
            .object_bindings
            .iter()
            .any(|binding| binding.paths.contains(&vec![
                goa2_core::codec::ObjectPathPart::Key("execution_context".into()),
                goa2_core::codec::ObjectPathPart::Key("played".into())
            ]) && binding.paths.iter().any(|path| path.last()
                == Some(&goa2_core::codec::ObjectPathPart::Key(
                    "played_cards".into()
                ))))
    );
    let resumed = NativeSession::compile(envelope.clone()).unwrap();
    assert_eq!(
        resumed.export_envelope().unwrap().object_bindings,
        envelope.object_bindings
    );
}

#[test]
fn turn_snapshots_replace_the_dictionary_but_keep_the_original_hex_models() {
    let mut session =
        NativeSession::compile(source("object-execution-turn-boundary-no-effects-1")).unwrap();
    session.end_turn().unwrap();
    let envelope = session.export_envelope().unwrap();
    let bindings = &envelope.object_bindings;
    assert!(
        bindings
            .iter()
            .any(|binding| binding.paths.iter().any(|path| path.first()
                == Some(&goa2_core::codec::ObjectPathPart::Key(
                    "entity_locations".into()
                ))
                && path.len() == 2)
                && binding.paths.iter().any(|path| path.first()
                    == Some(&goa2_core::codec::ObjectPathPart::Key(
                        "last_turn_positions".into()
                    ))
                    && path.len() == 2))
    );
    assert!(!bindings.iter().any(|binding| binding.paths.contains(&vec![
        goa2_core::codec::ObjectPathPart::Key("last_turn_positions".into())
    ]) && binding.paths.contains(&vec![
        goa2_core::codec::ObjectPathPart::Key("execution_context".into()),
        goa2_core::codec::ObjectPathPart::Key("old_snapshot".into())
    ])));
}

#[test]
fn deferred_turn_input_and_generation_state_are_private_between_workers() {
    let mut root =
        NativeSession::compile(source("object-execution-turn-boundary-pending-input")).unwrap();
    root.end_turn().unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let before = root.export_envelope().unwrap();
    for choice in [1, 2] {
        let mut worker = root.clone();
        let value = worker.import_selection(&json!(choice)).unwrap();
        worker.submit(request.id, value).unwrap();
        worker.advance(&ExecutionBudget::default()).unwrap();
        let state = worker.semantic_state().unwrap();
        assert_eq!(state["turn"], 3);
        assert_eq!(state["execution_context"]["selected"], choice);
        assert!(state["active_effects"].as_array().unwrap().is_empty());
    }
    assert_eq!(root.export_envelope().unwrap().state, before.state);
    assert_eq!(
        root.export_envelope().unwrap().object_bindings,
        before.object_bindings
    );
}

#[test]
fn unbound_round_consumers_reject_before_popping_the_live_step_or_mutating_state() {
    let mut session =
        NativeSession::compile(source("object-execution-turn-boundary-no-effects-4")).unwrap();
    session.end_turn().unwrap();
    session.advance_prefix(1).unwrap();
    let before = session.export_envelope().unwrap();
    let mut restored = NativeSession::compile(before.clone()).unwrap();
    for worker in [&mut session, &mut restored] {
        assert!(
            matches!(worker.advance(&ExecutionBudget::default()), Err(EngineError::Unsupported(message)) if message.contains("battle/cleanup"))
        );
        let after = worker.export_envelope().unwrap();
        assert_eq!(
            serde_json::to_value(after).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
    }
}

#[test]
fn end_turn_generation_exhaustion_rejects_before_resetting_the_log_or_effects() {
    let mut envelope = source("object-execution-turn-boundary-flag");
    envelope.native_resume = Some(goa2_core::codec::NativeResume {
        version: 1,
        request_high_water: 0,
        step_high_water: u64::MAX - 2,
    });
    let mut session = NativeSession::compile(envelope).unwrap();
    let before = session.export_envelope().unwrap();
    assert!(
        matches!(session.end_turn(), Err(EngineError::Unsupported(message)) if message.contains("counter exhausted"))
    );
    assert_eq!(
        serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
}
