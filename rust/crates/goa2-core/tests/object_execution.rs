//! Live source identity, restore, worker isolation and execution boundaries.
#[path = "../../../test-support/fixture_cache.rs"]
mod fixture_cache;
use goa2_core::{
    codec::StateEnvelope,
    data::Symbols,
    interpreter::{EngineError, ExecutionBudget, NativeSession},
    world::NativeWorld,
};
use serde_json::{Value, json};
use std::{sync::atomic::AtomicBool, time::Instant};
#[path = "../../../test-support/parallel.rs"]
mod fixture_parallel;

fn cases() -> impl Iterator<Item = Value> {
    cases_matching(|_| true)
}
fn cases_matching(predicate: impl Fn(&str) -> bool) -> impl Iterator<Item = Value> {
    let base = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
    let index: Value = serde_json::from_reader(std::io::BufReader::new(
        fixture_cache::open(base.join("object-execution.json")).unwrap(),
    ))
    .unwrap();
    // Keep only the small index in memory; full state/identity traces can be
    // large, and each test needs just one source case at a time.
    index["cases"]
        .as_array()
        .unwrap()
        .clone()
        .into_iter()
        .filter(move |entry| predicate(entry["name"].as_str().unwrap()))
        .map(move |entry| {
            serde_json::from_reader(std::io::BufReader::new(
                fixture_cache::open(base.join(entry["path"].as_str().unwrap())).unwrap(),
            ))
            .unwrap()
        })
}
fn case(name: &str) -> Value {
    let base = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
    let index: Value = serde_json::from_reader(std::io::BufReader::new(
        fixture_cache::open(base.join("object-execution.json")).unwrap(),
    ))
    .unwrap();
    let entry = index["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["name"] == name)
        .unwrap();
    serde_json::from_reader(std::io::BufReader::new(
        fixture_cache::open(base.join(entry["path"].as_str().unwrap())).unwrap(),
    ))
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
        "commit" | "uncommit" | "pass" | "finish" => {
            let goa2_core::data::Datum::Text(hero) =
                session.import_selection(&operation["hero"]).unwrap()
            else {
                panic!("planning hero must be text");
            };
            let result = match operation["kind"].as_str().unwrap() {
                "commit" => session.commit_card(
                    hero,
                    goa2_core::cards::CardId(operation["card"].as_u64().unwrap() as u32),
                ),
                "uncommit" => session.uncommit_card(hero),
                "pass" => session.pass_planning(hero),
                "finish" => session.finish_planning(hero),
                _ => unreachable!(),
            };
            match result {
                Ok(()) => {}
                Err(EngineError::InvalidInput(_)) => error = json!("invalid_planning_action"),
                Err(error) => panic!("{error}"),
            }
        }
        "start_revelation" => {
            session.start_revelation().unwrap();
        }
        "next_actor" => {
            session.resolve_next_actor().unwrap();
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    let operations = AtomicUsize::new(0);
    let checked = fixture_parallel::for_each_case(cases(), |case| {
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
                restored.export_envelope().unwrap().board_keys,
                envelope.board_keys
            );
            assert_eq!(
                restored.export_envelope().unwrap().object_bindings,
                envelope.object_bindings
            );
            assert!(
                apply(&mut session, op) == apply(&mut restored, op),
                "{}: {op}",
                case["name"]
            );
            operations.fetch_add(1, Ordering::Relaxed);
        }
    });
    assert_eq!(checked, 14255);
    assert_eq!(operations.load(Ordering::Relaxed), 74472);
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
fn legacy_dense_import_and_setup_phase_remain_guarded_for_shared_worlds() {
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
    let mut checked = 0;
    for case in cases_matching(|name| {
        name.starts_with("object-execution-scope-owner-")
            || (name.starts_with("object-execution-scope-actor-")
                && !name.starts_with("object-execution-scope-actor-swap-"))
    }) {
        checked += 1;
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
    assert!(checked > 0);
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

#[test]
fn revelation_keeps_known_frozen_record_identity_and_detaches_the_source_buffers() {
    let mut session =
        NativeSession::compile(source("object-execution-revelation-known-card")).unwrap();
    session.start_revelation().unwrap();
    let envelope = session.export_envelope().unwrap();
    let state = envelope.materialize().unwrap();
    assert!(state["pending_inputs"].as_object().unwrap().is_empty());
    assert_eq!(state["public_revealed_cards"].as_array().unwrap().len(), 4);
    assert!(envelope.object_bindings.iter().any(|binding| {
        binding.paths.contains(
            &vec!["execution_context", "known"]
                .into_iter()
                .map(|key| goa2_core::codec::ObjectPathPart::Key(key.into()))
                .collect(),
        ) && binding.paths.iter().any(|path| {
            path.first()
                == Some(&goa2_core::codec::ObjectPathPart::Key(
                    "public_revealed_cards".into(),
                ))
        })
    }));
    assert!(
        !envelope
            .object_bindings
            .iter()
            .any(|binding| binding
                .paths
                .contains(&vec![goa2_core::codec::ObjectPathPart::Key(
                    "public_revealed_cards".into()
                )]))
    );
    assert!(
        state["execution_context"]["old_pending"]
            .as_object()
            .unwrap()
            .values()
            .all(|card| card["is_facedown"] == false)
    );
}

#[test]
fn dual_retrieval_workers_mutate_the_same_hand_only_within_their_private_fork() {
    let mut root =
        NativeSession::compile(source("object-execution-revelation-dual-first")).unwrap();
    root.start_revelation().unwrap();
    let request = root
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    let before = root.export_envelope().unwrap();
    let state = before.materialize().unwrap();
    for name in ["first", "second"] {
        let mut worker = root.clone();
        let selection = worker
            .import_selection(&state["execution_context"][name]["id"])
            .unwrap();
        worker.submit(request.id, selection).unwrap();
        let result = worker.advance_prefix(1).unwrap();
        assert_eq!(result.events.len(), 1);
        let after = worker.semantic_state().unwrap();
        assert_eq!(after["execution_context"][name]["state"], "HAND");
        assert_eq!(
            after["execution_context"]["hand"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["id"],
            state["execution_context"][name]["id"]
        );
        if name == "first" {
            assert!(
                after["execution_context"][name]
                    .get("initiative_override")
                    .is_none()
            );
        }
    }
    assert_eq!(root.export_envelope().unwrap().state, before.state);
    assert_eq!(
        root.export_envelope().unwrap().object_bindings,
        before.object_bindings
    );
}

#[test]
fn global_reverse_initiative_matches_source_activation_including_token_passive_effects() {
    for (variant, hero) in [
        ("reversed", "hero_tigerclaw"),
        ("token-passive", "hero_tigerclaw"),
        ("inactive", "hero_arien"),
        ("stale", "hero_arien"),
        ("cross-round", "hero_arien"),
    ] {
        let mut session = NativeSession::compile(source(&format!(
            "object-execution-revelation-rank-{variant}"
        )))
        .unwrap();
        session.advance_prefix(1).unwrap();
        assert_eq!(session.semantic_state().unwrap()["current_actor_id"], hero);
    }
}

#[test]
fn rejected_tie_input_reissues_without_installing_a_wrong_team_actor_or_flipping_coin() {
    let mut session =
        NativeSession::compile(source("object-execution-revelation-tie-wrong-choice")).unwrap();
    let first = session.advance_prefix(1).unwrap().request.unwrap();
    assert_eq!(session.request_json(&first)["player_id"], "team:RED");
    let wrong = session.import_selection(&json!("hero_brogan")).unwrap();
    session.submit(first.id, wrong).unwrap();
    let next = session.advance_prefix(1).unwrap().request.unwrap();
    assert_ne!(
        session.request_token(first.id),
        session.request_token(next.id)
    );
    let chosen = session.import_selection(&json!("hero_arien")).unwrap();
    session.submit(next.id, chosen).unwrap();
    session.advance_prefix(1).unwrap();
    let state = session.semantic_state().unwrap();
    assert_eq!(state["current_actor_id"], "hero_arien");
    assert_eq!(state["tie_breaker_team"], "RED");
    assert!(state["execution_context"]["tie"]["pending_input"].is_null());
    assert!(
        state["execution_stack"]
            .as_array()
            .unwrap()
            .iter()
            .any(|step| step["type"] == "flip_tie_breaker_coin")
    );
}

#[test]
fn retrieval_omits_cleared_override_without_mutating_copy_or_finished_step_payload() {
    let mut session =
        NativeSession::compile(source("object-execution-revelation-retrieve-override")).unwrap();
    session.advance_prefix(1).unwrap();
    let state = session.semantic_state().unwrap();
    assert!(
        state["execution_context"]["first"]
            .get("initiative_override")
            .is_none()
    );
    assert_eq!(
        state["execution_context"]["card_copy"]["initiative_override"],
        30
    );
    assert_eq!(
        state["execution_context"]["step"]["pending_input"]["selection"],
        state["execution_context"]["first"]["id"]
    );
    assert_eq!(
        state["execution_context"]["step"]["pending_request_id"],
        "fixture/dirty-retrieval"
    );
    let restored = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
    assert_eq!(restored.semantic_state().unwrap(), state);
}

#[test]
fn revealed_actor_opens_the_same_action_menu_after_restore() {
    let mut session = NativeSession::compile(source("object-execution-revelation-basic")).unwrap();
    session.start_revelation().unwrap();
    let mut restored = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
    let operation = json!({"kind": "advance"});
    let result = apply(&mut session, &operation);
    assert_eq!(result, apply(&mut restored, &operation));
    assert_eq!(result["request"]["request_type"], "CHOOSE_ACTION");
    assert!(result["request"]["options"].as_array().unwrap().len() > 1);
}

#[test]
fn live_planning_pass_detaches_requests_but_retains_pending_and_rows() {
    let fixture = case("object-execution-planning-command-pass-requests");
    let mut session =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let result = apply(&mut session, &fixture["operations"][0]);
    assert!(result["error"].is_null());
    let state = &result["semantic_state"];
    assert_eq!(
        state["starting_position_requests"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        state["execution_context"]["requests"]
            .as_object()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        state["starting_position_requests"]["keep"],
        state["execution_context"]["kept_request"]
    );
    assert_eq!(
        state["pending_inputs"],
        state["execution_context"]["pending"]
    );
    // The general boundary test also restores this boundary and compares all
    // instance and aggregate bindings before applying the next operation.
}

#[test]
fn live_planning_errors_leave_state_untouched_and_finish_is_idempotent() {
    for name in [
        "pass-nonempty",
        "pass-unknown",
        "pass-duplicate",
        "pass-wrong-phase",
        "finish-missing",
    ] {
        let fixture = case(&format!("object-execution-planning-command-{name}"));
        let mut session =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let before = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
        assert_eq!(
            apply(&mut session, &fixture["operations"][0])["error"],
            "invalid_planning_action"
        );
        assert_eq!(
            serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
            before
        );
    }
    let fixture = case("object-execution-planning-command-finish");
    let mut session =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    apply(&mut session, &fixture["operations"][0]);
    let once = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
    apply(&mut session, &fixture["operations"][1]);
    assert_eq!(
        serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
        once
    );
}

#[test]
fn confirmation_workers_keep_choices_request_ids_and_shared_step_state_private() {
    let mut root = NativeSession::compile(source("object-execution-confirmation-basic")).unwrap();
    root.advance(&ExecutionBudget::default()).unwrap();
    let token = root.last_request.as_ref().unwrap().id;
    let before = serde_json::to_value(root.export_envelope().unwrap()).unwrap();
    let mut confirmed = root.clone();
    let mut retry = root.clone();
    for (worker, selection) in [(&mut confirmed, "CONFIRM"), (&mut retry, "NO")] {
        let selection = worker.import_selection(&json!(selection)).unwrap();
        worker.submit(token, selection).unwrap();
    }
    assert!(
        confirmed
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .is_none()
    );
    let request = retry
        .advance(&ExecutionBudget::default())
        .unwrap()
        .request
        .unwrap();
    assert_ne!(retry.request_token(request.id), root.request_token(token));
    let state = retry.semantic_state().unwrap();
    assert_eq!(
        state["execution_context"]["confirmation"],
        state["execution_context"]["same_step"]
    );
    assert_eq!(
        state["execution_stack"][0],
        state["execution_context"]["confirmation"]
    );
    assert_eq!(
        serde_json::to_value(root.export_envelope().unwrap()).unwrap(),
        before
    );
}

#[test]
fn malformed_confirmation_input_rejects_before_pop_unless_rollback_auto_confirms() {
    use goa2_core::codec::{ObjectKind, ObjectPathPart};
    let mut envelope = source("object-execution-confirmation-basic");
    let paths: Vec<_> = envelope
        .object_bindings
        .iter()
        .filter(|binding| binding.kind == ObjectKind::Step)
        .flat_map(|binding| binding.paths.clone())
        .collect();
    for path in paths {
        let mut value = &mut envelope.state;
        for part in path {
            value = match part {
                ObjectPathPart::Key(key) => &mut value[key],
                ObjectPathPart::Index(index) => &mut value[index],
            };
        }
        value["pending_input"] = json!(1);
    }
    envelope.validate().unwrap();
    let mut session = NativeSession::compile(envelope.clone()).unwrap();
    let before = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
    let error = session.advance(&ExecutionBudget::default()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("confirmation input must be a mapping")
    );
    assert_eq!(
        serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
        before
    );
    envelope.state["execution_context"]["rollback_frozen"] = json!(true);
    let mut frozen = NativeSession::compile(envelope).unwrap();
    assert!(
        frozen
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .is_none()
    );
    assert_eq!(
        frozen.semantic_state().unwrap()["execution_context"]["confirmation"]["pending_input"],
        1
    );
}

#[test]
fn ordinary_and_live_confirmation_keep_the_same_prompt_and_active_step_input_state() {
    let mut names = vec![
        "basic".to_owned(),
        "retry".to_owned(),
        "stale".to_owned(),
        "retained-valid".to_owned(),
        "retained-invalid".to_owned(),
        "empty-mapping".to_owned(),
        "empty-list".to_owned(),
    ];
    for flag in ["rollback_frozen", "rollback_reanchor_pending"] {
        for index in 0..6 {
            names.push(format!("{flag}-{index}"));
        }
    }
    for name in names {
        let fixture = case(&format!("object-execution-confirmation-{name}"));
        let mut envelope: StateEnvelope =
            serde_json::from_value(fixture["envelope"].clone()).unwrap();
        let mut live = NativeSession::compile(envelope.clone()).unwrap();
        envelope.object_bindings.clear();
        let mut ordinary = NativeSession::compile(envelope).unwrap();
        for operation in fixture["operations"].as_array().unwrap() {
            let live = apply(&mut live, operation);
            let ordinary = apply(&mut ordinary, operation);
            assert_eq!(live["request"], ordinary["request"], "{name}: {operation}");
            assert_eq!(live["error"], ordinary["error"], "{name}: {operation}");
            assert_eq!(
                live["semantic_state"]["execution_stack"],
                ordinary["semantic_state"]["execution_stack"],
                "{name}: {operation}"
            );
        }
    }
}

#[test]
fn finalization_workers_keep_repeated_slots_continuations_and_context_private() {
    let root = NativeSession::compile(source("object-execution-finalization-repeated")).unwrap();
    let before = serde_json::to_value(root.export_envelope().unwrap()).unwrap();
    let mut one = root.clone();
    let mut two = root.clone();
    for worker in [&mut one, &mut two] {
        let result = worker.advance_prefix(1).unwrap();
        assert_eq!(result.events.len(), 1);
        assert!(
            worker.semantic_state().unwrap()["execution_context"]
                .as_object()
                .unwrap()
                .is_empty()
        );
    }
    let path: Vec<goa2_core::codec::ObjectPathPart> =
        serde_json::from_value(json!(["active_effects", 0, "finishing_steps"])).unwrap();
    two.queue_steps(&path).unwrap();
    two.advance_prefix(1).unwrap();
    assert_eq!(
        one.semantic_state().unwrap()["execution_stack"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        two.semantic_state().unwrap()["execution_stack"]
            .as_array()
            .unwrap()
            .len(),
        6
    );
    assert_eq!(
        one.semantic_state().unwrap()["teams"],
        two.semantic_state().unwrap()["teams"]
    );
    assert_eq!(
        serde_json::to_value(root.export_envelope().unwrap()).unwrap(),
        before
    );
}

#[test]
fn minion_walking_executes_after_restore_and_retains_spatial_aliases() {
    let mut session =
        NativeSession::compile(source("object-execution-minion-return-walk")).unwrap();
    session.advance_prefix(1).unwrap();
    let mut restored = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
    let actual = apply(&mut session, &json!({"kind":"advance"}));
    assert_eq!(actual, apply(&mut restored, &json!({"kind":"advance"})));
    assert_eq!(actual["events"][0]["event_type"], "UNIT_MOVED");
    assert!(
        actual["semantic_state"]["execution_stack"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn minion_destination_workers_keep_team_choices_and_retained_input_private() {
    let mut root = NativeSession::compile(source("object-execution-minion-return-choice")).unwrap();
    root.advance_prefix(1).unwrap();
    let before = root.export_envelope().unwrap();
    let token = root.last_request.as_ref().unwrap().id;
    let mut a = root.clone();
    let mut b = root.clone();
    for (worker, name) in [(&mut a, "choice"), (&mut b, "choice-last")] {
        let fixture = case(&format!("object-execution-minion-return-{name}"));
        let chosen = &fixture["operations"][1]["selection"];
        let value = worker.import_selection(chosen).unwrap();
        worker.submit(token, value).unwrap();
        worker.advance_prefix(1).unwrap();
        let state = worker.semantic_state().unwrap();
        assert_eq!(&state["execution_stack"][0]["target_hex_arg"], chosen);
        assert_eq!(
            &state["execution_context"]["return_step"]["pending_input"]["selection"],
            chosen
        );
    }
    assert_ne!(a.semantic_state().unwrap(), b.semantic_state().unwrap());
    let after = root.export_envelope().unwrap();
    assert_eq!(before.state, after.state);
    assert_eq!(before.object_bindings, after.object_bindings);
}

#[test]
fn ordinary_and_live_piece_planning_keep_owner_cards_and_raw_buffers_identical() {
    for name in ["cycle", "pass", "last", "buffers"] {
        let fixture = case(&format!("object-execution-piece-planning-{name}"));
        let mut envelope: StateEnvelope =
            serde_json::from_value(fixture["envelope"].clone()).unwrap();
        let mut live = NativeSession::compile(envelope.clone()).unwrap();
        envelope.object_bindings.clear();
        let mut ordinary = NativeSession::compile(envelope).unwrap();
        for operation in fixture["operations"].as_array().unwrap() {
            let live = apply(&mut live, operation);
            let ordinary = apply(&mut ordinary, operation);
            assert_eq!(live["error"], ordinary["error"], "{name}: {operation}");
            for field in [
                "teams",
                "pending_inputs",
                "pending_second_cards",
                "planning_done",
                "phase",
                "current_actor_id",
                "resolution_owner_id",
                "unresolved_hero_ids",
            ] {
                assert_eq!(
                    live["semantic_state"][field], ordinary["semantic_state"][field],
                    "{name}: {field}: {operation}"
                );
            }
        }
    }
}

#[test]
fn minion_placement_fallback_executes_and_restores_dense_and_arena_positions() {
    for name in ["fallback", "cap-zero", "cap-one"] {
        let fixture = case(&format!("object-execution-placement-minion-return-{name}"));
        let mut root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        root.advance_prefix(1).unwrap();
        let mut restored = NativeSession::compile(root.export_envelope().unwrap()).unwrap();
        let actual = apply(&mut root, &json!({"kind":"advance_prefix","steps":1}));
        assert_eq!(
            actual,
            apply(&mut restored, &json!({"kind":"advance_prefix","steps":1}))
        );
        assert_eq!(actual["events"][0]["event_type"], "UNIT_PLACED");
        assert!(
            actual["semantic_state"]["execution_stack"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn live_discard_private_equality_guards_before_pop_and_preserves_root_clone() {
    let root = NativeSession::compile(source("object-execution-discard-equal-current")).unwrap();
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    worker.advance_prefix(1).unwrap();
    let state = worker.semantic_state().unwrap();
    let hero = &state["teams"]["RED"]["heroes"][0];
    assert!(hero["current_turn_card"].is_null());
    assert_eq!(hero["played_cards"][0]["state"], "DISCARD");
    assert_eq!(root.export_envelope().unwrap().state, before.state);
    let after = worker.export_envelope().unwrap();
    let mut restored = NativeSession::compile(after.clone()).unwrap();
    assert_eq!(restored.export_envelope().unwrap().state, after.state);
    worker.advance_prefix(1).unwrap();
    restored.advance_prefix(1).unwrap();
    assert!(
        worker.semantic_state().unwrap()["execution_stack"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        worker.export_envelope().unwrap().state,
        restored.export_envelope().unwrap().state
    );
    assert_eq!(
        worker.export_envelope().unwrap().object_bindings,
        restored.export_envelope().unwrap().object_bindings
    );
    let mut missing = before.clone();
    for instance in &mut missing.instances {
        instance.equality = None;
    }
    let mut guarded = NativeSession::compile(missing.clone()).unwrap();
    assert!(
        guarded
            .advance_prefix(1)
            .unwrap_err()
            .to_string()
            .contains("private equality metadata")
    );
    assert_eq!(guarded.export_envelope().unwrap().state, missing.state);
    assert_eq!(
        guarded.export_envelope().unwrap().object_bindings,
        missing.object_bindings
    );
}

#[test]
fn live_passive_movement_macro_preserves_pending_aliases_across_restore() {
    let root = NativeSession::compile(source(
        "object-execution-passive-offer-living_tsunami-before_attack-request",
    ))
    .unwrap();
    let root_before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let request = worker.advance_prefix(1).unwrap().request.unwrap();
    let yes = worker.import_selection(&json!("YES")).unwrap();
    worker.submit(request.id, yes).unwrap();
    worker.advance_prefix(1).unwrap();
    let before = worker.export_envelope().unwrap();
    assert_eq!(
        before.state["execution_stack"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["type"],
        "move_sequence"
    );
    let mut restored = NativeSession::compile(before.clone()).unwrap();
    worker.advance_prefix(1).unwrap();
    restored.advance_prefix(1).unwrap();
    assert_eq!(
        worker.export_envelope().unwrap().state,
        restored.export_envelope().unwrap().state
    );
    assert_eq!(
        worker.export_envelope().unwrap().object_bindings,
        restored.export_envelope().unwrap().object_bindings
    );
    assert_eq!(root.export_envelope().unwrap().state, root_before.state);
}

#[test]
fn accepted_rune_body_and_usage_are_private_to_each_search_worker() {
    let root = NativeSession::compile(source(
        "object-execution-passive-body-rune_mastery-before_action-many-runes",
    ))
    .unwrap();
    let root_before = root.export_envelope().unwrap();
    let mut first = root.clone();
    let mut second = root.clone();
    first.advance_prefix(1).unwrap();
    second.advance_prefix(1).unwrap();
    let second_before = second.export_envelope().unwrap();
    let request = first.advance_prefix(1).unwrap().request.unwrap();
    let choice = first.import_selection(&json!("bird")).unwrap();
    first.submit(request.id, choice).unwrap();
    first.advance(&ExecutionBudget::default()).unwrap();
    let state = first.semantic_state().unwrap();
    assert_eq!(state["execution_context"]["snorri_ult_rune_action"], "bird");
    assert_eq!(
        state["teams"]["RED"]["heroes"][0]["ultimate_card"]["passive_uses_this_turn"],
        1
    );
    assert_eq!(second.export_envelope().unwrap().state, second_before.state);
    assert_eq!(
        second.export_envelope().unwrap().object_bindings,
        second_before.object_bindings
    );
    assert_eq!(root.export_envelope().unwrap().state, root_before.state);
}

#[test]
fn automatic_hex_keys_preserve_private_identity_across_restore_and_worker_forks() {
    for variant in ["tile-key", "private-key", "visible-key"] {
        let name = format!("object-execution-hex-identity-{variant}");
        let root = NativeSession::compile(source(&name)).unwrap();
        let before = root.export_envelope().unwrap();
        assert_eq!(
            before.board_keys.as_ref().unwrap()[0].path.is_none(),
            variant == "private-key"
        );
        let mut worker = root.clone();
        let mut restored = NativeSession::compile(before.clone()).unwrap();
        worker.advance_prefix(1).unwrap();
        restored.advance_prefix(1).unwrap();
        let after = worker.export_envelope().unwrap();
        assert_eq!(after.state, restored.export_envelope().unwrap().state);
        assert_eq!(
            after.object_bindings,
            restored.export_envelope().unwrap().object_bindings
        );
        assert_eq!(
            after.board_keys,
            restored.export_envelope().unwrap().board_keys
        );
        let chosen = serde_json::from_value(json!(["execution_context", "chosen"])).unwrap();
        let tile = serde_json::from_value(json!(["board", "tiles", "(-4, 0, 4)", "hex"])).unwrap();
        let binding = after
            .object_bindings
            .iter()
            .find(|binding| binding.paths.contains(&chosen))
            .unwrap();
        assert_eq!(binding.paths.contains(&tile), variant == "tile-key");
        assert!(after.board_keys.as_ref().unwrap()[0].path.is_some());
        let again = NativeSession::compile(after.clone())
            .unwrap()
            .export_envelope()
            .unwrap();
        assert_eq!(after.object_bindings, again.object_bindings);
        assert_eq!(after.board_keys, again.board_keys);
        assert_eq!(root.export_envelope().unwrap().state, before.state);
        assert_eq!(
            root.export_envelope().unwrap().object_bindings,
            before.object_bindings
        );
    }
}

#[test]
fn missing_or_malformed_hex_key_identity_is_rejected_without_popping() {
    let valid = source("object-execution-hex-identity-tile-key");
    let mut missing = valid.clone();
    missing.board_keys = None;
    let mut session = NativeSession::compile(missing.clone()).unwrap();
    assert!(
        session
            .advance_prefix(1)
            .unwrap_err()
            .to_string()
            .contains("board-key")
    );
    let after = session.export_envelope().unwrap();
    assert_eq!(after.state, missing.state);
    assert_eq!(after.object_bindings, missing.object_bindings);
    let mut bad = valid.clone();
    bad.board_keys.as_mut().unwrap().clear();
    assert!(bad.validate().unwrap_err().contains("count mismatch"));
    let mut bad = valid.clone();
    bad.board_keys.as_mut().unwrap()[0].hex = goa2_core::geometry::Hex::new(0, 0, 0).unwrap();
    assert!(bad.validate().unwrap_err().contains("coordinates mismatch"));
    let mut bad = valid.clone();
    bad.board_keys.as_mut().unwrap()[0].path =
        Some(serde_json::from_value(json!(["execution_context", "chosen"])).unwrap());
    assert!(bad.validate().unwrap_err().contains("alias coordinates"));
    let mut bad = valid;
    let path = bad.board_keys.as_ref().unwrap()[0]
        .path
        .as_ref()
        .unwrap()
        .clone();
    let binding = bad
        .object_bindings
        .iter_mut()
        .find(|binding| binding.paths.contains(&path))
        .unwrap();
    binding.kind = goa2_core::codec::ObjectKind::Dict;
    binding.deepcopy_shared = false;
    assert!(
        NativeSession::compile(bad)
            .unwrap_err()
            .to_string()
            .contains("immutable Hex")
    );
}

#[test]
fn live_unit_selection_keeps_worker_choices_and_pending_aliases_private() {
    let mut root =
        NativeSession::compile(source("object-execution-unit-world-plain-unit")).unwrap();
    let request = root.advance_prefix(1).unwrap().request.unwrap();
    let before = root.export_envelope().unwrap();
    let mut left = root.clone();
    let mut right = root.clone();
    for (worker, choice) in [(&mut left, "hero_mrak"), (&mut right, "hero_hanu")] {
        let selected = worker.import_selection(&json!(choice)).unwrap();
        worker.submit(request.id, selected).unwrap();
        worker.advance_prefix(1).unwrap();
        let after = worker.export_envelope().unwrap();
        assert_eq!(after.state["execution_context"]["chosen"], choice);
        assert!(after.state["execution_context"]["selector"]["pending_input"].is_null());
        assert!(after.state["execution_context"]["selector"]["pending_request_id"].is_null());
        let restored = NativeSession::compile(after.clone())
            .unwrap()
            .export_envelope()
            .unwrap();
        assert_eq!(after.state, restored.state);
        assert_eq!(after.object_bindings, restored.object_bindings);
    }
    assert_ne!(
        left.semantic_state().unwrap(),
        right.semantic_state().unwrap()
    );
    assert_eq!(root.export_envelope().unwrap().state, before.state);
    assert_eq!(
        root.export_envelope().unwrap().object_bindings,
        before.object_bindings
    );
}

#[test]
fn unbound_live_unit_filter_leaves_stack_and_aliases_unchanged() {
    let mut source = source("object-execution-unit-filter-plain-0");
    source.state["execution_stack"][0]["filters"][0]["type"] = json!("unbound_unit_fixture");
    source.state["execution_context"]["selector"]["filters"][0]["type"] =
        json!("unbound_unit_fixture");
    source.state["execution_context"]["filters_alias"][0]["type"] = json!("unbound_unit_fixture");
    let mut session = NativeSession::compile(source).unwrap();
    let before = session.export_envelope().unwrap();
    assert!(
        session
            .advance_prefix(1)
            .unwrap_err()
            .to_string()
            .contains("live unit filter not bound")
    );
    let after = session.export_envelope().unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.object_bindings, before.object_bindings);
}

#[test]
fn nested_primary_filters_bind_the_card_used_for_fast_travel_color_exceptions() {
    use goa2_core::{
        data::Registers,
        primary::{PrimaryOperands, PrimaryValues},
        spatial::{SpatialContext, compile_filter},
        stats::CardStats,
        targeting::UnitFilter,
    };
    let source = case("object-execution-hex-travel-source-gold");
    let mut envelope: StateEnvelope = serde_json::from_value(source["envelope"].clone()).unwrap();
    // This rule-only probe does not execute the source selector's arena graph.
    envelope.object_bindings.clear();
    envelope.board_keys = None;
    envelope.state["execution_context"] = json!({});
    envelope.state["execution_stack"] = json!([]);
    let mut symbols = Symbols::default();
    let world = NativeWorld::compile(&envelope, &mut symbols).unwrap();
    let slots = PrimaryOperands::compile(&mut symbols).unwrap();
    let values = PrimaryValues {
        hero: symbols.intern("hero_arien"),
        card: symbols.intern("noble_blade"),
        name: symbols.intern("Arien"),
        stats: CardStats {
            primary_value: 0,
            range: 0,
            radius: None,
        },
        ranged: false,
    };
    let mut filter=UnitFilter::And(vec![UnitFilter::Spatial(compile_filter(&json!({"type":"fast_travel_destination_filter","unit_id":goa2_core::primary::HERO,"source_card_id":goa2_core::primary::CARD,"require_zone_change":false}),&mut symbols).unwrap())]);
    slots.unit_filter(&mut filter, &values).unwrap();
    let UnitFilter::And(children) = filter else {
        unreachable!()
    };
    let UnitFilter::Spatial(filter) = &children[0] else {
        unreachable!()
    };
    let coordinates = ["q", "r", "s"].map(|name| symbols.intern(name));
    let registers = Registers::default();
    let spatial = SpatialContext {
        count_observer: None,
        world: &world,
        registers: &registers,
        symbols: &symbols,
        actor: Some(values.hero),
        frame: None,
        coordinates,
        range_override: None,
        through_override: None,
    };
    let mut accepted = vec![true; world.tile_hexes().len()];
    filter
        .apply_scan(&spatial, world.tile_hexes(), &mut accepted)
        .unwrap();
    let expected = source["expected"][0]["request"]["options"]
        .as_array()
        .unwrap()
        .len();
    assert!(expected > 0);
    assert_eq!(accepted.into_iter().filter(|yes| *yes).count(), expected);
}

#[test]
fn query_result_lists_and_history_are_private_between_search_workers() {
    let mut root =
        NativeSession::compile(source("object-execution-query-live-collect-repeat")).unwrap();
    root.advance_prefix(1).unwrap();
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let mut restored = NativeSession::compile(before.clone()).unwrap();
    for session in [&mut worker, &mut restored] {
        session.advance_prefix(3).unwrap();
    }
    let after = worker.export_envelope().unwrap();
    assert_eq!(after.state, restored.export_envelope().unwrap().state);
    assert_eq!(
        after.object_bindings,
        restored.export_envelope().unwrap().object_bindings
    );
    let context = &after.state["execution_context"];
    assert!(
        context["units"]
            .as_array()
            .unwrap()
            .contains(&json!("hero_arien"))
    );
    assert!(
        !context["snapshots"][0]
            .as_array()
            .unwrap()
            .contains(&json!("hero_arien"))
    );
    assert_eq!(root.export_envelope().unwrap().state, before.state);
    assert_eq!(
        root.export_envelope().unwrap().object_bindings,
        before.object_bindings
    );
}

#[test]
fn unbound_query_predicate_rejects_before_pop_or_context_replacement() {
    use goa2_core::codec::ObjectPathPart;
    let mut envelope = source("object-execution-query-world-plain-UNIT");
    let target = serde_json::from_value(json!(["execution_stack", 1, "filters", 0])).unwrap();
    let paths = envelope
        .object_bindings
        .iter()
        .find(|binding| binding.paths.contains(&target))
        .unwrap()
        .paths
        .clone();
    for path in paths {
        let mut value = &mut envelope.state;
        for part in path {
            value = match part {
                ObjectPathPart::Key(key) => value.get_mut(&key).unwrap(),
                ObjectPathPart::Index(index) => value.get_mut(index).unwrap(),
            };
        }
        value["type"] = json!("unbound_query_fixture");
    }
    let mut session = NativeSession::compile(envelope).unwrap();
    let before = session.export_envelope().unwrap();
    assert!(
        session
            .advance_prefix(1)
            .unwrap_err()
            .to_string()
            .contains("live unit filter not bound")
    );
    let after = session.export_envelope().unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.object_bindings, before.object_bindings);
}

#[test]
fn retrieval_and_effect_list_replacement_are_private_and_restore_exactly() {
    let root = NativeSession::compile(source("object-execution-retrieve-effects")).unwrap();
    let before = root.export_envelope().unwrap();
    let mut worker = root.clone();
    let mut restored = NativeSession::compile(before.clone()).unwrap();
    for session in [&mut worker, &mut restored] {
        let result = session.advance_prefix(1).unwrap();
        assert_eq!(result.events.len(), 1);
    }
    let after = worker.export_envelope().unwrap();
    assert_eq!(after.state, restored.export_envelope().unwrap().state);
    assert_eq!(
        after.object_bindings,
        restored.export_envelope().unwrap().object_bindings
    );
    assert_eq!(after.state["active_effects"].as_array().unwrap().len(), 1);
    assert_eq!(
        after.state["execution_context"]["old_effects"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(root.export_envelope().unwrap().state, before.state);
    assert_eq!(
        root.export_envelope().unwrap().object_bindings,
        before.object_bindings
    );
}

#[test]
fn retrieval_equality_guard_does_not_commit_staged_card_mutations() {
    let mut envelope = source("object-execution-retrieve-equal-current");
    let clone = envelope.state["execution_context"]["clone"]["$goa2_instance"]
        .as_u64()
        .unwrap() as usize;
    envelope.instances[clone].equality = None;
    let mut session = NativeSession::compile(envelope).unwrap();
    let before = session.export_envelope().unwrap();
    assert!(
        session
            .advance_prefix(1)
            .unwrap_err()
            .to_string()
            .contains("private equality metadata")
    );
    let after = session.export_envelope().unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.object_bindings, before.object_bindings);
}

#[test]
fn token_properties_and_query_results_are_private_and_restore_after_placement() {
    for copied in [false, true] {
        for immune in [false, true] {
            let name = format!(
                "object-execution-token-query-{}-{}",
                if copied { "True" } else { "False" },
                if immune { "True" } else { "False" }
            );
            let root = NativeSession::compile(source(&name)).unwrap();
            let before = root.export_envelope().unwrap();
            let mut branch = root.clone();
            assert_eq!(branch.advance_prefix(1).unwrap().events.len(), 1);
            let checkpoint = branch.export_envelope().unwrap();
            let mut restored = NativeSession::compile(checkpoint.clone()).unwrap();
            for session in [&mut branch, &mut restored] {
                session.advance_prefix(4).unwrap();
            }
            let after = branch.export_envelope().unwrap();
            let restored = restored.export_envelope().unwrap();
            assert_eq!(after.state, restored.state);
            assert_eq!(after.object_bindings, restored.object_bindings);
            let expanded = after.materialize().unwrap();
            let context = &expanded["execution_context"];
            assert_eq!(context["pool_alias"]["owner_id"], json!("hero_wuk"));
            assert_eq!(
                context["pool_alias"]["is_immune_to_enemy_actions"],
                json!(immune)
            );
            assert_eq!(
                context["token_alias"]["owner_id"],
                json!(if copied { "hero_brogan" } else { "hero_wuk" })
            );
            assert_eq!(context["own_units"].as_array().unwrap().len(), 1);
            assert_eq!(
                context["enemy_units"].as_array().unwrap().len(),
                usize::from(copied || !immune)
            );
            let untouched = root.export_envelope().unwrap();
            assert_eq!(untouched.state, before.state);
            assert_eq!(untouched.object_bindings, before.object_bindings);
        }
    }
}

#[test]
fn token_path_removal_and_copied_retry_do_not_mutate_the_parent_worker() {
    for trail in ["False", "True"] {
        let fixture = case(&format!(
            "object-execution-token-path-removal-{trail}-True-invalid"
        ));
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let before = root.export_envelope().unwrap();
        let mut branch = root.clone();
        let operations = fixture["operations"].as_array().unwrap();
        apply(&mut branch, &operations[0]);
        let checkpoint = branch.export_envelope().unwrap();
        let mut restored = NativeSession::compile(checkpoint).unwrap();
        for operation in &operations[1..] {
            assert_eq!(
                apply(&mut branch, operation),
                apply(&mut restored, operation)
            );
        }
        let after = branch.export_envelope().unwrap();
        assert_eq!(after.state["execution_stack"], json!([]));
        let untouched = root.export_envelope().unwrap();
        assert_eq!(untouched.state, before.state);
        assert_eq!(untouched.object_bindings, before.object_bindings);
        assert_eq!(untouched.board_keys, before.board_keys);
    }
}

#[test]
fn batch_scratch_search_and_generated_filter_aliases_restore_in_private_workers() {
    for name in [
        "object-execution-token-batch-farthest-removals-invalid",
        "object-execution-token-batch-card-flashback-True-invalid",
    ] {
        let fixture = case(name);
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let before = root.export_envelope().unwrap();
        let mut branch = root.clone();
        let operations = fixture["operations"].as_array().unwrap();
        apply(&mut branch, &operations[0]);
        let checkpoint = branch.export_envelope().unwrap();
        let mut restored = NativeSession::compile(checkpoint).unwrap();
        for operation in &operations[1..] {
            assert_eq!(
                apply(&mut branch, operation),
                apply(&mut restored, operation)
            );
        }
        let after = branch.export_envelope().unwrap();
        assert_eq!(after.state["execution_stack"], json!([]));
        assert!(
            after.state["execution_context"]
                .get("_batch_freed_hexes")
                .is_none()
        );
        let untouched = root.export_envelope().unwrap();
        assert_eq!(untouched.state, before.state);
        assert_eq!(untouched.object_bindings, before.object_bindings);
        assert_eq!(untouched.board_keys, before.board_keys);
    }
}

#[test]
fn unbound_batch_slot_predicate_rejects_before_pop_or_owner_write() {
    use goa2_core::codec::ObjectPathPart;
    let mut envelope = source("object-execution-token-batch-reserve-last-first");
    let target =
        serde_json::from_value(json!(["execution_stack", 1, "slot_filters", 0, 0])).unwrap();
    let paths = envelope
        .object_bindings
        .iter()
        .find(|binding| binding.paths.contains(&target))
        .unwrap()
        .paths
        .clone();
    for path in paths {
        let mut value = &mut envelope.state;
        for part in path {
            value = match part {
                ObjectPathPart::Key(key) => value.get_mut(&key).unwrap(),
                ObjectPathPart::Index(index) => value.get_mut(index).unwrap(),
            };
        }
        value["type"] = json!("unbound_batch_fixture");
    }
    let mut session = NativeSession::compile(envelope).unwrap();
    let before = session.export_envelope().unwrap();
    assert!(
        session
            .advance_prefix(1)
            .unwrap_err()
            .to_string()
            .contains("live hex filter not bound")
    );
    let after = session.export_envelope().unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.object_bindings, before.object_bindings);
}

#[test]
fn batch_feasibility_rejects_mismatched_tile_geometry_before_pop() {
    let mut envelope = source("object-execution-token-batch-plain-first");
    let key = envelope.state["board"]["tiles"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    let path: Vec<goa2_core::codec::ObjectPathPart> =
        serde_json::from_value(json!(["board", "tiles", key, "hex"])).unwrap();
    // Detach the source dict's immutable key from its now-inconsistent Tile.hex.
    for board_key in envelope.board_keys.as_mut().unwrap() {
        if board_key.path.as_ref() == Some(&path) {
            board_key.path = None;
        }
    }
    envelope.state["board"]["tiles"][&key]["hex"] = json!({"q":20,"r":-20,"s":0});
    let mut session = NativeSession::compile(envelope).unwrap();
    let before = session.export_envelope().unwrap();
    assert!(
        session
            .advance_prefix(1)
            .unwrap_err()
            .to_string()
            .contains("token batch board tile/key geometry drift")
    );
    let after = session.export_envelope().unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.object_bindings, before.object_bindings);
    assert_eq!(after.board_keys, before.board_keys);
}

#[test]
fn movement_rechecks_its_live_gate_after_clearing_its_success_output() {
    for gate in ["active", "skip"] {
        let fixture = case(&format!("object-execution-walking-alias-success-{gate}"));
        let mut session =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let before = session.export_envelope().unwrap();
        let record = apply(&mut session, &fixture["operations"][0]);
        assert_eq!(record["events"], json!([]));
        assert_eq!(
            record["semantic_state"],
            fixture["expected"][0]["semantic_state"]
        );
        let after = session.export_envelope().unwrap();
        assert_eq!(
            after.state["entity_locations"],
            before.state["entity_locations"]
        );
        if gate == "active" {
            assert!(after.state["execution_context"].get("moved").is_none());
        } else {
            assert_eq!(
                after.state["execution_context"]["moved"],
                before.state["execution_context"]["moved"]
            );
        }
    }
}

#[test]
fn rock_ultimate_discard_continuations_restore_without_mutating_other_workers() {
    for name in [
        "object-execution-rock-ultimate-pieces-False-invalid",
        "object-execution-rock-ultimate-stone-grip-True-True-first",
    ] {
        let fixture = case(name);
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let before = root.export_envelope().unwrap();
        let mut branch = root.clone();
        let operations = fixture["operations"].as_array().unwrap();
        apply(&mut branch, &operations[0]);
        let mut restored = NativeSession::compile(branch.export_envelope().unwrap()).unwrap();
        restored.last_request = branch.last_request.clone().map(|mut request| {
            request.id = restored.import_request_token(&branch.request_token(request.id));
            request
        });
        for operation in &operations[1..] {
            assert_eq!(
                apply(&mut branch, operation),
                apply(&mut restored, operation)
            );
        }
        let after = branch.export_envelope().unwrap();
        assert_eq!(after.state["execution_stack"], json!([]));
        let untouched = root.export_envelope().unwrap();
        assert_eq!(untouched.state, before.state);
        assert_eq!(untouched.object_bindings, before.object_bindings);
        assert_eq!(untouched.board_keys, before.board_keys);
    }
}

#[test]
fn copied_generated_discard_keeps_its_id_value_and_has_distinct_trace_identity() {
    let fixture = case("object-execution-rock-ultimate-plain-first");
    let mut session =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    let operations = fixture["operations"].as_array().unwrap();
    apply(&mut session, &operations[0]);
    let before = session.export_envelope().unwrap();
    let template: Vec<goa2_core::codec::ObjectPathPart> =
        serde_json::from_value(json!(["execution_stack", 1, "steps_template", 0])).unwrap();
    let index = before
        .object_bindings
        .iter()
        .position(|binding| binding.paths.contains(&template))
        .unwrap();
    let identities = session
        .generated_step_objects(&before.object_bindings)
        .unwrap();
    let original = identities
        .iter()
        .find(|(binding, _)| *binding == index)
        .unwrap()
        .1;
    let raw_id = before.state["execution_stack"][1]["steps_template"][0]["step_id"].clone();
    for operation in &operations[1..7] {
        apply(&mut session, operation);
    }
    let after = session.export_envelope().unwrap();
    let target: Vec<goa2_core::codec::ObjectPathPart> =
        serde_json::from_value(json!(["execution_stack", 1])).unwrap();
    let index = after
        .object_bindings
        .iter()
        .position(|binding| binding.paths.contains(&target))
        .unwrap();
    let identities = session
        .generated_step_objects(&after.object_bindings)
        .unwrap();
    let copied = identities
        .iter()
        .find(|(binding, _)| *binding == index)
        .unwrap()
        .1;
    assert_ne!(original, copied);
    assert_eq!(after.state["execution_stack"][1]["step_id"], raw_id);
    assert!(after.state.get("generated_step_objects").is_none());
}

#[test]
fn rock_ultimate_coordinate_guard_is_atomic_and_used_gate_ignores_unused_coordinates() {
    let mut envelope = source("object-execution-rock-ultimate-plain-first");
    envelope.state["execution_context"]["rock"]["q"] = json!("bad");
    let mut session = NativeSession::compile(envelope.clone()).unwrap();
    let before = session.export_envelope().unwrap();
    assert!(session.advance_prefix(1).is_err());
    let after = session.export_envelope().unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.object_bindings, before.object_bindings);
    envelope.state["execution_context"]["rock_ultimate_used"] = json!(true);
    let mut skipped = NativeSession::compile(envelope).unwrap();
    skipped.advance_prefix(1).unwrap();
    assert_eq!(
        skipped.export_envelope().unwrap().state["execution_context"]["_rock_ult_affected"],
        json!(["old"])
    );
}

#[test]
fn turret_creation_removal_and_salvage_restore_in_private_workers() {
    for name in [
        "object-execution-turret-place-new",
        "object-execution-turret-reuse-after-remove",
        "object-execution-turret-queries",
        "object-execution-turret-salvage-1-False-invalid",
        "object-execution-turret-salvage-2-True-first",
        "object-execution-turret-salvage-3-True-invalid",
    ] {
        let fixture = case(name);
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let before = root.export_envelope().unwrap();
        let mut branch = root.clone();
        let operations = fixture["operations"].as_array().unwrap();
        apply(&mut branch, &operations[0]);
        let mut restored = NativeSession::compile(branch.export_envelope().unwrap()).unwrap();
        restored.last_request = branch.last_request.clone().map(|mut request| {
            request.id = restored.import_request_token(&branch.request_token(request.id));
            request
        });
        for operation in &operations[1..] {
            assert_eq!(
                apply(&mut branch, operation),
                apply(&mut restored, operation),
                "{name}"
            );
        }
        assert_eq!(
            branch.export_envelope().unwrap().state["execution_stack"],
            json!([])
        );
        let untouched = root.export_envelope().unwrap();
        assert_eq!(untouched.state, before.state);
        assert_eq!(untouched.object_bindings, before.object_bindings);
        assert_eq!(untouched.board_keys, before.board_keys);
    }
}

#[test]
fn turret_invalid_owner_and_registration_collision_are_atomic() {
    for name in [
        "object-execution-turret-place-owner-key",
        "object-execution-turret-place-reuse",
    ] {
        let mut envelope = source(name);
        if name.ends_with("owner-key") {
            envelope.state["execution_context"]["who"] = json!(17);
        } else {
            // Change the existing model's class discriminator at all alias paths.
            envelope.state["misc_entities"]["trinkets_turret"]["entity_kind"] =
                json!("placeholder");
            envelope.state["execution_context"]["turret_alias"]["entity_kind"] =
                json!("placeholder");
        }
        let mut session = NativeSession::compile(envelope).unwrap();
        let before = session.export_envelope().unwrap();
        assert!(session.advance_prefix(1).is_err());
        let after = session.export_envelope().unwrap();
        assert_eq!(after.state, before.state);
        assert_eq!(after.object_bindings, before.object_bindings);
        assert_eq!(after.board_keys, before.board_keys);
    }
}

#[test]
fn multi_selection_lists_and_filter_updates_restore_in_private_workers() {
    for name in [
        "object-execution-multi-shared-output-invalid",
        "object-execution-multi-output-filter-first",
        "object-execution-multi-hex-actor-piece-range-invalid",
        "object-execution-multi-card-self_destruct-True-invalid",
        "object-execution-multi-card-emergency_protocol-True-first",
        "object-execution-multi-injected-request-1.5",
    ] {
        let fixture = case(name);
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let before = root.export_envelope().unwrap();
        let mut branch = root.clone();
        let operations = fixture["operations"].as_array().unwrap();
        apply(&mut branch, &operations[0]);
        let mut restored = NativeSession::compile(branch.export_envelope().unwrap()).unwrap();
        restored.last_request = branch.last_request.clone().map(|mut request| {
            request.id = restored.import_request_token(&branch.request_token(request.id));
            request
        });
        for operation in &operations[1..] {
            assert_eq!(
                apply(&mut branch, operation),
                apply(&mut restored, operation),
                "{name}"
            );
        }
        let untouched = root.export_envelope().unwrap();
        assert_eq!(untouched.state, before.state);
        assert_eq!(untouched.object_bindings, before.object_bindings);
        assert_eq!(untouched.board_keys, before.board_keys);
    }
}

#[test]
fn multi_selection_unbound_filter_is_atomic_and_actorless_bypasses_it() {
    use goa2_core::codec::ObjectPathPart;
    let fixture = case("object-execution-multi-output-filter-first");
    let mut envelope = source("object-execution-multi-output-filter-first");
    let target: Vec<ObjectPathPart> =
        serde_json::from_value(json!(["execution_context", "selector", "filters", 0])).unwrap();
    let paths = envelope
        .object_bindings
        .iter()
        .find(|binding| binding.paths.contains(&target))
        .unwrap()
        .paths
        .clone();
    for path in paths {
        let mut value = &mut envelope.state;
        for part in path {
            value = match part {
                ObjectPathPart::Key(key) => value.get_mut(&key).unwrap(),
                ObjectPathPart::Index(index) => value.get_mut(index).unwrap(),
            };
        }
        value["type"] = json!("unbound_multi_fixture");
    }
    let mut session = NativeSession::compile(envelope.clone()).unwrap();
    let before = session.export_envelope().unwrap();
    assert!(session.advance_prefix(1).is_err());
    let after = session.export_envelope().unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.object_bindings, before.object_bindings);
    assert_eq!(after.board_keys, before.board_keys);
    envelope.state["current_actor_id"] = Value::Null;
    let mut actorless = NativeSession::compile(envelope).unwrap();
    apply(&mut actorless, &fixture["operations"][0]);
    let after = actorless.export_envelope().unwrap();
    let output: Vec<ObjectPathPart> =
        serde_json::from_value(json!(["execution_context", "chosen"])).unwrap();
    let selections: Vec<ObjectPathPart> =
        serde_json::from_value(json!(["execution_context", "selector", "selections"])).unwrap();
    assert!(
        after
            .object_bindings
            .iter()
            .any(|binding| binding.paths.contains(&output) && binding.paths.contains(&selections))
    );
}

#[test]
fn multi_selection_retains_injected_request_and_rejects_early_done() {
    let mut session =
        NativeSession::compile(source("object-execution-multi-injected-request-1.5")).unwrap();
    let request = session.advance_prefix(1).unwrap().request.unwrap();
    assert_eq!(session.request_token(request.id), "fixture/multi/request");
    assert!(
        !session.request_json(&request)["can_skip"]
            .as_bool()
            .unwrap()
    );
    let selection = session.import_selection(&json!("DONE")).unwrap();
    session.submit(request.id, selection).unwrap();
    let request = session.advance_prefix(1).unwrap().request.unwrap();
    assert!(
        !session.request_json(&request)["can_skip"]
            .as_bool()
            .unwrap()
    );
    assert!(
        session.export_envelope().unwrap().state["execution_context"]["selector"]["selections"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn defeat_protection_and_terminal_boundaries_restore_in_private_workers() {
    for name in [
        "object-execution-defeat-hero-current",
        "object-execution-defeat-hero-shared-tracking",
        "object-execution-live-combat-effect-protection-two-skip-False",
        "object-execution-live-combat-effect-protection-totem-priority-False",
        "object-execution-live-combat-effect-protection-card-linked-False",
        "object-execution-defeat-card-emergency_protocol-both-True-invalid",
        "object-execution-defeat-terminal-hero_swift-RESOLUTION",
        "object-execution-defeat-terminal-RED-GAME_OVER",
    ] {
        let fixture = case(name);
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let before = root.export_envelope().unwrap();
        let mut branch = root.clone();
        let operations = fixture["operations"].as_array().unwrap();
        apply(&mut branch, &operations[0]);
        let mut restored = NativeSession::compile(branch.export_envelope().unwrap()).unwrap();
        restored.last_request = branch.last_request.clone().map(|mut request| {
            request.id = restored.import_request_token(&branch.request_token(request.id));
            request
        });
        for operation in &operations[1..] {
            assert_eq!(
                apply(&mut branch, operation),
                apply(&mut restored, operation),
                "{name}"
            );
        }
        let untouched = root.export_envelope().unwrap();
        assert_eq!(untouched.state, before.state);
        assert_eq!(untouched.object_bindings, before.object_bindings);
        assert_eq!(untouched.board_keys, before.board_keys);
    }
}

#[test]
fn defeat_log_and_gold_overflow_guards_are_atomic() {
    for overflow in [false, true] {
        let mut envelope = source("object-execution-defeat-hero-plain");
        if overflow {
            envelope.state["teams"]["RED"]["heroes"][0]["gold"] = json!(i64::MAX);
        } else {
            envelope.state["execution_context"]["defeat_log"] = Value::Null;
        }
        let mut session = NativeSession::compile(envelope).unwrap();
        let before = session.export_envelope().unwrap();
        assert!(session.advance_prefix(1).is_err());
        let after = session.export_envelope().unwrap();
        assert_eq!(after.state, before.state);
        assert_eq!(after.object_bindings, before.object_bindings);
        assert_eq!(after.board_keys, before.board_keys);
    }
}

#[test]
fn game_over_rejects_ambiguous_winner_without_clearing_shared_stacks() {
    use goa2_core::codec::ObjectPathPart;
    let mut envelope = source("object-execution-defeat-terminal-RED-RESOLUTION");
    let path: Vec<ObjectPathPart> =
        serde_json::from_value(json!(["execution_context", "defeat_steps", 0])).unwrap();
    let aliases = envelope
        .object_bindings
        .iter()
        .find(|binding| binding.paths.contains(&path))
        .unwrap()
        .paths
        .clone();
    for alias in aliases {
        let mut value = &mut envelope.state;
        for part in alias {
            value = match part {
                ObjectPathPart::Key(key) => value.get_mut(&key).unwrap(),
                ObjectPathPart::Index(index) => value.get_mut(index).unwrap(),
            };
        }
        value["individual_winner_id"] = json!("hero_swift");
    }
    let mut session = NativeSession::compile(envelope).unwrap();
    let before = session.export_envelope().unwrap();
    assert!(session.advance_prefix(1).is_err());
    let after = session.export_envelope().unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.object_bindings, before.object_bindings);
}

#[test]
fn shared_bounty_budgets_are_private_and_equal_ids_do_not_merge_models() {
    for suffix in [
        "1-2-True",
        "2-2-True",
        "1-3-True",
        "2-3-True",
        "1-2-False",
        "2-2-False",
    ] {
        let fixture = case(&format!("object-execution-defeat-bounty-model-{suffix}"));
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let before = root.export_envelope().unwrap();
        let mut left = root.clone();
        let mut right = root.clone();
        for operation in fixture["operations"].as_array().unwrap() {
            assert_eq!(apply(&mut left, operation), apply(&mut right, operation));
        }
        let after = left.export_envelope().unwrap();
        let expected = &fixture["expected"].as_array().unwrap().last().unwrap()["semantic_state"];
        let materialized = after.materialize().unwrap();
        assert_eq!(materialized["active_effects"], expected["active_effects"]);
        assert_eq!(materialized["teams"], expected["teams"]);
        let untouched = root.export_envelope().unwrap();
        assert_eq!(untouched.state, before.state);
        assert_eq!(untouched.object_bindings, before.object_bindings);
    }
}

#[test]
fn combat_resolution_snapshots_and_history_restore_in_private_workers() {
    for name in [
        "object-execution-combat-live-precision-0",
        "object-execution-combat-live-precision-7",
        "object-execution-combat-live-flags-alias-block",
        "object-execution-combat-snapshot-targeting-independent",
        "object-execution-combat-snapshot-plain-ANY",
        "object-execution-combat-track-special-tuple-log",
        "object-execution-combat-track-special-output-marks",
    ] {
        let fixture = case(name);
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let before = root.export_envelope().unwrap();
        let mut branch = root.clone();
        let operations = fixture["operations"].as_array().unwrap();
        apply(&mut branch, &operations[0]);
        let mut restored = NativeSession::compile(branch.export_envelope().unwrap()).unwrap();
        for operation in &operations[1..] {
            assert_eq!(
                apply(&mut branch, operation),
                apply(&mut restored, operation),
                "{name}"
            );
        }
        let untouched = root.export_envelope().unwrap();
        assert_eq!(untouched.state, before.state);
        assert_eq!(untouched.object_bindings, before.object_bindings);
        assert_eq!(untouched.board_keys, before.board_keys);
    }
}

#[test]
fn live_combat_numeric_guards_are_atomic_and_missing_targets_bypass_them() {
    let mut envelope = source("object-execution-combat-live-flags-custom-key");
    envelope.state["execution_context"]["defense_value"] = json!("not-numeric");
    let mut session = NativeSession::compile(envelope.clone()).unwrap();
    let before = session.export_envelope().unwrap();
    assert!(session.advance_prefix(1).is_err());
    let after = session.export_envelope().unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.object_bindings, before.object_bindings);
    envelope.state["execution_context"]
        .as_object_mut()
        .unwrap()
        .shift_remove("custom");
    let mut missing = NativeSession::compile(envelope).unwrap();
    missing.advance_prefix(1).unwrap();
    let state = missing.export_envelope().unwrap().state;
    assert_eq!(state["execution_context"]["block_succeeded"], json!(false));
    assert_eq!(
        state["execution_context"]["last_combat_target"],
        json!("old")
    );
}

#[test]
fn attack_history_mark_guard_does_not_change_the_saved_log_or_output() {
    let mut envelope = source("object-execution-combat-track-special-output-marks");
    envelope.state["execution_context"]["defeat_marks"]["attack_target"] = json!(1.5);
    let mut session = NativeSession::compile(envelope).unwrap();
    let before = session.export_envelope().unwrap();
    assert!(session.advance_prefix(1).is_err());
    let after = session.export_envelope().unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.object_bindings, before.object_bindings);
}

#[test]
fn attack_macro_copies_and_filter_aliases_restore_in_private_workers() {
    for name in [
        "default",
        "existing",
        "optional-shared",
        "optional-skip",
        "shared-tracking",
        "piece-bound-1",
    ] {
        let fixture = case(&format!("object-execution-attack-macro-{name}"));
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let before = root.export_envelope().unwrap();
        let mut branch = root.clone();
        let operations = fixture["operations"].as_array().unwrap();
        apply(&mut branch, &operations[0]);
        let mut restored = NativeSession::compile(branch.export_envelope().unwrap()).unwrap();
        for operation in &operations[1..] {
            assert_eq!(
                apply(&mut branch, operation),
                apply(&mut restored, operation),
                "{name}"
            );
        }
        let untouched = root.export_envelope().unwrap();
        assert_eq!(untouched.state, before.state);
        assert_eq!(untouched.object_bindings, before.object_bindings);
        assert_eq!(untouched.board_keys, before.board_keys);
    }
}

#[test]
fn attack_bonus_guard_is_atomic_and_missing_targets_bypass_unused_values() {
    use goa2_core::codec::ObjectPathPart;
    let mut envelope = source("object-execution-attack-macro-bonus-int");
    envelope.state["execution_context"]["damage_bonus"] = json!("not-an-integer");
    let mut session = NativeSession::compile(envelope.clone()).unwrap();
    let before = session.export_envelope().unwrap();
    assert!(session.advance_prefix(1).is_err());
    let after = session.export_envelope().unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.object_bindings, before.object_bindings);
    let path: Vec<ObjectPathPart> =
        serde_json::from_value(json!(["execution_context", "attack_alias"])).unwrap();
    let aliases = envelope
        .object_bindings
        .iter()
        .find(|binding| binding.paths.contains(&path))
        .unwrap()
        .paths
        .clone();
    for alias in aliases {
        let mut value = &mut envelope.state;
        for part in alias {
            value = match part {
                ObjectPathPart::Key(key) => value.get_mut(&key).unwrap(),
                ObjectPathPart::Index(index) => value.get_mut(index).unwrap(),
            };
        }
        value["target_id_key"] = json!("missing_target");
    }
    let mut missing = NativeSession::compile(envelope).unwrap();
    missing.advance_prefix(1).unwrap();
    let after = missing.export_envelope().unwrap();
    assert_eq!(
        after.state["execution_context"]["attack_damage"],
        json!("old")
    );
    assert_eq!(
        after.state["execution_context"]["defeat_marks"],
        before.state["execution_context"]["defeat_marks"]
    );
}

#[test]
fn attack_expansion_executes_defense_continuations_and_restores_each_boundary() {
    let mut session =
        NativeSession::compile(source("object-execution-attack-macro-existing")).unwrap();
    session.advance_prefix(1).unwrap();
    let mut defense = false;
    for _ in 0..30 {
        let before = session.export_envelope().unwrap();
        let stack = before.state["execution_stack"].as_array().unwrap();
        if stack.is_empty() {
            assert!(defense);
            return;
        }
        defense |= stack.last().unwrap()["type"] == "resolve_defense_text";
        let mut restored = NativeSession::compile(before).unwrap();
        restored.last_request = session.last_request.clone().map(|mut request| {
            request.id = restored.import_request_token(&session.request_token(request.id));
            request
        });
        let operation = json!({"kind":"advance_prefix","steps":1});
        let actual = apply(&mut session, &operation);
        assert_eq!(actual, apply(&mut restored, &operation));
        if !actual["request"].is_null() {
            let input = json!({"kind":"input","request_id":"current","selection":"PASS"});
            assert_eq!(apply(&mut session, &input), apply(&mut restored, &input));
        }
    }
    panic!("attack continuation did not finish");
}

#[test]
fn reaction_windows_restore_pending_aliases_and_action_stacks_in_private_workers() {
    for name in [
        "Arien-prompt-invalid-pass",
        "context-stack-aggregate-current",
        "shield-equal-hand",
        "shield-unequal-hand",
        "piece-piece-3",
        "reused-card",
        "attack-hero-card",
        "actor-save-then-read",
        "actor-numeric",
    ] {
        let fixture = case(&format!("object-execution-live-reaction-{name}"));
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let before = root.export_envelope().unwrap();
        let mut branch = root.clone();
        let operations = fixture["operations"].as_array().unwrap();
        apply(&mut branch, &operations[0]);
        let mut restored = NativeSession::compile(branch.export_envelope().unwrap()).unwrap();
        restored.last_request = branch.last_request.clone().map(|mut request| {
            request.id = restored.import_request_token(&branch.request_token(request.id));
            request
        });
        for operation in &operations[1..] {
            assert_eq!(
                apply(&mut branch, operation),
                apply(&mut restored, operation),
                "{name}"
            );
        }
        let untouched = root.export_envelope().unwrap();
        assert_eq!(untouched.state, before.state);
        assert_eq!(untouched.object_bindings, before.object_bindings);
        assert_eq!(untouched.board_keys, before.board_keys);
    }
}

#[test]
fn invalid_reaction_stack_is_rejected_before_resetting_flags_or_consuming_input() {
    let mut opening = NativeSession::compile(source(
        "object-execution-live-reaction-context-stack-null-no-current",
    ))
    .unwrap();
    let request = opening.advance_prefix(1).unwrap().request.unwrap();
    let wire = opening.request_json(&request);
    let selected = opening.import_selection(&wire["options"][0]["id"]).unwrap();
    opening.submit(request.id, selected).unwrap();
    let mut envelope = opening.export_envelope().unwrap();
    envelope.state["execution_context"]["current_action_type"] = json!("ATTACK");
    envelope.state["execution_context"]["auto_block"] = json!(true);
    let mut session = NativeSession::compile(envelope).unwrap();
    let before = session.export_envelope().unwrap();
    assert!(session.advance_prefix(1).is_err());
    let after = session.export_envelope().unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.object_bindings, before.object_bindings);
}

#[test]
fn generated_actor_switches_read_live_save_and_actor_keys() {
    let fixture = case("object-execution-live-reaction-attack-minion");
    let mut session =
        NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
            .unwrap();
    for operation in fixture["operations"].as_array().unwrap() {
        apply(&mut session, operation);
    }
    let state = session.export_envelope().unwrap().materialize().unwrap();
    assert_eq!(
        state["current_actor_id"],
        state["execution_context"]["defender_id"]
    );
    assert_eq!(
        state["execution_context"]["_pre_pam_actor"],
        json!("hero_swift")
    );
    assert!(
        !state["execution_context"]
            .as_object()
            .unwrap()
            .contains_key("saved_actor_id")
    );
}

#[test]
fn pre_action_children_and_reentry_restore_in_private_workers() {
    for name in [
        "live-targeting-pre-move-mine-blast-first",
        "live-targeting-pre-discard-one-card-last",
        "pre-action-discard-alias-shared-step",
        "pre-action-discard-alias-shared-processed",
        "pre-action-card-disruptor_grid-defender",
        "pre-action-card-mastery-first",
        "live-targeting-attack-pre-leave-radius-False-first",
        "live-targeting-pre-discard-owner-bound-first",
    ] {
        let fixture = case(&format!("object-execution-{name}"));
        let root =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let before = root.export_envelope().unwrap();
        let mut branch = root.clone();
        let operations = fixture["operations"].as_array().unwrap();
        apply(&mut branch, &operations[0]);
        let mut restored = NativeSession::compile(branch.export_envelope().unwrap()).unwrap();
        restored.last_request = branch.last_request.clone().map(|mut request| {
            request.id = restored.import_request_token(&branch.request_token(request.id));
            request
        });
        for operation in &operations[1..] {
            assert_eq!(
                apply(&mut branch, operation),
                apply(&mut restored, operation),
                "{name}"
            );
        }
        let untouched = root.export_envelope().unwrap();
        assert_eq!(untouched.state, before.state);
        assert_eq!(untouched.object_bindings, before.object_bindings);
        assert_eq!(untouched.board_keys, before.board_keys);
    }
}

#[test]
fn pre_action_processed_append_guard_is_atomic_and_unread_state_is_accepted() {
    use goa2_core::codec::ObjectPathPart;
    let mut envelope = source("object-execution-pre-action-discard-alias-processed-tuple");
    let path: Vec<ObjectPathPart> = serde_json::from_value(json!(["active_effects", 0])).unwrap();
    let aliases = envelope
        .object_bindings
        .iter()
        .find(|binding| binding.paths.contains(&path))
        .unwrap()
        .paths
        .clone();
    for alias in aliases {
        let mut value = &mut envelope.state;
        for part in alias {
            value = match part {
                ObjectPathPart::Key(key) => value.get_mut(&key).unwrap(),
                ObjectPathPart::Index(index) => value.get_mut(index).unwrap(),
            };
        }
        value["id"] = json!("new-disrupt");
    }
    let mut session = NativeSession::compile(envelope).unwrap();
    let before = session.export_envelope().unwrap();
    assert!(session.advance_prefix(1).is_err());
    let after = session.export_envelope().unwrap();
    assert_eq!(after.state, before.state);
    assert_eq!(after.object_bindings, before.object_bindings);
    let mut no_effects = NativeSession::compile(source(
        "object-execution-pre-action-discard-alias-no-effects-null",
    ))
    .unwrap();
    no_effects.advance_prefix(1).unwrap();
    assert!(no_effects.export_envelope().unwrap().state["execution_context"]["pre_action_alias"]["processed_effect_ids"].is_null());
}

#[test]
fn resource_effects_restore_at_every_boundary_and_keep_workers_private() {
    let mut cases_checked = 0;
    let mut operations = 0;
    for case in cases_matching(|name| name.starts_with("object-execution-resource-")) {
        let envelope: StateEnvelope = serde_json::from_value(case["envelope"].clone()).unwrap();
        let root = NativeSession::compile(envelope).unwrap();
        let before = root.export_envelope().unwrap();
        let mut worker = root.clone();
        for operation in case["operations"].as_array().unwrap() {
            let mut restored = NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
            assert_eq!(
                apply(&mut worker, operation),
                apply(&mut restored, operation),
                "{}",
                case["name"]
            );
            operations += 1;
        }
        let after = root.export_envelope().unwrap();
        assert_eq!(before.state, after.state);
        assert_eq!(before.object_bindings, after.object_bindings);
        cases_checked += 1;
    }
    assert_eq!(cases_checked, 51);
    assert_eq!(operations, 207);
}

#[test]
fn primary_text_bodies_restore_all_boundaries_and_keep_worker_models_private() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let operations = AtomicUsize::new(0);
    let count = fixture_parallel::for_each_case(
        cases_matching(|name| name.starts_with("object-execution-primary-text-")),
        |case| {
            let root =
                NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                    .unwrap();
            let before = root.export_envelope().unwrap();
            let mut worker = root.clone();
            for operation in case["operations"].as_array().unwrap() {
                let mut restored =
                    NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
                restored.last_request = worker.last_request.clone().map(|mut request| {
                    request.id = restored.import_request_token(&worker.request_token(request.id));
                    request
                });
                assert_eq!(
                    apply(&mut worker, operation),
                    apply(&mut restored, operation),
                    "{}",
                    case["name"]
                );
                operations.fetch_add(1, Ordering::Relaxed);
            }
            let after = root.export_envelope().unwrap();
            assert_eq!(before.state, after.state);
            assert_eq!(before.object_bindings, after.object_bindings);
        },
    );
    assert_eq!(count, 781);
    assert_eq!(operations.load(Ordering::Relaxed), 2051);
}

#[test]
fn utility_continuations_restore_all_boundaries_with_private_model_identities() {
    let mut cases_checked = 0;
    let mut operations = 0;
    for case in cases_matching(|name| name.starts_with("object-execution-utility-")) {
        let root =
            NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                .unwrap();
        let before = root.export_envelope().unwrap();
        let mut worker = root.clone();
        for operation in case["operations"].as_array().unwrap() {
            let mut restored = NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
            restored.last_request = worker.last_request.clone().map(|mut request| {
                request.id = restored.import_request_token(&worker.request_token(request.id));
                request
            });
            assert_eq!(
                apply(&mut worker, operation),
                apply(&mut restored, operation),
                "{}",
                case["name"]
            );
            operations += 1;
        }
        let after = root.export_envelope().unwrap();
        assert_eq!(after.state, before.state);
        assert_eq!(after.object_bindings, before.object_bindings);
        cases_checked += 1;
    }
    assert_eq!(cases_checked, 99);
    assert_eq!(operations, 1070);
}

#[test]
fn push_execution_restores_each_boundary_and_keeps_spatial_and_token_models_private() {
    let mut count = 0;
    let mut operations = 0;
    for case in cases_matching(|name| name.starts_with("object-execution-arena-push-")) {
        let root =
            NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                .unwrap();
        let before = root.export_envelope().unwrap();
        let mut worker = root.clone();
        for operation in case["operations"].as_array().unwrap() {
            let mut restored = NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
            restored.last_request = worker.last_request.clone().map(|mut request| {
                request.id = restored.import_request_token(&worker.request_token(request.id));
                request
            });
            assert_eq!(
                apply(&mut worker, operation),
                apply(&mut restored, operation),
                "{}",
                case["name"]
            );
            operations += 1;
        }
        let after = root.export_envelope().unwrap();
        assert_eq!(before.state, after.state);
        assert_eq!(before.object_bindings, after.object_bindings);
        count += 1;
    }
    assert_eq!(count, 252);
    assert_eq!(operations, 1635);
}

#[test]
fn card_actions_restore_every_menu_and_lifecycle_boundary_with_private_models() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let operations = AtomicUsize::new(0);
    let count = fixture_parallel::for_each_case(
        cases_matching(|name| name.starts_with("object-execution-card-action-")),
        |case| {
            let root =
                NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                    .unwrap();
            let before = root.export_envelope().unwrap();
            let mut worker = root.clone();
            for operation in case["operations"].as_array().unwrap() {
                let mut restored =
                    NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
                restored.last_request = worker.last_request.clone().map(|mut request| {
                    request.id = restored.import_request_token(&worker.request_token(request.id));
                    request
                });
                assert_eq!(
                    apply(&mut worker, operation),
                    apply(&mut restored, operation),
                    "{}",
                    case["name"]
                );
                operations.fetch_add(1, Ordering::Relaxed);
            }
            let after = root.export_envelope().unwrap();
            assert_eq!(before.state, after.state);
            assert_eq!(before.object_bindings, after.object_bindings);
        },
    );
    assert_eq!(count, 632);
    assert_eq!(operations.load(Ordering::Relaxed), 7268);
}

#[test]
fn fast_travel_restores_all_boundaries_with_private_locations_and_generated_models() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let operations = AtomicUsize::new(0);
    let count = fixture_parallel::for_each_case(
        cases_matching(|name| name.starts_with("object-execution-arena-travel-")),
        |case| {
            let root =
                NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                    .unwrap();
            let before = root.export_envelope().unwrap();
            let mut worker = root.clone();
            for operation in case["operations"].as_array().unwrap() {
                let mut restored =
                    NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
                restored.last_request = worker.last_request.clone().map(|mut request| {
                    request.id = restored.import_request_token(&worker.request_token(request.id));
                    request
                });
                assert_eq!(
                    apply(&mut worker, operation),
                    apply(&mut restored, operation),
                    "{}",
                    case["name"]
                );
                operations.fetch_add(1, Ordering::Relaxed);
            }
            let after = root.export_envelope().unwrap();
            assert_eq!(before.state, after.state);
            assert_eq!(before.object_bindings, after.object_bindings);
        },
    );
    assert_eq!(count, 184);
    assert_eq!(operations.load(Ordering::Relaxed), 917);
}

#[test]
fn primary_reperformance_restores_every_boundary_and_keeps_outer_scopes_private() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let operations = AtomicUsize::new(0);
    let count = fixture_parallel::for_each_case(
        cases_matching(|name| name.starts_with("object-execution-perform-primary-")),
        |case| {
            let root =
                NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                    .unwrap();
            let before = root.export_envelope().unwrap();
            let mut worker = root.clone();
            for operation in case["operations"].as_array().unwrap() {
                let mut restored =
                    NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
                restored.last_request = worker.last_request.clone().map(|mut request| {
                    request.id = restored.import_request_token(&worker.request_token(request.id));
                    request
                });
                assert_eq!(
                    apply(&mut worker, operation),
                    apply(&mut restored, operation),
                    "{}",
                    case["name"]
                );
                operations.fetch_add(1, Ordering::Relaxed);
            }
            let after = root.export_envelope().unwrap();
            assert_eq!(before.state, after.state);
            assert_eq!(before.object_bindings, after.object_bindings);
        },
    );
    assert_eq!(count, 605);
    assert_eq!(operations.load(Ordering::Relaxed), 1895);
}

#[test]
fn arien_primary_flows_restore_each_boundary_with_private_effects_and_positions() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let operations = AtomicUsize::new(0);
    let count = fixture_parallel::for_each_case(
        cases_matching(|name| name.starts_with("object-execution-arien-primary-")),
        |case| {
            let root =
                NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                    .unwrap();
            let before = root.export_envelope().unwrap();
            let mut worker = root.clone();
            for operation in case["operations"].as_array().unwrap() {
                let mut restored =
                    NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
                restored.last_request = worker.last_request.clone().map(|mut request| {
                    request.id = restored.import_request_token(&worker.request_token(request.id));
                    request
                });
                assert_eq!(
                    apply(&mut worker, operation),
                    apply(&mut restored, operation),
                    "{}",
                    case["name"]
                );
                operations.fetch_add(1, Ordering::Relaxed);
            }
            let after = root.export_envelope().unwrap();
            assert_eq!(before.state, after.state);
            assert_eq!(before.object_bindings, after.object_bindings);
        },
    );
    assert_eq!(count, 136);
    assert_eq!(operations.load(Ordering::Relaxed), 2012);
}

#[test]
fn copied_card_flows_restore_each_boundary_with_foreign_card_and_scope_aliases() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let operations = AtomicUsize::new(0);
    let count = fixture_parallel::for_each_case(
        cases_matching(|name| {
            name.starts_with("object-execution-perform-card-edge-")
                || name.starts_with("object-execution-perform-card-flow-")
                || name.starts_with("object-execution-perform-card-menu-gydion-")
                || name.starts_with("object-execution-perform-card-menu-misa-")
                || name.starts_with("object-execution-perform-card-menu-wasp-")
                || name.starts_with("object-execution-perform-card-menu-min-")
        }),
        |case| {
            let root =
                NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                    .unwrap();
            let before = root.export_envelope().unwrap();
            let mut worker = root.clone();
            for operation in case["operations"].as_array().unwrap() {
                let mut restored =
                    NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
                restored.last_request = worker.last_request.clone().map(|mut request| {
                    request.id = restored.import_request_token(&worker.request_token(request.id));
                    request
                });
                assert_eq!(
                    apply(&mut worker, operation),
                    apply(&mut restored, operation),
                    "{}",
                    case["name"]
                );
                operations.fetch_add(1, Ordering::Relaxed);
            }
            let after = root.export_envelope().unwrap();
            assert_eq!(before.state, after.state);
            assert_eq!(before.object_bindings, after.object_bindings);
        },
    );
    assert_eq!(count, 199);
    assert_eq!(operations.load(Ordering::Relaxed), 2448);
}
#[test]
fn mind_grip_restores_every_branch_with_previous_card_and_nested_scope_identity() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let operations = AtomicUsize::new(0);
    let count = fixture_parallel::for_each_case(
        cases_matching(|name| name.starts_with("object-execution-mind-grip-")),
        |case| {
            let root =
                NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                    .unwrap();
            let before = root.export_envelope().unwrap();
            let mut worker = root.clone();
            for operation in case["operations"].as_array().unwrap() {
                let mut restored =
                    NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
                restored.last_request = worker.last_request.clone().map(|mut request| {
                    request.id = restored.import_request_token(&worker.request_token(request.id));
                    request
                });
                assert_eq!(
                    apply(&mut worker, operation),
                    apply(&mut restored, operation),
                    "{}",
                    case["name"]
                );
                operations.fetch_add(1, Ordering::Relaxed);
            }
            let after = root.export_envelope().unwrap();
            assert_eq!(before.state, after.state);
            assert_eq!(before.object_bindings, after.object_bindings);
        },
    );
    assert_eq!(count, 115);
    assert_eq!(operations.load(Ordering::Relaxed), 3036);
}
#[test]
fn tigerclaw_primary_and_primitive_boundaries_restore_with_private_gold_terrain_and_effects() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let operations = AtomicUsize::new(0);
    let count = fixture_parallel::for_each_case(
        cases_matching(|name| name.starts_with("object-execution-tiger-primary-")),
        |case| {
            let root =
                NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                    .unwrap();
            let before = root.export_envelope().unwrap();
            let mut worker = root.clone();
            for operation in case["operations"].as_array().unwrap() {
                let mut restored =
                    NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
                restored.last_request = worker.last_request.clone().map(|mut request| {
                    request.id = restored.import_request_token(&worker.request_token(request.id));
                    request
                });
                assert_eq!(
                    apply(&mut worker, operation),
                    apply(&mut restored, operation),
                    "{}",
                    case["name"]
                );
                operations.fetch_add(1, Ordering::Relaxed);
            }
            let after = root.export_envelope().unwrap();
            assert_eq!(before.state, after.state);
            assert_eq!(before.object_bindings, after.object_bindings);
        },
    );
    assert_eq!(count, 161);
    assert_eq!(operations.load(Ordering::Relaxed), 2488);
}

#[test]
fn brogan_bodies_and_queries_restore_each_boundary_with_private_collections_and_effects() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let operations = AtomicUsize::new(0);
    let count = fixture_parallel::for_each_case(
        cases_matching(|name| name.starts_with("object-execution-brogan-")),
        |case| {
            let root =
                NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                    .unwrap();
            let before = root.export_envelope().unwrap();
            let mut worker = root.clone();
            for operation in case["operations"].as_array().unwrap() {
                let mut restored =
                    NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
                restored.last_request = worker.last_request.clone().map(|mut request| {
                    request.id = restored.import_request_token(&worker.request_token(request.id));
                    request
                });
                assert_eq!(
                    apply(&mut worker, operation),
                    apply(&mut restored, operation),
                    "{}",
                    case["name"]
                );
                operations.fetch_add(1, Ordering::Relaxed);
            }
            let after = root.export_envelope().unwrap();
            assert_eq!(before.state, after.state);
            assert_eq!(before.object_bindings, after.object_bindings);
        },
    );
    assert_eq!(count, 286);
    assert_eq!(operations.load(Ordering::Relaxed), 3712);
}

#[test]
fn spellbook_operations_restore_each_boundary_with_private_cards_counters_and_continuations() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let operations = AtomicUsize::new(0);
    let count = fixture_parallel::for_each_case(
        cases_matching(|name| name.starts_with("object-execution-spellbook-")),
        |case| {
            let root =
                NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                    .unwrap();
            let before = root.export_envelope().unwrap();
            let mut worker = root.clone();
            for operation in case["operations"].as_array().unwrap() {
                let mut restored =
                    NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
                restored.last_request = worker.last_request.clone().map(|mut request| {
                    request.id = restored.import_request_token(&worker.request_token(request.id));
                    request
                });
                assert_eq!(
                    apply(&mut worker, operation),
                    apply(&mut restored, operation),
                    "{}",
                    case["name"]
                );
                operations.fetch_add(1, Ordering::Relaxed);
            }
            let after = root.export_envelope().unwrap();
            assert_eq!(before.state, after.state);
            assert_eq!(before.object_bindings, after.object_bindings);
        },
    );
    assert_eq!(count, 47);
    assert_eq!(operations.load(Ordering::Relaxed), 177);
}

#[test]
fn spell_bodies_restore_each_boundary_with_private_cards_filters_and_respawns() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let operations = AtomicUsize::new(0);
    let count = fixture_parallel::for_each_case(
        cases_matching(|name| {
            name.starts_with("object-execution-spell-body-")
                || name.starts_with("object-execution-spell-primitive-")
        }),
        |case| {
            let root =
                NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                    .unwrap();
            let before = root.export_envelope().unwrap();
            let mut worker = root.clone();
            for operation in case["operations"].as_array().unwrap() {
                let mut restored =
                    NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
                restored.last_request = worker.last_request.clone().map(|mut request| {
                    request.id = restored.import_request_token(&worker.request_token(request.id));
                    request
                });
                assert_eq!(
                    apply(&mut worker, operation),
                    apply(&mut restored, operation),
                    "{}",
                    case["name"]
                );
                operations.fetch_add(1, Ordering::Relaxed);
            }
            let after = root.export_envelope().unwrap();
            assert_eq!(before.state, after.state);
            assert_eq!(before.object_bindings, after.object_bindings);
        },
    );
    assert_eq!(count, 304);
    assert_eq!(operations.load(Ordering::Relaxed), 6113);
}

#[test]
fn gydion_ordinary_bodies_restore_each_boundary_with_private_books_and_passives() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let operations = AtomicUsize::new(0);
    let count = fixture_parallel::for_each_case(
        cases_matching(|name| name.starts_with("object-execution-gydion-primary-")),
        |case| {
            let root =
                NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                    .unwrap();
            let before = root.export_envelope().unwrap();
            let mut worker = root.clone();
            for operation in case["operations"].as_array().unwrap() {
                let mut restored =
                    NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
                restored.last_request = worker.last_request.clone().map(|mut request| {
                    request.id = restored.import_request_token(&worker.request_token(request.id));
                    request
                });
                assert_eq!(
                    apply(&mut worker, operation),
                    apply(&mut restored, operation),
                    "{}",
                    case["name"]
                );
                operations.fetch_add(1, Ordering::Relaxed);
            }
            let after = root.export_envelope().unwrap();
            assert_eq!(before.state, after.state);
            assert_eq!(before.object_bindings, after.object_bindings);
        },
    );
    assert_eq!(count, 338);
    assert_eq!(operations.load(Ordering::Relaxed), 9631);
}

fn primary_family_restores_each_boundary(
    prefixes: &[&str],
    expected_cases: usize,
    expected_operations: usize,
) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let operations = AtomicUsize::new(0);
    let count = fixture_parallel::for_each_case(
        cases_matching(|name| prefixes.iter().any(|prefix| name.starts_with(prefix))),
        |case| {
            let root =
                NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                    .unwrap();
            let before = root.export_envelope().unwrap();
            let mut worker = root.clone();
            for operation in case["operations"].as_array().unwrap() {
                let mut restored =
                    NativeSession::compile(worker.export_envelope().unwrap()).unwrap();
                restored.last_request = worker.last_request.clone().map(|mut request| {
                    request.id = restored.import_request_token(&worker.request_token(request.id));
                    request
                });
                assert_eq!(
                    apply(&mut worker, operation),
                    apply(&mut restored, operation),
                    "{}",
                    case["name"]
                );
                operations.fetch_add(1, Ordering::Relaxed);
            }
            let after = root.export_envelope().unwrap();
            assert_eq!(before.state, after.state);
            assert_eq!(before.object_bindings, after.object_bindings);
        },
    );
    assert_eq!(count, expected_cases);
    assert_eq!(operations.load(Ordering::Relaxed), expected_operations);
}

#[test]
fn misa_bodies_and_geometry_restore_each_boundary_with_private_effects_and_hex_models() {
    primary_family_restores_each_boundary(
        &[
            "object-execution-misa-primary-",
            "object-execution-misa-geometry-",
        ],
        511,
        8038,
    );
}
#[test]
fn wasp_bodies_and_queries_restore_each_boundary_with_private_tokens_filters_and_effects() {
    primary_family_restores_each_boundary(
        &[
            "object-execution-wasp-primary-",
            "object-execution-wasp-query-",
        ],
        428,
        7971,
    );
}

#[test]
fn min_bodies_items_and_deferred_text_restore_each_boundary_with_private_models() {
    primary_family_restores_each_boundary(
        &[
            "object-execution-min-primary-",
            "object-execution-min-item-",
            "object-execution-min-after-",
        ],
        363,
        7873,
    );
}

#[test]
fn dynamic_passive_bodies_restore_each_boundary_with_private_cards_filters_and_effects() {
    primary_family_restores_each_boundary(&["object-execution-dynamic-passive-"], 176, 652);
}

#[test]
fn live_battles_restore_each_boundary_with_private_maps_spawns_effects_and_pending_steps() {
    primary_family_restores_each_boundary(&["object-execution-live-battle-"], 267, 1446);
}

#[test]
fn live_cleanup_restores_each_boundary_with_private_cards_lists_effects_and_level_up_snapshots() {
    primary_family_restores_each_boundary(&["object-execution-cleanup-live-"], 95, 192);
}

#[test]
fn live_upgrades_and_round_reset_restore_every_boundary_with_private_cards_items_and_maps() {
    primary_family_restores_each_boundary(&["object-execution-upgrade-live-"], 107, 653);
}

#[test]
fn live_hero_respawn_restores_each_boundary_with_private_tokens_effects_and_hex_aliases() {
    primary_family_restores_each_boundary(&["object-execution-hero-respawn-live-"], 123, 417);
}

#[test]
fn complete_turn_and_round_phase_flows_restore_each_boundary_with_private_maps_and_cards() {
    primary_family_restores_each_boundary(&["object-execution-phase-closure-live-"], 191, 1819);
}

#[test]
fn current_card_status_filters_restore_each_boundary_with_private_card_aliases() {
    primary_family_restores_each_boundary(&["object-execution-card-status-filter-"], 37, 222);
}

#[test]
fn coordinated_unit_and_token_movement_restores_every_boundary_with_private_hex_aliases() {
    primary_family_restores_each_boundary(&["object-execution-co-movement-"], 280, 1020);
}

#[test]
fn initiative_overrides_restore_every_boundary_with_private_physical_card_aliases() {
    primary_family_restores_each_boundary(&["object-execution-initiative-"], 52, 118);
}

#[test]
fn manager_control_and_generic_effect_creation_restore_all_boundaries_with_private_scopes() {
    primary_family_restores_each_boundary(
        &[
            "object-execution-action-control-",
            "object-execution-live-effect-step-",
        ],
        125,
        560,
    );
}

#[test]
fn journey_delayed_programs_restore_every_boundary_with_private_steps_filters_and_effects() {
    primary_family_restores_each_boundary(&["object-execution-journey-"], 77, 321);
}

#[test]
fn hanu_bodies_and_passive_rebuilds_restore_every_boundary_with_private_shared_filters() {
    primary_family_restores_each_boundary(&["object-execution-hanu-primary-"], 505, 8339);
}

#[test]
fn xargatha_bodies_and_cached_adjacency_restore_every_boundary_with_private_workers() {
    primary_family_restores_each_boundary(&["object-execution-xargatha-"], 482, 7850);
}

#[test]
fn bain_bounty_cards_and_marker_targeting_restore_every_boundary() {
    primary_family_restores_each_boundary(&["object-execution-bain-bounty-"], 85, 2441);
}

#[test]
fn line_of_sight_and_bain_line_attacks_restore_every_boundary() {
    primary_family_restores_each_boundary(&["object-execution-line-of-sight-"], 181, 2366);
}

#[test]
fn grappling_bolt_restores_every_boundary() {
    primary_family_restores_each_boundary(&["object-execution-grappling-"], 33, 353);
}

#[test]
fn walk_the_plank_restores_every_boundary() {
    primary_family_restores_each_boundary(&["object-execution-plank-"], 20, 402);
}

#[test]
fn cutter_primary_family_restores_every_boundary() {
    primary_family_restores_each_boundary(&["object-execution-cutter-primary-"], 320, 5871);
}

#[test]
fn session_rollback_restores_live_branches_and_staged_transitions() {
    let names = [
        "object-execution-hero-respawn-live-ordinary-first",
        "object-execution-cleanup-live-Takahide-ultimate-with-upgrades",
        "object-execution-upgrade-live-reset-shared-snapshot",
        "object-execution-phase-closure-live-round-upgrade-reset",
        "object-execution-cutter-primary-a_fistful_of_coins-text-last-win",
    ];
    let snapshot = |session: &NativeSession| {
        json!({
            "envelope": session.export_envelope().unwrap(),
            "request": session.last_request.as_ref().map(|request| session.request_json(request)),
        })
    };
    let mut checked = 0;
    for case in cases_matching(|name| names.contains(&name)) {
        checked += 1;
        let mut worker =
            NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                .unwrap();
        let operations = case["operations"].as_array().unwrap();
        let root = snapshot(&worker);
        let error = worker.with_rollback(|outer| {
            apply(outer, &operations[0]);
            let before_inner = snapshot(outer);
            outer.with_rollback(|inner| {
                for operation in &operations[1..] {
                    apply(inner, operation);
                }
                Ok(())
            })?;
            assert_eq!(snapshot(outer), before_inner, "{}", case["name"]);
            Err::<(), _>(EngineError::Deadline)
        });
        assert!(matches!(error, Err(EngineError::Deadline)));
        assert_eq!(snapshot(&worker), root, "{}", case["name"]);

        for operation in operations {
            let before = snapshot(&worker);
            let result = worker
                .with_rollback(|branch| Ok(apply(branch, operation)))
                .unwrap();
            assert_eq!(snapshot(&worker), before, "{}", case["name"]);
            assert_eq!(apply(&mut worker, operation), result, "{}", case["name"]);
        }
    }
    assert_eq!(checked, names.len());
}
