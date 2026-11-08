//! Run against generated rules-v9 setup cases from the pinned Python oracle.
#[path = "../../../test-support/json_diff.rs"]
mod json_diff;
use flate2::read::GzDecoder;
use goa2_core::{
    codec::{Phase, StateEnvelope},
    interpreter::{
        ExecutionBudget,
        live_game::{GameContent, GameMode, GameSetup, LiveGame, PlayerCommand, TimeControlConfig},
    },
};
use json_diff::difference;
use serde::Deserialize;
use std::{
    env,
    fs::File,
    io::{BufRead, BufReader},
};

#[derive(Deserialize)]
struct SetupCase {
    setup: GameSetup,
    expected: StateEnvelope,
}

#[test]
#[ignore = "requires GOA2_GAME_CONTENT and GOA2_GAME_SETUP_CASES from live_game_content.py"]
fn native_setup_matches_pinned_python() {
    let input = |name| BufReader::new(File::open(env::var(name).expect(name)).unwrap());
    let content = GameContent::read(input("GOA2_GAME_CONTENT")).unwrap();
    let cases = BufReader::new(GzDecoder::new(input("GOA2_GAME_SETUP_CASES")));
    let budget = ExecutionBudget::default();
    let mut count = 0;
    for row in cases.lines() {
        let case: SetupCase = serde_json::from_str(&row.unwrap()).unwrap();
        count += 1;
        let (game, opened) = LiveGame::create(&content, &case.setup, &budget)
            .unwrap_or_else(|error| panic!("{:?}: {error}", case.setup));
        assert_eq!(opened.position.phase, Phase::Planning);
        assert_eq!(opened.position.revision, 0);
        assert!(opened.events.is_empty());
        let checkpoint = serde_json::to_value(game.checkpoint().unwrap()).unwrap();
        let actual: StateEnvelope = serde_json::from_value(checkpoint["envelope"].clone()).unwrap();
        let expected = case.expected;
        let actual_state = actual.materialize().unwrap();
        let expected_state = expected.materialize().unwrap();
        if let Some((path, actual, expected)) = difference(&actual_state, &expected_state) {
            panic!("{:?}: {path}: Rust={actual}, Python={expected}", case.setup);
        }
        let instance_groups = |envelope: &StateEnvelope| {
            let groups = envelope.instance_bindings().unwrap();
            let mut groups = groups
                .into_iter()
                .map(|paths| {
                    let mut paths = paths
                        .iter()
                        .map(|path| serde_json::to_string(path).unwrap())
                        .collect::<Vec<_>>();
                    paths.sort();
                    paths
                })
                .collect::<Vec<_>>();
            groups.sort();
            groups
        };
        assert_eq!(
            instance_groups(&actual),
            instance_groups(&expected),
            "{:?}: physical instances",
            case.setup
        );
        let object_groups = |envelope: &StateEnvelope| {
            let mut groups = envelope
                .object_bindings
                .iter()
                .map(|binding| {
                    let mut paths = binding
                        .paths
                        .iter()
                        .map(|path| serde_json::to_string(path).unwrap())
                        .collect::<Vec<_>>();
                    paths.sort();
                    (paths, binding.kind, binding.deepcopy_shared)
                })
                .collect::<Vec<_>>();
            groups.sort_by(|left, right| left.0.cmp(&right.0));
            groups
        };
        let left = object_groups(&actual);
        let right = object_groups(&expected);
        if left != right {
            let missing = right.iter().find(|row| !left.contains(row));
            let extra = left.iter().find(|row| !right.contains(row));
            panic!(
                "{:?}: missing binding {missing:?}, extra binding {extra:?}",
                case.setup
            );
        }
        let actual_keys = actual.board_keys.as_ref().unwrap();
        let expected_keys = expected.board_keys.as_ref().unwrap();
        assert_eq!(actual_keys.len(), expected_keys.len());
        for (actual, expected) in actual_keys.iter().zip(expected_keys) {
            assert_eq!(actual, expected, "{:?}: board key", case.setup);
        }
        let mut restored =
            LiveGame::restore(serde_json::from_value(checkpoint).unwrap(), &budget).unwrap();
        assert_eq!(restored.position(), game.position());
        let mut heroes = Vec::new();
        for team in ["RED", "BLUE"] {
            for hero in actual_state["teams"][team]["heroes"].as_array().unwrap() {
                let actor = hero["id"].as_str().unwrap();
                let card_id = hero["hand"][0]["id"].as_str().unwrap().to_owned();
                restored
                    .submit(
                        actor,
                        restored.position().revision,
                        PlayerCommand::Commit { card_id },
                        &budget,
                    )
                    .unwrap_or_else(|error| panic!("{:?}: {actor}: {error}", case.setup));
                heroes.push(actor);
            }
        }
        assert!(
            heroes
                .iter()
                .any(|hero| restored.request_for(hero).is_some()),
            "{:?}: no resolution prompt after the opening commitments",
            case.setup
        );
    }
    assert!(count > 0);
    println!("Matched {count} opening states and identity graphs");
}

#[test]
#[ignore = "requires GOA2_GAME_CONTENT from live_game_content.py"]
fn timed_bot_search_respects_each_seats_clock() {
    let content = GameContent::read(BufReader::new(
        File::open(env::var("GOA2_GAME_CONTENT").unwrap()).unwrap(),
    ))
    .unwrap();
    let budget = ExecutionBudget::default();
    let (mut game, _) = LiveGame::create(
        &content,
        &GameSetup {
            map: "forgotten_island".into(),
            red_heroes: vec!["Arien".into()],
            blue_heroes: vec!["Brogan".into()],
            seed: 29,
            mode: GameMode::Long,
            cheats_enabled: false,
            tie_breaker: None,
        },
        &budget,
    )
    .unwrap();
    game.install_time_control(
        TimeControlConfig {
            planning_allowance_seconds: 1,
            resolution_allowance_seconds: 1,
            initiative_bonus_seconds: 0,
            response_grant_seconds: 0,
            initial_time_bank_seconds: 0,
            time_bank_increment_seconds: 0,
            max_time_bank_seconds: 0,
            upgrade_allowance_seconds: 0,
            automatic_turn_limit: 1,
            enforce_timeouts: true,
        },
        500,
    )
    .unwrap();
    game.ready_bot_seats(["hero_brogan"], 500).unwrap();
    assert_eq!(game.waiting_ready_players(), Some(vec!["hero_arien"]));
    assert_eq!(game.bot_search_window_ms("hero_brogan", 500, 5100), 0);

    let (running, _) = game
        .prepare_submission_at(
            "hero_arien",
            0,
            PlayerCommand::Ready { ready: true },
            &budget,
            1000,
        )
        .unwrap();
    assert_eq!(
        running.bot_search_window_ms("hero_brogan", 1000, 5100),
        1000
    );
    assert_eq!(running.bot_search_window_ms("hero_brogan", 1999, 5100), 1);
    assert_eq!(running.bot_search_window_ms("hero_brogan", 2000, 5100), 0);
    let (planned, _) = running
        .prepare_bot_submission_at(
            "hero_brogan",
            1,
            PlayerCommand::Commit {
                card_id: "mad_dash".into(),
            },
            &budget,
            1500,
        )
        .unwrap();
    assert_eq!(planned.bot_search_window_ms("hero_brogan", 1500, 5100), 0);
    assert_eq!(planned.bot_search_window_ms("hero_arien", 1500, 5100), 500);
    assert_eq!(
        serde_json::to_value(planned.checkpoint().unwrap()).unwrap()["envelope"]["state"]["clock"]
            ["human_action_seen_this_turn"],
        false
    );
}
