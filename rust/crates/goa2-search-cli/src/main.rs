//! JSON-lines diagnostic worker. Native search is not yet advertised.
#![forbid(unsafe_code)]
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
            json!({"plain_fast_travel":true,"unit_selection_traces":true,"acting_piece_traces":true,"action_recipes":true,"fast_travel_zones":true,"planning_phase_traces":true,"card_models":true,"scalar_stats":true,"native_step_traces":true,"state_codec":true,"plain_board_movement":true,"stateful_plain_movement":true,"game_transitions":false,"search":false,"rules_versions":[]}),
        ),
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
        record_ready(record);
    }
    Ok(())
}
fn main() -> io::Result<()> {
    let mut board = None;
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let line = line?;
        let response = match serde_json::from_str::<Request>(&line) {
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
            if !record["request"].is_null() {
                normalize_id(&mut record["request"]["id"], &mut ids);
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
                            &mut ids,
                            &mut steps,
                            generated.as_ref().map(|ids| ids.get(&index).copied()),
                        );
                    }
                }
            } else {
                for step in semantic["execution_stack"].as_array_mut().unwrap() {
                    normalize_step(step, &mut ids, &mut steps);
                }
            }
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
    #[test]
    fn live_shared_steps_and_context_match_python_values_requests_and_identity() {
        verify_index("object-execution.json", Some(13766));
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
            std::fs::File::open(base.join(filename)).unwrap(),
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
                std::fs::File::open(base.join(entry.path)).unwrap(),
            ))
            .unwrap();
            assert_eq!(case.name, entry.name);
            verify_case(case);
        });
        assert_eq!(checked, total, "every oracle case must run");
    }
}
