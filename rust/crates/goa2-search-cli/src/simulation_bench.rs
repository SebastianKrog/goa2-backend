//! Matched replay timing: import, command decoding and export stay outside clocks.
use super::*;
use goa2_core::{
    cards::CardId,
    data::{Datum, Symbol},
};
use std::{hint::black_box, time::Instant};

#[derive(Clone)]
enum Command {
    Advance,
    Commit(Symbol, CardId),
    Pass(Symbol),
    Input(Datum),
}
#[derive(Default, PartialEq, Eq)]
struct Counts {
    operations: usize,
    requests: usize,
    events: usize,
}
impl Counts {
    fn json(&self) -> Value {
        json!({"operations":self.operations,"requests":self.requests,"events":self.events})
    }
}
fn prepare(root: &mut NativeSession, operations: &[Value]) -> Result<Vec<Command>, String> {
    operations
        .iter()
        .map(|op| {
            Ok(match op["kind"].as_str() {
                Some("advance") => Command::Advance,
                Some("commit" | "pass") => {
                    let Datum::Text(hero) = root
                        .import_selection(&op["hero"])
                        .map_err(|e| e.to_string())?
                    else {
                        return Err("benchmark hero must be a string".into());
                    };
                    if op["kind"] == "pass" {
                        Command::Pass(hero)
                    } else {
                        let card = op["card"]
                            .as_u64()
                            .and_then(|v| u32::try_from(v).ok())
                            .ok_or("benchmark card reference invalid")?;
                        Command::Commit(hero, CardId(card))
                    }
                }
                Some("input") => {
                    if op
                        .get("request_id")
                        .and_then(Value::as_str)
                        .is_some_and(|v| v != "current")
                    {
                        return Err("benchmark inputs must use the live request".into());
                    }
                    Command::Input(
                        root.import_selection(&op["selection"])
                            .map_err(|e| e.to_string())?,
                    )
                }
                _ => {
                    return Err(
                        "benchmark supports coalesced advance, commit, pass and live input".into(),
                    );
                }
            })
        })
        .collect()
}
fn run(session: &mut NativeSession, commands: &[Command]) -> Result<Counts, String> {
    let mut counts = Counts::default();
    for command in commands {
        match command {
            Command::Advance => {
                let output = session
                    .advance(&ExecutionBudget::default())
                    .map_err(|e| e.to_string())?;
                counts.requests += usize::from(output.request.is_some());
                counts.events += output.events.len();
                black_box(output);
            }
            Command::Commit(hero, card) => session
                .commit_card(*hero, *card)
                .map_err(|e| e.to_string())?,
            Command::Pass(hero) => session.pass_planning(*hero).map_err(|e| e.to_string())?,
            Command::Input(selection) => {
                let id = session
                    .last_request
                    .as_ref()
                    .ok_or("benchmark input without request")?
                    .id;
                session
                    .submit(id, selection.clone())
                    .map_err(|e| e.to_string())?;
            }
        }
        counts.operations += 1;
    }
    Ok(counts)
}
pub(super) fn measure_rollout(
    envelope: StateEnvelope,
    player_id: String,
    iterations: u32,
    sample_seed: Option<u64>,
    sample_count: u8,
    lab_evaluation: bool,
) -> Result<Value, String> {
    if !(1..=10_000).contains(&iterations) {
        return Err("rollout benchmark requires iterations in 1..10000".into());
    }
    if !(1..=8).contains(&sample_count)
        || u32::from(sample_count) > iterations
        || (sample_seed.is_none() && sample_count != 1)
    {
        return Err("rollout benchmark requires 1..8 samples, a seed for multiple samples and enough iterations".into());
    }
    let seeds = (0..sample_count)
        .map(|index| sample_seed.map(|seed| seed.wrapping_add(u64::from(index))))
        .collect::<Vec<_>>();
    let mut root = NativeSession::compile(envelope).map_err(|error| error.to_string())?;
    let player = super::sequence_legality::player(&mut root, player_id)?;
    let budget = ExecutionBudget::default();
    let before = root.semantic_state().map_err(|error| error.to_string())?;
    let mut run = |sample_seed, sampling_time: &mut std::time::Duration| {
        let rollout = |worker: &mut NativeSession| {
            worker.rollout_game_turns_diagnostic(player, 2, 4096, &budget, false, lab_evaluation)
        };
        let outcome = if let Some(seed) = sample_seed {
            let start = Instant::now();
            let mut rollout_time = std::time::Duration::ZERO;
            let outcome = root.with_sampled_planning(player, seed, |sampled| {
                let start = Instant::now();
                let outcome = rollout(sampled);
                rollout_time = start.elapsed();
                outcome
            });
            *sampling_time += start.elapsed() - rollout_time;
            outcome
        } else {
            rollout(&mut root)
        };
        outcome
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "rollout benchmark exhausted its node budget".to_owned())
    };
    let expected = seeds
        .iter()
        .map(|&seed| run(seed, &mut std::time::Duration::default()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut sampling_time = std::time::Duration::ZERO;
    let start = Instant::now();
    for index in 0..iterations as usize {
        let sample = index % seeds.len();
        let actual = black_box(run(seeds[sample], &mut sampling_time)?);
        if actual != expected[sample] {
            return Err("rollout benchmark diverged from its warmup".into());
        }
    }
    let elapsed = start.elapsed();
    let seconds = elapsed.as_secs_f64();
    if root.semantic_state().map_err(|error| error.to_string())? != before {
        return Err("rollout benchmark mutated its root state".into());
    }
    Ok(json!({
        "iterations": iterations,
        "turns": 2,
        "sample_seed": sample_seed,
        "lab_value": expected[0].lab_value,
        "nodes_per_iteration": expected[0].nodes,
        "sample_results": seeds.iter().zip(&expected).map(|(seed, result)| json!({
            "seed": seed, "nodes": result.nodes, "lab_value": result.lab_value,
        })).collect::<Vec<_>>(),
        "seconds": seconds,
        "rollouts_per_second": f64::from(iterations) / seconds,
        "sampling_and_restore_seconds": sampling_time.as_secs_f64(),
        "remaining_seconds": (elapsed - sampling_time).as_secs_f64(),
    }))
}

pub(super) fn measure(
    envelope: StateEnvelope,
    operations: Vec<Value>,
    iterations: u32,
    rollback: bool,
) -> Result<Value, String> {
    if !(1..=100_000).contains(&iterations) || operations.is_empty() {
        return Err("benchmark requires nonempty commands and 1..100000 iterations".into());
    }
    let mut root = NativeSession::compile(envelope).map_err(|e| e.to_string())?;
    let commands = prepare(&mut root, &operations)?;
    let before = session_record(&root, Value::Null, Vec::new(), Value::Null)?;
    let mut validation = root.clone();
    let expected = run(&mut validation, &commands)?;
    let final_record = session_record(&validation, Value::Null, Vec::new(), Value::Null)?;
    drop(validation);
    if rollback {
        let (counts, record) = root
            .with_rollback(|working| {
                Ok(run(working, &commands).and_then(|counts| {
                    Ok((
                        counts,
                        session_record(working, Value::Null, Vec::new(), Value::Null)?,
                    ))
                }))
            })
            .map_err(|e| e.to_string())??;
        if counts != expected || record != final_record {
            return Err("rollback replay diverged from clone replay".into());
        }
    }
    let mut clone_seconds = 0.0;
    let mut advance_seconds = 0.0;
    let mut cleanup_seconds = 0.0;
    let mut rollback_seconds = 0.0;
    let wall = Instant::now();
    for _ in 0..iterations {
        if rollback {
            let start = Instant::now();
            let (counts, advance) = root
                .with_rollback(|working| {
                    let start = Instant::now();
                    let counts = run(working, &commands);
                    Ok((counts, start.elapsed()))
                })
                .map_err(|e| e.to_string())?;
            rollback_seconds += start.elapsed().saturating_sub(advance).as_secs_f64();
            advance_seconds += advance.as_secs_f64();
            if counts? != expected {
                return Err("benchmark execution counts changed between rollback scopes".into());
            }
            continue;
        }
        let start = Instant::now();
        let mut working = black_box(&root).clone();
        clone_seconds += start.elapsed().as_secs_f64();
        let start = Instant::now();
        let counts = run(&mut working, &commands)?;
        advance_seconds += start.elapsed().as_secs_f64();
        if counts != expected {
            return Err("benchmark execution counts changed between clones".into());
        }
        let start = Instant::now();
        drop(black_box(working));
        cleanup_seconds += start.elapsed().as_secs_f64();
    }
    let total_seconds = wall.elapsed().as_secs_f64();
    if session_record(&root, Value::Null, Vec::new(), Value::Null)? != before {
        return Err("benchmark mutated the root session".into());
    }
    Ok(
        json!({"iterations":iterations,"clone_seconds":clone_seconds,
        "advance_seconds":advance_seconds,"cleanup_seconds":cleanup_seconds,
        "rollback_seconds":rollback_seconds,"total_seconds":total_seconds,"counts":expected.json(),
        "validation_record":final_record,"root_unchanged":true}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn case() -> (StateEnvelope, Vec<Value>) {
        let base =
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
        let index: Value = serde_json::from_reader(
            crate::fixture_cache::open(base.join("complete-games-hanu.json")).unwrap(),
        )
        .unwrap();
        let row = &index["cases"][0];
        let case: Value = serde_json::from_reader(
            crate::fixture_cache::open(base.join(row["path"].as_str().unwrap())).unwrap(),
        )
        .unwrap();
        let mut commands = case["operations"].as_array().unwrap()[..4].to_vec();
        assert!(commands.iter().all(|op| op["kind"] == "commit"));
        commands.push(json!({"kind":"advance"}));
        (
            serde_json::from_value(case["envelope"].clone()).unwrap(),
            commands,
        )
    }
    #[test]
    fn benchmark_clones_root_and_matches_ordinary_engine_replay() {
        let (envelope, commands) = case();
        let result = measure(envelope.clone(), commands.clone(), 2, false).unwrap();
        let mut ordinary = stack_trace(envelope, commands).unwrap();
        let last = ordinary["records"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap();
        last["request"] = Value::Null;
        last["events"] = json!([]);
        assert_eq!(result["validation_record"], *last);
        assert_eq!(result["counts"]["operations"], 5);
        assert_eq!(result["root_unchanged"], true);
        let (envelope, commands) = case();
        let rolled_back = measure(envelope, commands, 2, true).unwrap();
        assert_eq!(
            rolled_back["validation_record"],
            result["validation_record"]
        );
        assert_eq!(rolled_back["counts"], result["counts"]);
        assert_eq!(rolled_back["root_unchanged"], true);
        assert_eq!(rolled_back["clone_seconds"], 0.0);
    }
    #[test]
    fn benchmark_rejects_empty_invalid_and_diagnostic_work_instead_of_timing_it() {
        let (envelope, commands) = case();
        assert!(measure(envelope.clone(), commands, 0, false).is_err());
        assert!(measure(envelope.clone(), Vec::new(), 1, false).is_err());
        assert!(
            measure(
                envelope.clone(),
                vec![json!({"kind":"advance_prefix","steps":1})],
                1,
                false
            )
            .is_err()
        );
        assert!(
            measure(
                envelope,
                vec![json!({"kind":"input","selection":1})],
                1,
                false
            )
            .is_err()
        );
    }
}
