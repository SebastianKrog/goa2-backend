//! JSON-lines diagnostic worker. Native search is not yet advertised.
#![forbid(unsafe_code)]
mod resolution_search;
mod sequence_legality;
mod simulation_bench;
use goa2_core::codec::StateEnvelope;
use goa2_core::geometry::{BoardGeometry, MovementQuery, TileSpec};
use goa2_core::interpreter::{EngineError, ExecutionBudget, NativeSession};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
#[cfg(test)]
#[path = "../../../test-support/parallel.rs"]
mod fixture_parallel;

#[derive(Deserialize)]
struct Request {
    schema_version: u32,
    id: Value,
    #[serde(flatten)]
    operation: Operation,
}
#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Capabilities,
    AggregateTrace {
        envelope: StateEnvelope,
        operations: Vec<Value>,
    },
    MinePaths {
        envelope: StateEnvelope,
        queries: Vec<Value>,
    },
    HexFilters {
        envelope: StateEnvelope,
        queries: Vec<Value>,
    },
    Terrain {
        envelope: StateEnvelope,
        queries: Vec<Value>,
    },
    ActionLegality {
        envelope: StateEnvelope,
        queries: Vec<Value>,
    },
    Targeting {
        envelope: StateEnvelope,
        queries: Vec<Value>,
    },
    DisplacementLegality {
        envelope: StateEnvelope,
        queries: Vec<Value>,
    },
    Stats {
        envelope: StateEnvelope,
        queries: Vec<Value>,
    },
    TravelZones {
        envelope: StateEnvelope,
        queries: Vec<Value>,
    },
    ActionTrace {
        envelope: StateEnvelope,
        operations: Vec<Value>,
    },
    TieTrace {
        envelope: StateEnvelope,
        tied: Vec<String>,
        operations: Vec<Value>,
    },
    PlanningTrace {
        envelope: StateEnvelope,
        operations: Vec<Value>,
    },
    EffectTrace {
        envelope: StateEnvelope,
        operations: Vec<Value>,
    },
    CardEquality {
        envelope: StateEnvelope,
        queries: Vec<Value>,
    },
    CardTrace {
        envelope: StateEnvelope,
        operations: Vec<Value>,
    },
    HeuristicInput {
        envelope: StateEnvelope,
        player_id: String,
    },
    EvaluateLabAttackRange {
        envelope: StateEnvelope,
        player_id: String,
    },
    SearchResolution {
        envelope: StateEnvelope,
        player_id: String,
        max_depth: u8,
        max_nodes: usize,
        time_ms: u64,
        rollout_turns: Option<u8>,
        #[serde(default)]
        opponent_policy: goa2_core::interpreter::OpponentPolicy,
    },
    SearchSampledPlanning {
        envelope: StateEnvelope,
        player_id: String,
        seed: u64,
        cap: usize,
        max_depth: u8,
        max_nodes: usize,
        time_ms: u64,
        rollout_turns: Option<u8>,
        workers: Option<usize>,
    },
    RolloutGameTurns {
        envelope: StateEnvelope,
        player_id: String,
        turns: u8,
        max_nodes: usize,
        time_ms: u64,
        #[serde(default)]
        capture_state: bool,
        sample_seed: Option<u64>,
    },
    CheckSequences {
        envelope: StateEnvelope,
        sequences: Vec<Vec<sequence_legality::WireMove>>,
    },
    RolloutBench {
        envelope: StateEnvelope,
        player_id: String,
        iterations: u32,
        sample_seed: Option<u64>,
        #[serde(default)]
        lab_evaluation: bool,
    },
    SimulationBench {
        envelope: StateEnvelope,
        operations: Vec<Value>,
        iterations: u32,
        #[serde(default)]
        rollback: bool,
    },
    StackTraceStream {
        envelope: StateEnvelope,
        operations: Vec<Value>,
    },
    StackTrace {
        envelope: StateEnvelope,
        operations: Vec<Value>,
    },
    RoundtripState {
        envelope: StateEnvelope,
    },
    ValidateExecution {
        envelope: StateEnvelope,
    },
    LoadBoard {
        tiles: Vec<TileSpec>,
    },
    Movement {
        queries: Vec<MovementQuery>,
        #[serde(default)]
        stateful: bool,
    },
}
fn execute(
    request: Request,
    board: &mut Option<(BoardGeometry, goa2_core::geometry::Occupancy)>,
) -> Result<Value, String> {
    if request.schema_version != 1 {
        return Err("unsupported protocol schema".into());
    }
    match request.operation {
        Operation::Capabilities => Ok(
            json!({"plain_fast_travel":true,"unit_selection_traces":true,"acting_piece_traces":true,"action_recipes":true,"fast_travel_zones":true,"planning_phase_traces":true,"card_models":true,"scalar_stats":true,"native_step_traces":true,"streamed_step_traces":true,"state_codec":true,"sequence_legality":true,"resolution_search":true,"plain_board_movement":true,"stateful_plain_movement":true,"game_transitions":false,"search":false,"rules_versions":[]}),
        ),
        Operation::HeuristicInput { envelope, player_id } => {
            let mut session = NativeSession::compile(envelope).map_err(|error| error.to_string())?;
            let player = sequence_legality::player(&mut session, player_id)?;
            session.advance_to_decision(&ExecutionBudget::default()).map_err(|error| error.to_string())?;
            let choice = session.heuristic_input_choice(player).map_err(|error| error.to_string())?;
            let selection = match choice {
                Some(goa2_core::interpreter::DecisionMove::Input { selection, .. }) => session.selection_json(&selection),
                _ => Value::Null,
            };
            Ok(json!({"selection": selection}))
        }
        Operation::EvaluateLabAttackRange { envelope, player_id } => {
            let mut session = NativeSession::compile(envelope).map_err(|error| error.to_string())?;
            let player = sequence_legality::player(&mut session, player_id)?;
            let value = session.evaluate_lab_attack_range(player).map_err(|error| error.to_string())?;
            Ok(json!({"recipe_id": "lab-attack-range-v1", "value": value}))
        }
        Operation::SearchResolution {
            envelope,
            player_id,
            max_depth,
            max_nodes,
            time_ms,
            rollout_turns,
            opponent_policy,
        } => resolution_search::search(
            envelope,
            player_id,
            time_ms,
            rollout_turns,
            goa2_core::interpreter::SearchLimits {
                max_depth,
                max_nodes,
                opponent_policy,
            },
        ),
        Operation::SearchSampledPlanning {
            envelope,
            player_id,
            seed,
            cap,
            max_depth,
            max_nodes,
            time_ms,
            rollout_turns,
            workers,
        } => resolution_search::sampled_planning(
            envelope,
            player_id,
            seed,
            cap,
            time_ms,
            goa2_core::interpreter::PlanningSearchOptions {
                rollout_turns,
                workers: workers.unwrap_or(1),
            },
            goa2_core::interpreter::SearchLimits {
                max_depth,
                max_nodes,
                opponent_policy: goa2_core::interpreter::OpponentPolicy::Adversarial,
            },
        ),
        Operation::RolloutGameTurns {
            envelope,
            player_id,
            turns,
            max_nodes,
            time_ms,
            capture_state,
            sample_seed,
        } => resolution_search::rollout(envelope, player_id, turns, max_nodes, time_ms, capture_state, sample_seed),
        Operation::CheckSequences {
            envelope,
            sequences,
        } => sequence_legality::check(envelope, sequences),
        Operation::RolloutBench {
            envelope,
            player_id,
            iterations,
            sample_seed,
            lab_evaluation,
        } => simulation_bench::measure_rollout(envelope, player_id, iterations, sample_seed, lab_evaluation),
        Operation::SimulationBench {
            envelope,
            operations,
            iterations,
            rollback,
        } => simulation_bench::measure(envelope, operations, iterations, rollback),
        Operation::AggregateTrace {
            envelope,
            operations,
        } => goa2_core::objects::aggregate_trace(envelope, operations),
        Operation::MinePaths { envelope, queries } => {
            goa2_core::diagnostics::mine_path_batch(envelope, queries)
        }
        Operation::HexFilters { envelope, queries } => {
            goa2_core::diagnostics::hex_filter_batch(envelope, queries)
        }
        Operation::Terrain { envelope, queries } => {
            goa2_core::diagnostics::terrain_batch(envelope, queries)
        }
        Operation::ActionLegality { envelope, queries } => {
            goa2_core::diagnostics::action_legality_batch(envelope, queries)
        }
        Operation::Targeting { envelope, queries } => {
            goa2_core::diagnostics::targeting_batch(envelope, queries)
        }
        Operation::DisplacementLegality { envelope, queries } => {
            goa2_core::diagnostics::displacement_batch(envelope, queries)
        }
        Operation::Stats { envelope, queries } => {
            goa2_core::diagnostics::stats_batch(envelope, queries)
        }
        Operation::TravelZones { envelope, queries } => {
            goa2_core::diagnostics::travel_zones(envelope, queries)
        }
        Operation::ActionTrace {
            envelope,
            operations,
        } => goa2_core::diagnostics::action_trace(envelope, operations),
        Operation::TieTrace {
            envelope,
            tied,
            operations,
        } => goa2_core::diagnostics::tie_trace(envelope, tied, operations),
        Operation::PlanningTrace {
            envelope,
            operations,
        } => goa2_core::diagnostics::planning_trace(envelope, operations),
        Operation::EffectTrace {
            envelope,
            operations,
        } => goa2_core::diagnostics::effect_trace(envelope, operations),
        Operation::CardEquality { envelope, queries } => {
            goa2_core::diagnostics::card_equality_batch(envelope, queries)
        }
        Operation::CardTrace {
            envelope,
            operations,
        } => goa2_core::diagnostics::card_trace(envelope, operations),
        Operation::StackTraceStream { .. } => Err("streamed trace requires an output sink".into()),
        Operation::StackTrace {
            envelope,
            operations,
        } => stack_trace(envelope, operations),
        Operation::RoundtripState { envelope } => {
            envelope.validate()?;
            Ok(json!({"semantic_state":envelope.materialize()?,"envelope":envelope}))
        }
        Operation::ValidateExecution { envelope } => {
            envelope.require_native_execution()?;
            Err("execution capability must not be advertised before transition parity".into())
        }
        Operation::LoadBoard { tiles } => {
            *board = Some(BoardGeometry::compile(&tiles)?);
            Ok(json!({"tiles":tiles.len()}))
        }
        Operation::Movement { queries, stateful } => {
            let (geometry, occupancy) = board.as_ref().ok_or("load_board required")?;
            Ok(
                json!({"results": queries.iter().map(|q| if stateful {geometry.stateful_paths(occupancy,q)}else{geometry.movement_paths(occupancy,q)}).collect::<Vec<_>>() }),
            )
        }
    }
}
/// Diagnostic boundary; the interpreter itself consumes no JSON.
fn stack_trace(envelope: StateEnvelope, operations: Vec<Value>) -> Result<Value, String> {
    let mut records = Vec::new();
    replay_trace(envelope, operations, |record| records.push(record))?;
    Ok(json!({"records":records}))
}
/// Fixture verification consumes one complete operation snapshot at a time.
/// The worker adapter above retains its existing collected wire response.
fn replay_trace(
    envelope: StateEnvelope,
    operations: Vec<Value>,
    mut record_ready: impl FnMut(Value),
) -> Result<(), String> {
    replay_trace_checked(envelope, operations, |record| {
        record_ready(record);
        Ok(())
    })
}
fn replay_trace_checked(
    envelope: StateEnvelope,
    operations: Vec<Value>,
    mut record_ready: impl FnMut(Value) -> Result<(), String>,
) -> Result<(), String> {
    let mut session = NativeSession::compile(envelope).map_err(|e| e.to_string())?;
    for operation in operations {
        let mut request = Value::Null;
        let mut events = Vec::new();
        let mut error = Value::Null;
        match operation["kind"].as_str() {
            Some("advance" | "advance_prefix") => {
                let result = if operation["kind"] == "advance_prefix" {
                    let steps = operation["steps"]
                        .as_u64()
                        .and_then(|v| usize::try_from(v).ok())
                        .filter(|v| *v <= 1000)
                        .ok_or("invalid diagnostic step prefix")?;
                    session.advance_prefix(steps)
                } else {
                    session.advance(&ExecutionBudget::default())
                }
                .map_err(|e| e.to_string())?;
                request = result
                    .request
                    .as_ref()
                    .map_or(Value::Null, |r| session.request_json(r));
                events = result
                    .events
                    .iter()
                    .map(|e| session.event_json(e))
                    .collect();
            }
            Some("commit" | "uncommit" | "pass" | "finish") => {
                let goa2_core::data::Datum::Text(hero) = session
                    .import_selection(&operation["hero"])
                    .map_err(|e| e.to_string())?
                else {
                    return Err("planning hero must be a string".into());
                };
                let result = match operation["kind"].as_str() {
                    Some("commit") => {
                        let card = operation["card"]
                            .as_u64()
                            .and_then(|n| u32::try_from(n).ok())
                            .ok_or("invalid planning card reference")?;
                        session.commit_card(hero, goa2_core::cards::CardId(card))
                    }
                    Some("uncommit") => session.uncommit_card(hero),
                    Some("pass") => session.pass_planning(hero),
                    Some("finish") => session.finish_planning(hero),
                    _ => unreachable!(),
                };
                match result {
                    Ok(()) => {}
                    Err(EngineError::InvalidInput(_)) => error = json!("invalid_planning_action"),
                    Err(e) => return Err(e.to_string()),
                }
            }
            Some("start_revelation") => {
                session
                    .start_revelation()
                    .map_err(|error| error.to_string())?;
            }
            Some("next_actor") => {
                session
                    .resolve_next_actor()
                    .map_err(|error| error.to_string())?;
            }
            Some("end_turn") => {
                session.end_turn().map_err(|error| error.to_string())?;
            }
            Some("after_cards_played") => {
                session
                    .queue_after_cards_played()
                    .map_err(|error| error.to_string())?;
            }
            Some("expire_turn_effects") => {
                session
                    .expire_turn_effects()
                    .map_err(|error| error.to_string())?;
            }
            Some("expire_effects") => {
                let duration = goa2_core::effects::Duration::parse(
                    operation["duration"].as_str().ok_or("duration missing")?,
                )?;
                session
                    .expire_effects(duration)
                    .map_err(|error| error.to_string())?;
            }
            Some("remove_effects") => {
                let mut ids = Vec::new();
                for value in operation["ids"].as_array().ok_or("effect ids missing")? {
                    let goa2_core::data::Datum::Text(id) = session
                        .import_selection(value)
                        .map_err(|error| error.to_string())?
                    else {
                        return Err("effect id must be text".into());
                    };
                    ids.push(id);
                }
                session
                    .remove_effects(&ids)
                    .map_err(|error| error.to_string())?;
            }
            Some("queue_steps") => {
                let path: Vec<goa2_core::codec::ObjectPathPart> =
                    serde_json::from_value(operation["path"].clone())
                        .map_err(|error| error.to_string())?;
                session
                    .queue_steps(&path)
                    .map_err(|error| error.to_string())?;
            }
            Some("push_action_context") => {
                let mut values = Vec::new();
                for key in ["action_type", "card", "owner"] {
                    let goa2_core::data::Datum::Text(value) = session
                        .import_selection(&operation[key])
                        .map_err(|error| error.to_string())?
                    else {
                        return Err("action scope arguments must be strings".into());
                    };
                    values.push(value);
                }
                session
                    .push_action_context(values[0], values[1], values[2])
                    .map_err(|error| error.to_string())?;
            }
            Some("input") => {
                let token = match operation["request_id"].as_str() {
                    Some("current") | None => session
                        .last_request
                        .as_ref()
                        .map(|r| r.id)
                        .ok_or("no current request")?,
                    Some(id) => session.import_request_token(id),
                };
                let selection = session
                    .import_selection(&operation["selection"])
                    .map_err(|e| e.to_string())?;
                match session.submit(token, selection) {
                    Ok(()) => {}
                    Err(EngineError::InvalidInput(_)) => error = json!("invalid_input"),
                    Err(e) => return Err(e.to_string()),
                }
            }
            _ => return Err("unknown trace operation".into()),
        }
        record_ready(session_record(&session, request, events, error)?)?;
    }
    Ok(())
}
fn session_record(
    session: &NativeSession,
    request: Value,
    events: Vec<Value>,
    error: Value,
) -> Result<Value, String> {
    let envelope = session.export_envelope().map_err(|e| e.to_string())?;
    let mut record = json!({"semantic_state":envelope.materialize()?,"request":request,"events":events,"error":error});
    if !envelope.object_bindings.is_empty() {
        record["context_keys"] = json!(
            envelope.state["execution_context"]
                .as_object()
                .ok_or("execution context missing")?
                .keys()
                .collect::<Vec<_>>()
        );
        record["instance_bindings"] = serde_json::to_value(envelope.instance_bindings()?)
            .map_err(|error| error.to_string())?;
        record["generated_step_objects"] = serde_json::to_value(
            session
                .generated_step_objects(&envelope.object_bindings)
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        record["object_bindings"] =
            serde_json::to_value(envelope.object_bindings).map_err(|e| e.to_string())?;
    }
    Ok(record)
}
fn write_packet(out: &mut impl Write, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *out, value)?;
    writeln!(out)?;
    out.flush()
}
fn stack_trace_stream(
    id: Value,
    envelope: StateEnvelope,
    operations: Vec<Value>,
    out: &mut impl Write,
) -> io::Result<()> {
    let mut count = 0;
    let result = replay_trace_checked(envelope, operations, |record| {
        write_packet(out,&json!({"schema_version":1,"id":id,"ok":true,"result":{"sequence":count,"record":record}})).map_err(|e|e.to_string())?;
        count += 1;
        Ok(())
    });
    let response = match result {
        Ok(()) => {
            json!({"schema_version":1,"id":id,"ok":true,"result":{"complete":true,"records":count}})
        }
        Err(error) => json!({"schema_version":1,"id":id,"ok":false,"error":error,"records":count}),
    };
    write_packet(out, &response)
}
fn main() -> io::Result<()> {
    let mut board = None;
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let line = line?;
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(Request {
                schema_version: 1,
                id,
                operation:
                    Operation::StackTraceStream {
                        envelope,
                        operations,
                    },
            }) => {
                stack_trace_stream(id, envelope, operations, &mut out)?;
                continue;
            }
            Ok(request) => {
                let id = request.id.clone();
                match execute(request, &mut board) {
                    Ok(result) => json!({"schema_version":1,"id":id,"ok":true,"result":result}),
                    Err(error) => json!({"schema_version":1,"id":id,"ok":false,"error":error}),
                }
            }
            Err(error) => {
                json!({"schema_version":1,"id":null,"ok":false,"error":error.to_string()})
            }
        };
        serde_json::to_writer(&mut out, &response)?;
        writeln!(out)?;
        out.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn stream_case() -> (StateEnvelope, Vec<Value>) {
        let base =
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
        let index: Value = serde_json::from_reader(std::io::BufReader::new(
            crate::fixture_cache::open(base.join("object-execution.json")).unwrap(),
        ))
        .unwrap();
        let entry = index["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == "object-execution-shared-pending-step")
            .unwrap();
        let mut case: Value = serde_json::from_reader(std::io::BufReader::new(
            crate::fixture_cache::open(base.join(entry["path"].as_str().unwrap())).unwrap(),
        ))
        .unwrap();
        (
            serde_json::from_value(case["envelope"].take()).unwrap(),
            serde_json::from_value(case["operations"].take()).unwrap(),
        )
    }
    #[test]
    fn streamed_packets_preserve_collected_records_order_identity_and_completion_count() {
        let (envelope, operations) = stream_case();
        let collected = stack_trace(envelope.clone(), operations.clone()).unwrap();
        let mut bytes = Vec::new();
        stack_trace_stream(json!("stream-test"), envelope, operations, &mut bytes).unwrap();
        let rows = bytes
            .split(|b| *b == b'\n')
            .filter(|r| !r.is_empty())
            .map(|r| serde_json::from_slice::<Value>(r).unwrap())
            .collect::<Vec<_>>();
        let expected = collected["records"].as_array().unwrap();
        assert_eq!(rows.len(), expected.len() + 1);
        for (i, (packet, record)) in rows.iter().zip(expected).enumerate() {
            assert_eq!(packet["id"], "stream-test");
            assert_eq!(packet["schema_version"], 1);
            assert_eq!(packet["result"]["sequence"], i);
            assert_eq!(packet["result"]["record"], *record);
        }
        assert_eq!(
            rows.last().unwrap()["result"],
            json!({"complete":true,"records":expected.len()})
        );
    }
    #[test]
    fn stream_failure_reports_the_successful_prefix_and_one_terminal_error() {
        let (envelope, _) = stream_case();
        let mut bytes = Vec::new();
        stack_trace_stream(
            json!(7),
            envelope,
            vec![
                json!({"kind":"advance_prefix","steps":0}),
                json!({"kind":"unknown"}),
            ],
            &mut bytes,
        )
        .unwrap();
        let rows = bytes
            .split(|b| *b == b'\n')
            .filter(|r| !r.is_empty())
            .map(|r| serde_json::from_slice::<Value>(r).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["result"]["sequence"], 0);
        assert_eq!(rows[1]["ok"], false);
        assert_eq!(rows[1]["records"], 1);
    }
    #[test]
    fn failed_stream_consumer_stops_replay_before_following_operations() {
        let (envelope, _) = stream_case();
        let mut calls = 0;
        let result = replay_trace_checked(
            envelope,
            vec![
                json!({"kind":"advance_prefix","steps":0}),
                json!({"kind":"unknown"}),
            ],
            |_| {
                calls += 1;
                Err("disconnected".into())
            },
        );
        assert_eq!(calls, 1);
        assert_eq!(result.unwrap_err(), "disconnected");
    }
    #[test]
    fn protocol_rejects_unknown_fields_and_preserves_prior_board_on_error() {
        assert!(
            serde_json::from_value::<Request>(
                json!({"schema_version":1,"id":1,"operation":"load_board","tiles":[],"typo":true})
            )
            .is_err()
        );
        let mut board = None;
        let request = serde_json::from_value(
            json!({"schema_version":1,"id":1,"operation":"load_board","tiles":[]}),
        )
        .unwrap();
        execute(request, &mut board).unwrap();
        let bad = serde_json::from_value(
            json!({"schema_version":99,"id":2,"operation":"load_board","tiles":[]}),
        )
        .unwrap();
        assert!(execute(bad, &mut board).is_err());
        assert!(board.is_some());
    }
}

#[cfg(test)]
mod trace_tests {
    use super::*;

    use std::collections::HashMap;
    fn normalize_id(value: &mut Value, ids: &mut HashMap<String, String>) {
        if let Some(id) = value.as_str() {
            let next = format!("request/{}", ids.len() + 1);
            *value = Value::String(ids.entry(id.into()).or_insert(next).clone());
        }
    }
    fn normalize_step_fields(
        step: &mut Value,
        ids: &mut HashMap<String, String>,
        steps: &mut HashMap<String, String>,
        identity: Option<Option<usize>>,
    ) {
        let key = match identity {
            Some(Some(id)) => Some(format!("object/{id}")),
            Some(None) => None,
            None => step
                .get("step_id")
                .and_then(Value::as_str)
                .filter(|id| id.starts_with("native-step-"))
                .map(str::to_owned),
        };
        if let Some(key) = key {
            let next = format!("dynamic-step/{}", steps.len() + 1);
            step["step_id"] = Value::String(steps.entry(key).or_insert(next).clone());
        }
        normalize_id(&mut step["pending_request_id"], ids);
    }
    fn normalize_step(
        step: &mut Value,
        ids: &mut HashMap<String, String>,
        steps: &mut HashMap<String, String>,
    ) {
        normalize_step_fields(step, ids, steps, None);
        if let Some(children) = step.get_mut("steps_template").and_then(Value::as_array_mut) {
            for child in children {
                normalize_step(child, ids, steps);
            }
        }
    }
    fn normalize_record(
        record: &mut Value,
        ids: &mut HashMap<String, String>,
        steps: &mut HashMap<String, String>,
    ) {
        if !record["request"].is_null() {
            normalize_id(&mut record["request"]["id"], ids);
        }
        let generated = record
            .as_object_mut()
            .unwrap()
            .remove("generated_step_objects")
            .map(|value| {
                serde_json::from_value::<Vec<(usize, usize)>>(value)
                    .unwrap()
                    .into_iter()
                    .collect::<HashMap<usize, usize>>()
            });
        // Borrow disjoint fields rather than clone every identity/path row.
        let (mut semantic, mut bindings) = (None, None);
        for (key, value) in record.as_object_mut().unwrap() {
            match key.as_str() {
                "semantic_state" => semantic = Some(value),
                "object_bindings" => bindings = Some(value),
                _ => {}
            }
        }
        let semantic = semantic.unwrap();
        if let Some(bindings) = bindings.and_then(|value| value.as_array()) {
            for (index, binding) in bindings.iter().enumerate() {
                if binding["kind"] != "step" {
                    continue;
                }
                for path in binding["paths"].as_array().unwrap() {
                    let mut step = &mut *semantic;
                    for part in path.as_array().unwrap() {
                        step = if let Some(key) = part.as_str() {
                            &mut step[key]
                        } else {
                            &mut step[part.as_u64().unwrap() as usize]
                        };
                    }
                    normalize_step_fields(
                        step,
                        ids,
                        steps,
                        generated.as_ref().map(|ids| ids.get(&index).copied()),
                    );
                }
            }
        } else {
            for step in semantic["execution_stack"].as_array_mut().unwrap() {
                normalize_step(step, ids, steps);
            }
        }
    }
    #[derive(Deserialize)]
    struct TraceCase {
        name: String,
        envelope: StateEnvelope,
        operations: Vec<Value>,
        expected: Vec<Value>,
        #[serde(default)]
        unordered_mine_keys: Vec<String>,
    }
    fn verify_case(case: TraceCase) {
        let TraceCase {
            name,
            envelope,
            operations,
            expected,
            unordered_mine_keys,
        } = case;
        let mut ids = HashMap::new();
        let mut steps = HashMap::new();
        let mut checked = 0;
        replay_trace(envelope, operations, |mut record| {
            if !unordered_mine_keys.is_empty() {
                goa2_core::diagnostics::normalize_mine_choice_record(
                    &mut record,
                    &unordered_mine_keys,
                );
            }
            normalize_record(&mut record, &mut ids, &mut steps);
            let want = expected
                .get(checked)
                .unwrap_or_else(|| panic!("{name}: unexpected operation {checked}"));
            assert!(
                record == *want,
                "{name} operation {checked} differs from the Python state/request/event snapshot"
            );
            checked += 1;
        })
        .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            checked,
            expected.len(),
            "{name}: missing operation snapshots"
        );
    }
    #[derive(Deserialize)]
    #[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
    enum SnapshotEdit {
        Set {
            path: Vec<goa2_core::codec::ObjectPathPart>,
            value: Value,
        },
        Delete {
            path: Vec<goa2_core::codec::ObjectPathPart>,
        },
        Splice {
            path: Vec<goa2_core::codec::ObjectPathPart>,
            index: usize,
            delete: usize,
            values: Vec<Value>,
        },
    }
    fn snapshot_target<'a>(
        mut target: &'a mut Value,
        path: &[goa2_core::codec::ObjectPathPart],
    ) -> Result<&'a mut Value, String> {
        use goa2_core::codec::ObjectPathPart;
        for part in path {
            target = match part {
                ObjectPathPart::Key(key) => target.as_object_mut().and_then(|m| m.get_mut(key)),
                ObjectPathPart::Index(i) => target.as_array_mut().and_then(|a| a.get_mut(*i)),
            }
            .ok_or("snapshot parent missing")?;
        }
        Ok(target)
    }
    fn apply_snapshot_edits(record: &mut Value, edits: Vec<SnapshotEdit>) -> Result<(), String> {
        use goa2_core::codec::ObjectPathPart;
        for edit in edits {
            if let SnapshotEdit::Splice {
                path,
                index,
                delete,
                values,
            } = edit
            {
                let target = snapshot_target(record, &path)?
                    .as_array_mut()
                    .ok_or("snapshot splice target is not an array")?;
                let end = index
                    .checked_add(delete)
                    .filter(|end| *end <= target.len())
                    .ok_or("snapshot splice range outside array")?;
                target.splice(index..end, values);
                continue;
            }
            let (path, value) = match edit {
                SnapshotEdit::Set { path, value } => (path, Some(value)),
                SnapshotEdit::Delete { path } => (path, None),
                SnapshotEdit::Splice { .. } => unreachable!(),
            };
            let Some((last, parents)) = path.split_last() else {
                *record = value.ok_or("cannot delete snapshot root")?;
                continue;
            };
            let target = snapshot_target(record, parents)?;
            match (last, value) {
                (ObjectPathPart::Key(key), Some(value)) => {
                    target
                        .as_object_mut()
                        .ok_or("snapshot target is not a dictionary")?
                        .insert(key.clone(), value);
                }
                (ObjectPathPart::Key(key), None) => {
                    target
                        .as_object_mut()
                        .ok_or("snapshot delete target is not a dictionary")?
                        .remove(key)
                        .ok_or("snapshot delete key missing")?;
                }
                (ObjectPathPart::Index(i), Some(value)) => {
                    *target
                        .as_array_mut()
                        .and_then(|a| a.get_mut(*i))
                        .ok_or("snapshot index outside array")? = value;
                }
                (ObjectPathPart::Index(_), None) => {
                    return Err("snapshot array deletion is not supported".into());
                }
            }
        }
        Ok(())
    }
    #[test]
    fn snapshot_edits_preserve_null_values_nested_arrays_and_literal_keys() {
        let mut record = json!({"a/b~":[{"old":1},2]});
        let edits = serde_json::from_value(json!([
            {"op":"set","path":["a/b~",0,"new"],"value":null},
            {"op":"delete","path":["a/b~",0,"old"]},
            {"op":"set","path":["a/b~",1],"value":[3,4]}
        ]))
        .unwrap();
        apply_snapshot_edits(&mut record, edits).unwrap();
        assert_eq!(record, json!({"a/b~":[{"new":null},[3,4]]}));
        let splice = serde_json::from_value(
            json!([{"op":"splice","path":["a/b~"],"index":1,"delete":1,"values":[3,4,5]}]),
        )
        .unwrap();
        apply_snapshot_edits(&mut record, splice).unwrap();
        assert_eq!(record, json!({"a/b~":[{"new":null},3,4,5]}));
        let invalid = serde_json::from_value(
            json!([{"op":"splice","path":["a/b~"],"index":9,"delete":0,"values":[]}]),
        )
        .unwrap();
        assert!(apply_snapshot_edits(&mut record, invalid).is_err());

        let invalid =
            serde_json::from_value(json!([{"op":"set","path":["missing",0],"value":3}])).unwrap();
        assert!(apply_snapshot_edits(&mut record, invalid).is_err());
    }
    #[test]
    fn complete_games_match_python_at_every_operation_using_lossless_snapshot_deltas() {
        assert_compact_games("complete-games.json", 2);
    }
    #[test]
    fn hanu_complete_games_match_python_at_every_operation_with_shared_models_and_turns() {
        assert_compact_games("complete-games-hanu.json", 2);
    }
    fn assert_compact_games(index_name: &str, expected_cases: usize) {
        #[derive(Deserialize)]
        struct Entry {
            name: String,
            path: String,
        }
        #[derive(Deserialize)]
        struct Index {
            schema_version: u32,
            contract: String,
            cases: Vec<Entry>,
        }
        #[derive(Deserialize)]
        struct Chunk {
            path: String,
            records: usize,
            sha256: String,
        }
        #[derive(Deserialize)]
        struct Case {
            name: String,
            envelope: StateEnvelope,
            operations: Vec<Value>,
            expected_delta_chunks: Vec<Chunk>,
        }
        let base =
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
        let index: Index = serde_json::from_reader(std::io::BufReader::new(
            crate::fixture_cache::open(base.join(index_name)).unwrap(),
        ))
        .unwrap();
        assert_eq!(index.schema_version, 2);
        assert_eq!(index.contract, "complete-games-snapshot-deltas-v3");
        assert_eq!(index.cases.len(), expected_cases);
        fixture_parallel::for_each_case(index.cases.into_iter(), |entry| {
            let case: Case = serde_json::from_reader(std::io::BufReader::new(
                crate::fixture_cache::open(base.join(entry.path)).unwrap(),
            ))
            .unwrap();
            assert_eq!(case.name, entry.name);
            assert_eq!(
                case.operations.len(),
                case.expected_delta_chunks
                    .iter()
                    .map(|c| c.records)
                    .sum::<usize>()
            );
            let mut expected = Value::Null;
            let mut ids = HashMap::new();
            let mut steps = HashMap::new();
            let mut count = 0;
            let mut chunks = case.expected_delta_chunks.into_iter();
            let mut deltas = std::collections::VecDeque::new();
            replay_trace(case.envelope, case.operations, |mut record| {
                if deltas.is_empty() {
                    let chunk = chunks.next().expect("missing snapshot delta chunk");
                    assert_eq!(chunk.sha256.len(), 64);
                    assert!(chunk.path.ends_with(&format!("/{}.json", chunk.sha256)));
                    let rows: Vec<Vec<SnapshotEdit>> =
                        serde_json::from_reader(std::io::BufReader::new(
                            crate::fixture_cache::open(base.join(chunk.path)).unwrap(),
                        ))
                        .unwrap();
                    assert_eq!(rows.len(), chunk.records);
                    deltas = rows.into();
                }
                apply_snapshot_edits(&mut expected, deltas.pop_front().unwrap()).unwrap();
                normalize_record(&mut record, &mut ids, &mut steps);
                assert!(
                    record == expected,
                    "{} operation {count} differs from Python",
                    case.name
                );
                count += 1;
            })
            .unwrap_or_else(|error| panic!("{}: {error}", case.name));
            assert!(deltas.is_empty() && chunks.next().is_none());
            assert_eq!(expected["semantic_state"]["phase"], "GAME_OVER");
        });
    }
    #[test]
    fn live_shared_steps_and_context_match_python_values_requests_and_identity() {
        verify_index("object-execution.json", Some(17347));
    }
    #[test]
    fn every_native_transition_matches_python_full_state_request_and_event_order() {
        verify_index("stack-traces.json", None);
    }
    fn verify_index(filename: &str, expected_count: Option<usize>) {
        #[derive(Deserialize)]
        struct Entry {
            name: String,
            path: String,
        }
        #[derive(Deserialize)]
        struct Index {
            schema_version: u32,
            cases: Vec<Entry>,
        }
        let base =
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
        let index: Index = serde_json::from_reader(std::io::BufReader::new(
            crate::fixture_cache::open(base.join(filename)).unwrap(),
        ))
        .unwrap();
        assert_eq!(index.schema_version, 2);
        assert!(!index.cases.is_empty());
        if let Some(count) = expected_count {
            assert_eq!(index.cases.len(), count);
        }
        let total = index.cases.len();
        let checked = fixture_parallel::for_each_case(index.cases.into_iter(), |entry| {
            let case: TraceCase = serde_json::from_reader(std::io::BufReader::new(
                crate::fixture_cache::open(base.join(entry.path)).unwrap(),
            ))
            .unwrap();
            assert_eq!(case.name, entry.name);
            verify_case(case);
        });
        assert_eq!(checked, total, "every oracle case must run");
    }
}

#[cfg(test)]
#[path = "../../../test-support/fixture_cache.rs"]
mod fixture_cache;
