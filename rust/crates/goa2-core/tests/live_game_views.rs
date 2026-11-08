//! Differential checks for recipient-scoped frontend payloads.
#[path = "../../../test-support/json_diff.rs"]
mod json_diff;

use flate2::read::GzDecoder;
use goa2_core::{
    codec::StateEnvelope,
    interpreter::{ExecutionBudget, live_game::LiveGame},
};
use serde::Deserialize;
use serde_json::Value;
use std::{
    env,
    fs::File,
    io::{BufRead, BufReader},
};

#[derive(Deserialize)]
struct Case {
    name: String,
    envelope: StateEnvelope,
    events: Vec<Value>,
    projections: Vec<Projection>,
}

#[derive(Deserialize)]
struct Projection {
    viewer: Option<String>,
    expected: Value,
    expected_events: Vec<Value>,
}

#[test]
#[ignore = "requires GOA2_GAME_VIEW_CASES from live_game_views.py"]
fn native_frontend_projections_match_pinned_python() {
    let file = File::open(env::var("GOA2_GAME_VIEW_CASES").unwrap()).unwrap();
    let cases = BufReader::new(GzDecoder::new(BufReader::new(file)));
    let budget = ExecutionBudget::default();
    let mut projections = 0;
    let mut mismatches = Vec::new();
    for line in cases.lines() {
        let case: Case = serde_json::from_str(&line.unwrap()).unwrap();
        let (game, mut transition) = LiveGame::import(case.envelope, &budget)
            .unwrap_or_else(|error| panic!("{}: {error}", case.name));
        transition.events = case.events;
        for mut row in case.projections {
            projections += 1;
            let viewer = row.viewer.as_deref();
            let view = game.client_view(viewer).unwrap();

            if let Some(request) = row.expected["input_request"].as_object_mut() {
                request.shift_remove("can_rollback");
            }

            if let Some((path, native, python)) = json_diff::difference(&view, &row.expected) {
                mismatches.push(format!(
                    "{} {viewer:?}: {path}: Rust={native}, Python={python}",
                    case.name
                ));
            }
            let result = game.client_result(&transition, viewer).unwrap();
            assert_eq!(
                result["input_request"], view["input_request"],
                "{} {viewer:?}",
                case.name
            );
            assert_eq!(
                result["awaiting_input"], view["awaiting_input"],
                "{} {viewer:?}",
                case.name
            );
            assert_eq!(
                result["events"].as_array().unwrap(),
                &row.expected_events,
                "{} {viewer:?}",
                case.name
            );
        }
    }
    assert!(projections > 0);
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
    println!("Matched {projections} scoped frontend projections");
}
