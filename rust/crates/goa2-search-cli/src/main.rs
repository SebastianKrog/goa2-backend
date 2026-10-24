//! JSON-lines diagnostic worker. Native search is not yet advertised.
#![forbid(unsafe_code)]
use goa2_core::codec::StateEnvelope;
use goa2_core::geometry::{BoardGeometry, MovementQuery, TileSpec};
use goa2_core::interpreter::{EngineError, ExecutionBudget, NativeSession};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};

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
    let mut session = NativeSession::compile(envelope).map_err(|e| e.to_string())?;
    let mut records = Vec::new();
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
        records.push(json!({"semantic_state":session.semantic_state().map_err(|e|e.to_string())?,"request":request,"events":events,"error":error}));
    }
    Ok(json!({"records":records}))
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
    fn normalize_step(
        step: &mut Value,
        ids: &mut HashMap<String, String>,
        steps: &mut HashMap<String, String>,
    ) {
        if let Some(id) = step
            .get("step_id")
            .and_then(Value::as_str)
            .filter(|id| id.starts_with("native-step-"))
        {
            let next = format!("dynamic-step/{}", steps.len() + 1);
            step["step_id"] = Value::String(steps.entry(id.to_owned()).or_insert(next).clone());
        }
        normalize_id(&mut step["pending_request_id"], ids);
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
    }
    fn verify_case(case: TraceCase) {
        let mut actual = stack_trace(case.envelope, case.operations)
            .unwrap_or_else(|error| panic!("{}: {error}", case.name));
        let mut ids = HashMap::new();
        let mut steps = HashMap::new();
        for record in actual["records"].as_array_mut().unwrap() {
            if !record["request"].is_null() {
                normalize_id(&mut record["request"]["id"], &mut ids);
            }
            for step in record["semantic_state"]["execution_stack"]
                .as_array_mut()
                .unwrap()
            {
                normalize_step(step, &mut ids, &mut steps);
            }
        }
        let actual = actual["records"].as_array().unwrap();
        assert_eq!(actual.len(), case.expected.len());
        for (index, (got, want)) in actual.iter().zip(&case.expected).enumerate() {
            assert!(
                got == want,
                "{} operation {index} differs from the Python state/request/event snapshot",
                case.name
            );
        }
    }
    #[test]
    fn every_native_transition_matches_python_full_state_request_and_event_order() {
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
            std::fs::File::open(base.join("stack-traces.json")).unwrap(),
        ))
        .unwrap();
        assert_eq!(index.schema_version, 2);
        assert!(!index.cases.is_empty());
        for entry in index.cases {
            let case: TraceCase = serde_json::from_reader(std::io::BufReader::new(
                std::fs::File::open(base.join(entry.path)).unwrap(),
            ))
            .unwrap();
            assert_eq!(case.name, entry.name);
            verify_case(case);
        }
    }
}
