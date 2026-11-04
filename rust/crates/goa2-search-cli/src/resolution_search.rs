use goa2_core::{
    codec::StateEnvelope,
    interpreter::{
        DecisionMove, ExecutionBudget, NativeSession, PlanningSearchOptions, SearchLimits,
    },
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub(super) fn search(
    envelope: StateEnvelope,
    player_id: String,
    time_ms: u64,
    rollout_turns: Option<u8>,
    limits: SearchLimits,
) -> Result<Value, String> {
    let mut session = NativeSession::compile(envelope).map_err(|error| error.to_string())?;
    let player = super::sequence_legality::player(&mut session, player_id.clone())?;
    let budget = time_budget(time_ms)?;
    session
        .advance_to_decision(&budget)
        .map_err(|error| error.to_string())?;
    let result = session
        .search_resolution_with_turn_rollout(player, limits, rollout_turns, &budget)
        .map_err(|error| error.to_string())?;
    let DecisionMove::Input {
        request: Some(token),
        selection,
        ..
    } = result.decision
    else {
        return Err("resolution search returned a move without a live request".into());
    };
    Ok(json!({
        "decision": {
            "kind": "input",
            "player_id": player_id,
            "request_id": session.request_token(token),
            "selection": session.selection_json(&selection),
        },
        "score": result.score,
        "score_kind": result.score_kind,
        "completed_depth": result.completed_depth,
        "resolved": result.resolved,
        "nodes": result.nodes,
    }))
}

fn time_budget(time_ms: u64) -> Result<ExecutionBudget<'static>, String> {
    if !(1..=60_000).contains(&time_ms) {
        return Err("resolution search requires time_ms in 1..60000".into());
    }
    Ok(ExecutionBudget {
        deadline: Some(Instant::now() + Duration::from_millis(time_ms)),
        ..ExecutionBudget::default()
    })
}

pub(super) fn rollout(
    envelope: StateEnvelope,
    player_id: String,
    turns: u8,
    max_nodes: usize,
    time_ms: u64,
    capture_state: bool,
    sample_seed: Option<u64>,
) -> Result<Value, String> {
    let mut session = NativeSession::compile(envelope).map_err(|error| error.to_string())?;
    let player = super::sequence_legality::player(&mut session, player_id)?;
    let budget = time_budget(time_ms)?;
    let run = |worker: &mut NativeSession| {
        worker.rollout_game_turns_diagnostic(
            player,
            turns,
            max_nodes,
            &budget,
            capture_state,
            false,
        )
    };
    let result = match sample_seed {
        Some(seed) => session.with_sampled_planning(player, seed, run),
        None => run(&mut session),
    }
    .map_err(|error| error.to_string())?;
    Ok(match result {
        Some(leaf) => json!({
            "complete": true,
            "round": leaf.round,
            "turn": leaf.turn,
            "phase": leaf.phase,
            "score": leaf.score,
            "nodes": leaf.nodes,
            "state": leaf.state,
        }),
        None => json!({"complete": false}),
    })
}

pub(super) fn sampled_planning(
    envelope: StateEnvelope,
    player_id: String,
    seed: u64,
    cap: usize,
    time_ms: u64,
    options: PlanningSearchOptions,
    limits: SearchLimits,
) -> Result<Value, String> {
    let mut session = NativeSession::compile(envelope).map_err(|error| error.to_string())?;
    let player = super::sequence_legality::player(&mut session, player_id.clone())?;
    let budget = time_budget(time_ms)?;
    let result = session
        .search_sampled_planning_with_options(player, seed, cap, limits, options, &budget)
        .map_err(|error| error.to_string())?;
    Ok(json!({
        "commitment": {
            "player_id": player_id,
            "first_card": result.commitment.first.map(|card| card.0),
            "second_card": result.commitment.second.map(|card| card.0),
        },
        "score": result.matrix.score,
        "completed_rows": result.matrix.completed_rows,
        "pruned_rows": result.matrix.pruned_rows,
        "evaluated_cells": result.matrix.evaluated_cells,
        "total_cells": result.matrix.total_cells,
        "total_friendly_plans": result.total_friendly_plans,
        "total_opposing_plans": result.total_opposing_plans,
        "nodes": result.matrix.nodes,
    }))
}
