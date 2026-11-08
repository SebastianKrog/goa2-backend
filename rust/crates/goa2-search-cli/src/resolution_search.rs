use goa2_core::{
    codec::StateEnvelope,
    interpreter::{
        BotPlanningScope, DecisionMove, ExecutionBudget, NativeSession, PlanningRolloutSample,
        PlanningSearchOptions, SearchLimits, TeamKnowledgeMode,
    },
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub(super) fn search(
    envelope: StateEnvelope,
    player_id: String,
    budget: ExecutionBudget<'_>,
    arena_bots: Option<Vec<String>>,
    rollout_turns: Option<u8>,
    resolution_sample: Option<(PlanningRolloutSample<'_>, usize, usize)>,
    planning_sample: Option<PlanningRolloutSample<'_>>,
    limits: SearchLimits,
) -> Result<Value, String> {
    let mut session = compile_with_budget(envelope, &budget)?;
    let player = super::sequence_legality::player(&mut session, player_id.clone())?;
    session
        .advance_to_decision(&budget)
        .map_err(|error| error.to_string())?;
    let result = match (arena_bots, resolution_sample) {
        (Some(bots), Some((sample, worlds, 1))) => {
            let bots = bots.into_iter().map(|id| super::sequence_legality::player(&mut session, id))
                .collect::<Result<Vec<_>, _>>()?;
            session.search_node_limited_resolution(
                player, sample, worlds, &bots, limits, rollout_turns.unwrap_or(4), &budget,
            )
        }
        (Some(_), _) => return Err("arena v2 requires a sampled resolution root and one worker".into()),
        (None, Some(sample)) => session.search_resolution_worlds(
            player,
            sample,
            limits,
            rollout_turns,
            planning_sample,
            &budget,
        ),
        (None, None) => session.search_resolution_with_sampled_turn_rollout(
            player,
            limits,
            rollout_turns,
            planning_sample,
            &budget,
        ),
    }
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
        "stop": result.stop,
        "nodes": result.nodes,
    }))
}

pub(super) fn time_budget(
    started: Instant,
    time_ms: u64,
) -> Result<ExecutionBudget<'static>, String> {
    if !(1..=60_000).contains(&time_ms) {
        return Err("search requires time_ms in 1..60000".into());
    }
    Ok(ExecutionBudget {
        deadline: Some(started + Duration::from_millis(time_ms)),
        ..ExecutionBudget::default()
    })
}

pub(super) fn compile_with_budget(
    envelope: StateEnvelope,
    budget: &ExecutionBudget<'_>,
) -> Result<NativeSession, String> {
    let check_deadline = || {
        if budget
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err("search deadline reached during request setup".to_string());
        }
        Ok(())
    };
    check_deadline()?;
    let session = NativeSession::compile(envelope).map_err(|error| error.to_string())?;
    check_deadline()?;
    Ok(session)
}

pub(super) fn rollout(
    envelope: StateEnvelope,
    player_id: String,
    turns: u8,
    max_nodes: usize,
    budget: ExecutionBudget<'_>,
    capture_state: bool,
    capture_sampled_root: bool,
    sample_seed: Option<u64>,
) -> Result<Value, String> {
    let mut session = compile_with_budget(envelope, &budget)?;
    let player = super::sequence_legality::player(&mut session, player_id)?;
    let run = |worker: &mut NativeSession| {
        let sampled_root = capture_sampled_root
            .then(|| worker.semantic_state())
            .transpose()?;
        let leaf = worker.rollout_game_turns_diagnostic(
            player,
            turns,
            max_nodes,
            &budget,
            capture_state,
            false,
        )?;
        Ok((leaf, sampled_root))
    };
    let (result, sampled_root) = match sample_seed {
        Some(seed) => session.with_sampled_planning(player, seed, run),
        None => run(&mut session),
    }
    .map_err(|error| error.to_string())?;
    let mut reply = match result {
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
    };
    if let Some(root) = sampled_root {
        reply["sampled_root"] = root;
    }
    Ok(reply)
}

#[derive(serde::Deserialize)]
pub(super) struct BotTeam {
    mode: TeamKnowledgeMode,
    bot_ids: Vec<String>,
    worlds: Option<usize>,
}

pub(super) fn sampled_planning(
    envelope: StateEnvelope,
    player_id: String,
    bot_team: Option<BotTeam>,
    (seed, cap): (u64, usize),
    budget: ExecutionBudget<'_>,
    arena_v2: bool,
    options: PlanningSearchOptions<'_>,
    limits: SearchLimits,
) -> Result<Value, String> {
    let mut session = compile_with_budget(envelope, &budget)?;
    let player = super::sequence_legality::player(&mut session, player_id.clone())?;
    if let Some(BotTeam {
        mode,
        bot_ids,
        worlds,
    }) = bot_team
    {
        let bots = bot_ids
            .iter()
            .map(|id| super::sequence_legality::player(&mut session, id.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        let scope = BotPlanningScope {
            bots: &bots,
            mode,
            worlds: worlds.unwrap_or(1),
            seed,
        };
        let result = if arena_v2 {
            session.search_node_limited_planning(
                player, scope, cap, limits,
                PlanningSearchOptions { rollout_turns: Some(options.rollout_turns.unwrap_or(4)), ..options },
                &budget,
            )
        } else {
            session.search_bot_team_planning(player, scope, cap, limits, options, &budget)
        }.map_err(|error| error.to_string())?;
        let commitments = result
            .commitments
            .iter()
            .map(|commitment| {
                let id = bots
                    .iter()
                    .position(|&bot| bot == commitment.hero)
                    .map(|index| &bot_ids[index])
                    .ok_or("search returned a commitment outside the bot roster")?;
                Ok(json!({
                    "player_id": id,
                    "first_card": commitment.first.map(|card| card.0),
                    "second_card": commitment.second.map(|card| card.0),
                }))
            })
            .collect::<Result<Vec<_>, &str>>()?;
        return Ok(json!({
            "commitments": commitments,
            "requested_worlds": result.requested_worlds,
            "total_rows": result.total_rows,
            "probe_nodes": result.probe_nodes,
            "screened_replies": result.screened_replies,
            "prepared_worlds": result.prepared_worlds,
            "score": result.matrix.score,
            "shallow_score": result.shallow_score,
            "completed_rows": result.matrix.completed_rows,
            "pruned_rows": result.matrix.pruned_rows,
            "evaluated_cells": result.matrix.evaluated_cells,
            "total_cells": result.matrix.total_cells,
            "nodes": result.matrix.nodes,
        }));
    }
    if arena_v2 {
        return Err("arena v2 requires scoped bot planning".into());
    }
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
