//! Deterministic bounded mutation/property cases; no external corpus or fuzz runtime.
use goa2_core::{
    api::{
        EngineError, ExecutionBudget, GameCheckpoint, GameContent, GameMode, GameSetup, LiveGame,
        PlayerCommand,
    },
    codec::StateEnvelope,
    geometry::Hex,
};
use serde_json::{Value, json};
use std::{sync::atomic::AtomicBool, time::Instant};

fn content() -> GameContent {
    GameContent::read(include_bytes!("../../../content/smoke-game.json").as_slice()).unwrap()
}
fn game(content: &GameContent, seed: u64) -> LiveGame {
    LiveGame::create(
        content,
        &GameSetup {
            map: "forgotten_island".into(),
            red_heroes: vec!["Arien".into()],
            blue_heroes: vec!["Brogan".into()],
            seed,
            mode: GameMode::Quick,
            cheats_enabled: false,
            tie_breaker: None,
        },
        &ExecutionBudget::default(),
    )
    .unwrap()
    .0
}
fn saved(game: &LiveGame) -> Value {
    serde_json::to_value(game.checkpoint().unwrap()).unwrap()
}

#[test]
fn malformed_reference_tables_bindings_and_versions_reject_without_panicking() {
    let initial = saved(&game(&content(), 29));
    let good = initial["envelope"].clone();
    let mut invalid = Vec::new();
    for index in [u64::MAX, 1_000_000] {
        let mut value = good.clone();
        value["state"]["corrupt_reference"] = json!({"$goa2_instance":index});
        invalid.push(value);
    }
    for bad in [json!(-1), json!("one"), Value::Null, json!([])] {
        let mut value = good.clone();
        value["state"]["corrupt_reference"] = json!({"$goa2_instance":bad});
        invalid.push(value);
    }
    let mut value = good.clone();
    value["instances"][0]["definition"] = json!(u32::MAX);
    invalid.push(value);
    for binding in [
        json!({"kind":"dict","paths":[["absent"]]}),
        json!({"kind":"dict","paths":[["teams"],["teams"]]}),
        json!({"kind":"list","paths":[["phase"]]}),
        json!({"kind":"step","paths":[["execution_stack",1000000]]}),
    ] {
        let mut value = good.clone();
        value["object_bindings"]
            .as_array_mut()
            .unwrap()
            .push(binding);
        invalid.push(value);
    }
    let mut value = good;
    value["native_resume"]["version"] = json!(u32::MAX);
    invalid.push(value);
    for (index, value) in invalid.into_iter().enumerate() {
        let rejected = match serde_json::from_value::<StateEnvelope>(value) {
            Ok(envelope) => envelope.validate().is_err(),
            Err(_) => true,
        };
        assert!(rejected, "accepted corruption {index}");
    }
}

#[test]
fn hex_roundtrips_and_numeric_type_range_boundaries_are_checked() {
    for q in -32..=32 {
        for r in -32..=32 {
            let hex = Hex::new(q, r, -q - r).unwrap();
            let restored: Hex = serde_json::from_value(serde_json::to_value(hex).unwrap()).unwrap();
            assert_eq!(restored.coordinates(), (q, r, -q - r));
        }
    }
    for bad in [
        json!({"q":i32::MAX,"r":i32::MIN,"s":1}),
        json!({"q":1,"r":1,"s":1}),
        json!({"q":1.5,"r":0,"s":-1.5}),
        json!({"q":u64::MAX,"r":0,"s":0}),
        json!({"q":0,"r":0,"s":0,"extra":1}),
        Value::Null,
    ] {
        assert!(serde_json::from_value::<Hex>(bad).is_err());
    }
}

#[test]
fn malformed_catalog_json_and_required_shapes_are_rejected_at_read_boundary() {
    for input in [
        b"{".as_slice(),
        b"[]",
        b"null",
        b"{\"version\":1}",
        b"{\"version\":-1}",
        b"true",
    ] {
        assert!(GameContent::read(input).is_err());
    }
    let good: Value =
        serde_json::from_slice(include_bytes!("../../../content/smoke-game.json")).unwrap();
    for field in ["maps", "heroes", "tokens", "minions"] {
        for bad in [json!(true), json!(0), Value::Null, json!("bad")] {
            let mut value = good.clone();
            value[field] = bad;
            assert!(
                GameContent::read(serde_json::to_vec(&value).unwrap().as_slice()).is_err(),
                "{field}"
            );
        }
    }
}

#[test]
fn prepared_candidates_roundtrip_identity_and_never_mutate_roots_or_siblings() {
    let content = content();
    let budget = ExecutionBudget::default();
    for seed in [0, 1, 2, 17, 29, u64::MAX] {
        let root = game(&content, seed);
        let before = saved(&root);
        let view = root.client_view(Some("hero_arien")).unwrap();
        let cards = view["view"]["teams"]["RED"]["heroes"][0]["hand"]
            .as_array()
            .unwrap();
        for card in cards.iter().take(3) {
            let card = card["id"].as_str().unwrap();
            let (mut next, transition) = root
                .prepare_submission(
                    "hero_arien",
                    0,
                    PlayerCommand::Commit {
                        card_id: card.into(),
                    },
                    &budget,
                )
                .unwrap();
            assert_eq!(saved(&root), before);
            let checkpoint = saved(&next);
            let restored = LiveGame::restore(
                serde_json::from_value::<GameCheckpoint>(checkpoint.clone()).unwrap(),
                &budget,
            )
            .unwrap();
            assert_eq!(
                serde_json::to_vec(&saved(&restored)).unwrap(),
                serde_json::to_vec(&checkpoint).unwrap(),
                "ordered physical references/bindings changed at seed {seed}"
            );
            let hidden = next.client_view(Some("hero_brogan")).unwrap();
            assert!(
                hidden["view"]["teams"]["RED"]["heroes"][0]["current_turn_card"]
                    .get("id")
                    .is_none()
            );
            assert!(
                root.client_result(&transition, None).is_err(),
                "foreign transition was accepted"
            );
            assert!(
                next.submit("hero_arien", 0, PlayerCommand::Uncommit, &budget)
                    .is_err()
            );
            assert_eq!(saved(&next), checkpoint);
            let (sibling, _) = root
                .prepare_submission(
                    "hero_arien",
                    0,
                    PlayerCommand::Commit {
                        card_id: cards[1]["id"].as_str().unwrap().into(),
                    },
                    &budget,
                )
                .unwrap();
            let sibling_before = saved(&sibling);
            next.submit("hero_arien", 1, PlayerCommand::Uncommit, &budget)
                .unwrap();
            assert_eq!(sibling.position().revision, 1);
            assert_eq!(saved(&sibling), sibling_before);
            assert_eq!(saved(&root), before);
        }
        let cancelled = AtomicBool::new(true);
        for (budget, error) in [
            (
                ExecutionBudget {
                    cancelled: Some(&cancelled),
                    ..Default::default()
                },
                EngineError::Cancelled,
            ),
            (
                ExecutionBudget {
                    deadline: Some(Instant::now()),
                    ..Default::default()
                },
                EngineError::Deadline,
            ),
            (
                ExecutionBudget {
                    max_steps: 0,
                    ..Default::default()
                },
                EngineError::StepLimit,
            ),
        ] {
            let failure = root
                .prepare_submission("hero_arien", 0, PlayerCommand::Pass, &budget)
                .err()
                .unwrap();
            assert_eq!(failure, error);
            assert_eq!(saved(&root), before);
        }
    }
}
