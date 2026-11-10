//! Small compatibility contracts independent of the differential corpus.
use goa2_core::api::{ExecutionBudget, GameContent, GameMode, GameSetup, LiveGame, PlayerCommand};
use serde_json::{Value, json};

fn game() -> LiveGame {
    let content =
        GameContent::read(include_bytes!("../../../content/smoke-game.json").as_slice()).unwrap();
    let setup = GameSetup {
        map: "forgotten_island".into(),
        red_heroes: vec!["Arien".into()],
        blue_heroes: vec!["Brogan".into()],
        seed: u64::MAX,
        mode: GameMode::Quick,
        cheats_enabled: false,
        tie_breaker: None,
    };
    LiveGame::create(&content, &setup, &ExecutionBudget::default())
        .unwrap()
        .0
}

#[test]
fn checkpoint_v1_defaults_restore_and_future_versions_reject() {
    let game = game();
    let mut saved = serde_json::to_value(game.checkpoint().unwrap()).unwrap();
    assert_eq!(saved["version"], 1);
    saved.as_object_mut().unwrap().remove("upgrade_history");
    saved.as_object_mut().unwrap().remove("rollback");
    let restored = LiveGame::restore(
        serde_json::from_value(saved.clone()).unwrap(),
        &ExecutionBudget::default(),
    )
    .unwrap();
    assert_eq!(
        game.client_view(Some("hero_arien")).unwrap(),
        restored.client_view(Some("hero_arien")).unwrap()
    );
    saved["version"] = json!(2);
    assert!(
        LiveGame::restore(
            serde_json::from_value(saved).unwrap(),
            &ExecutionBudget::default()
        )
        .is_err()
    );
}

#[test]
fn command_unknown_fields_and_stale_revisions_do_not_mutate_game() {
    assert!(
        serde_json::from_value::<PlayerCommand>(
            json!({"kind":"commit","card_id":"aspiring_duelist","extra":1})
        )
        .is_err()
    );
    let mut game = game();
    let before = serde_json::to_value(game.checkpoint().unwrap()).unwrap();
    assert!(
        game.submit(
            "hero_arien",
            game.position().revision + 1,
            PlayerCommand::Pass,
            &ExecutionBudget::default()
        )
        .is_err()
    );
    assert_eq!(
        before,
        serde_json::to_value(game.checkpoint().unwrap()).unwrap()
    );
}

#[test]
fn own_hidden_hand_is_absent_from_opponent_and_spectator_views() {
    let mut game = game();
    let own = game.client_view(Some("hero_arien")).unwrap();
    let card = own["view"]["teams"]["RED"]["heroes"][0]["hand"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    game.submit(
        "hero_arien",
        game.position().revision,
        PlayerCommand::Commit {
            card_id: card.clone(),
        },
        &ExecutionBudget::default(),
    )
    .unwrap();
    for viewer in [Some("hero_brogan"), None] {
        let view: Value = game.client_view(viewer).unwrap();
        let red = &view["view"]["teams"]["RED"]["heroes"][0];
        assert!(
            red["hand"]
                .as_array()
                .is_none_or(|cards| cards.iter().all(|c| c["id"] != card))
        );
        assert!(
            red.get("current_turn_card").is_some(),
            "current card field must remain present"
        );
        assert_ne!(red["current_turn_card"]["id"], card);
    }
    assert!(game.client_view(Some("unknown-player")).is_err());
}

#[test]
fn serialized_boundaries_keep_defaults_and_reject_future_identity_and_unknown_fields() {
    use goa2_core::api::{GameCheckpoint, TimeControlConfig};
    let setup: GameSetup = serde_json::from_value(json!({
        "map":"forgotten_island", "red_heroes":["Arien"], "blue_heroes":["Brogan"],
        "seed":u64::MAX
    }))
    .unwrap();
    assert!(matches!(setup.mode, GameMode::Long));
    assert!(!setup.cheats_enabled && setup.tie_breaker.is_none());
    let mut encoded = serde_json::to_value(&setup).unwrap();
    encoded["future"] = json!(true);
    assert!(serde_json::from_value::<GameSetup>(encoded).is_err());
    let mut content: Value =
        serde_json::from_slice(include_bytes!("../../../content/smoke-game.json")).unwrap();
    for key in ["version", "oracle_revision", "source_sha256", "future"] {
        let original = content.clone();
        content[key] = if key == "version" {
            json!(2)
        } else {
            json!("future")
        };
        assert!(GameContent::read(serde_json::to_vec(&content).unwrap().as_slice()).is_err());
        content = original;
    }
    let mut saved = serde_json::to_value(game().checkpoint().unwrap()).unwrap();
    saved["future"] = json!(true);
    assert!(serde_json::from_value::<GameCheckpoint>(saved.clone()).is_err());
    saved.as_object_mut().unwrap().remove("future");
    for (key, value) in [
        ("schema_version", json!(2)),
        ("rules_version", json!(10)),
        ("oracle_revision", json!("future")),
        ("source_sha256", json!("future")),
    ] {
        let mut changed = saved.clone();
        changed["envelope"][key] = value;
        assert!(
            LiveGame::restore(
                serde_json::from_value(changed).unwrap(),
                &ExecutionBudget::default()
            )
            .is_err()
        );
    }
    let clock: TimeControlConfig = serde_json::from_value(json!({
        "planning_allowance_seconds":1,"resolution_allowance_seconds":1,
        "response_grant_seconds":0,"initial_time_bank_seconds":0,
        "time_bank_increment_seconds":0,"max_time_bank_seconds":0,"upgrade_allowance_seconds":1,
        "future_optional":true
    }))
    .unwrap();
    assert_eq!(clock.initiative_bonus_seconds, 0);
    assert_eq!(clock.automatic_turn_limit, 2);
    assert!(clock.enforce_timeouts);
}

#[test]
fn simultaneous_upgrade_prompts_are_recipient_scoped_and_cannot_submit_for_another_seat() {
    use goa2_core::codec::StateEnvelope;
    let saved = serde_json::to_value(game().checkpoint().unwrap()).unwrap();
    let mut envelope: StateEnvelope = serde_json::from_value(saved["envelope"].clone()).unwrap();
    envelope.state["phase"] = json!("LEVEL_UP");
    envelope.state["pending_upgrades"] = json!({"hero_arien":1,"hero_brogan":1});
    envelope.state["execution_stack"] = json!([{ "type":"resolve_upgrades", "step_id":"synthetic/upgrades", "pending_input":null, "pending_request_id":null, "is_mandatory":true, "active_if_key":null, "skip_if_key":null }]);
    envelope.state["execution_context"] = json!({});
    envelope.object_bindings.push(
        serde_json::from_value(json!({"kind":"step","paths":[["execution_stack",0]]})).unwrap(),
    );
    let (mut game, _) = LiveGame::import(envelope, &ExecutionBudget::default()).unwrap();
    let arien = game.client_view(Some("hero_arien")).unwrap()["input_request"].clone();
    let brogan = game.client_view(Some("hero_brogan")).unwrap()["input_request"].clone();
    assert_eq!(arien["type"], "UPGRADE_PHASE");
    assert_eq!(arien["request_id"], brogan["request_id"]);
    for (id, prompt) in [("hero_arien", &arien), ("hero_brogan", &brogan)] {
        let players = prompt["players"].as_object().unwrap();
        assert_eq!(players.len(), 1);
        assert!(players.contains_key(id));
    }
    assert!(game.client_view(None).unwrap()["input_request"].is_null());
    let before = serde_json::to_value(game.checkpoint().unwrap()).unwrap();
    assert!(
        game.submit(
            "hero_brogan",
            0,
            PlayerCommand::Input {
                request_id: arien["request_id"].as_str().unwrap().into(),
                selection: json!({"hero_id":"hero_arien","card_id":"expert_duelist"})
            },
            &ExecutionBudget::default()
        )
        .is_err()
    );
    assert_eq!(
        before,
        serde_json::to_value(game.checkpoint().unwrap()).unwrap()
    );
}

#[test]
fn ordered_events_hide_facedown_identity_for_teammate_and_spectator() {
    let content =
        GameContent::read(include_bytes!("../../../content/smoke-game.json").as_slice()).unwrap();
    let setup = GameSetup {
        map: "forgotten_island".into(),
        red_heroes: vec!["Arien".into(), "Brogan".into()],
        blue_heroes: vec!["Hanu".into(), "Sabina".into()],
        seed: 29,
        mode: GameMode::Quick,
        cheats_enabled: false,
        tie_breaker: None,
    };
    let (mut game, _) = LiveGame::create(&content, &setup, &ExecutionBudget::default()).unwrap();
    let mut transition = game
        .submit(
            "hero_arien",
            0,
            PlayerCommand::Commit {
                card_id: "aspiring_duelist".into(),
            },
            &ExecutionBudget::default(),
        )
        .unwrap();
    transition.events = vec![
        json!({"event_type":"FIRST","metadata":{"card_id":"aspiring_duelist"}}),
        json!({"event_type":"SECOND","metadata":{"card_id":"aspiring_duelist"}}),
    ];
    for viewer in [Some("hero_brogan"), Some("hero_hanu"), None] {
        let view = game.client_view(viewer).unwrap();
        assert!(
            view["view"]["teams"]["RED"]["heroes"][0]["hand"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let result = game.client_result(&transition, viewer).unwrap();
        let events = result["events"].as_array().unwrap();
        assert_eq!(
            events
                .iter()
                .map(|v| v["event_type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["FIRST", "SECOND"]
        );
        assert!(
            !serde_json::to_string(&events)
                .unwrap()
                .contains("aspiring_duelist")
        );
    }
    assert!(
        game.client_result(&transition, Some("hero_arien"))
            .unwrap()
            .to_string()
            .contains("aspiring_duelist")
    );
    assert!(
        game.private_replay_projection()
            .unwrap()
            .to_string()
            .contains("aspiring_duelist")
    );
}

fn smoke_envelope(name: &str) -> goa2_core::codec::StateEnvelope {
    use std::{fs::File, path::Path};
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/smoke");
    let manifest: Value =
        serde_json::from_reader(File::open(base.join("manifest.json")).unwrap()).unwrap();
    let row = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == name)
        .unwrap();
    let file = File::open(base.join(format!("{}.gz", row["path"].as_str().unwrap()))).unwrap();
    let case: Value = serde_json::from_reader(flate2::read::GzDecoder::new(file)).unwrap();
    serde_json::from_value(case["envelope"].clone()).unwrap()
}

#[test]
fn team_decision_is_available_to_eligible_teammates_without_exposing_other_seats() {
    let (game, _) = LiveGame::import(
        smoke_envelope("object-execution-revelation-tie-wrong-choice"),
        &ExecutionBudget::default(),
    )
    .unwrap();
    let mut eligible = Vec::new();
    for (id, _) in game.players() {
        let view = game.client_view(Some(id)).unwrap();
        if let Some(request) = game.request_for(id) {
            assert_eq!(request["request_type"], "CHOOSE_ACTOR");
            assert_eq!(view["input_request"]["type"], "CHOOSE_ACTOR");
            assert_eq!(
                view["input_request"]["player_ids"],
                json!(["hero_arien", "hero_hanu"])
            );
            eligible.push(id);
        } else {
            assert!(view["input_request"].is_null());
        }
    }
    assert_eq!(eligible, vec!["hero_arien", "hero_hanu"]);
    assert!(game.client_view(None).unwrap()["input_request"].is_null());
}

#[test]
fn generic_number_labels_preserve_typed_choices_and_request_freshness() {
    use goa2_core::codec::StateEnvelope;
    let saved = serde_json::to_value(game().checkpoint().unwrap()).unwrap();
    let mut envelope: StateEnvelope = serde_json::from_value(saved["envelope"].clone()).unwrap();
    let generic = smoke_envelope("number-input-resume");
    let mut step = generic.state["execution_stack"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["output_key"] == "count")
        .unwrap()
        .clone();
    step["override_player_id"] = json!("hero_arien");
    envelope.state["phase"] = json!("RESOLUTION");
    envelope.state["current_actor_id"] = json!("hero_arien");
    envelope.state["execution_stack"] = json!([step]);
    envelope.state["execution_context"] = json!({});
    envelope.object_bindings.push(
        serde_json::from_value(json!({"kind":"step","paths":[["execution_stack",0]]})).unwrap(),
    );
    let (mut game, _) = LiveGame::import(envelope, &ExecutionBudget::default()).unwrap();
    let prompt = game.client_view(Some("hero_arien")).unwrap()["input_request"].clone();
    assert_eq!(prompt["type"], "SELECT_NUMBER");
    assert_eq!(prompt["valid_options"], json!([1, 2, 3]));
    assert_eq!(prompt["options"][0], json!({"id":"1","text":"One"}));
    assert!(game.client_view(Some("hero_brogan")).unwrap()["input_request"].is_null());
    let before = serde_json::to_value(game.checkpoint().unwrap()).unwrap();
    assert!(
        game.submit(
            "hero_arien",
            0,
            PlayerCommand::Input {
                request_id: "stale-token".into(),
                selection: json!(1)
            },
            &ExecutionBudget::default()
        )
        .is_err()
    );
    assert_eq!(
        before,
        serde_json::to_value(game.checkpoint().unwrap()).unwrap()
    );
}
