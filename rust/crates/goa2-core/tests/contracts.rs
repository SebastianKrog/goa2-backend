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
