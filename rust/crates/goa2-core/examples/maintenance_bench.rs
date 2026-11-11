//! Fixed-work maintenance measurements, separate from strength evaluation.
use goa2_core::{
    api::experimental_search::{NativeSession, OpponentPolicy, SearchEvaluation, SearchLimits},
    api::{ExecutionBudget, GameContent, GameMode, GameSetup, LiveGame, PlayerCommand},
    codec::StateEnvelope,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    hint::black_box,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn memory() -> BTreeMap<String, u64> {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            if ["VmRSS", "VmHWM", "VmSize"].contains(&key) {
                Some((
                    key.to_owned(),
                    value.split_whitespace().next()?.parse().ok()?,
                ))
            } else {
                None
            }
        })
        .collect()
}
fn measure(mut run: impl FnMut(), iterations: usize) -> Value {
    let start = Instant::now();
    for _ in 0..iterations {
        run();
    }
    let nanos = start.elapsed().as_nanos();
    json!({"iterations":iterations,"total_ns":nanos,"ns_per_operation":nanos as f64/iterations as f64})
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("pass a trusted content pack path")?;
    let memory_start = memory();
    let started = Instant::now();
    let content = GameContent::read(File::open(path)?)?;
    let content_read_ns = started.elapsed().as_nanos();
    let catalog_start = Instant::now();
    NativeSession::prepare_search()?;
    let catalog_first_ns = catalog_start.elapsed().as_nanos();
    let catalog_warm = measure(|| NativeSession::prepare_search().unwrap(), 100);
    let setup = GameSetup {
        map: "forgotten_island".into(),
        red_heroes: vec!["Arien".into(), "Brogan".into()],
        blue_heroes: vec!["Tigerclaw".into(), "Sabina".into()],
        seed: 29,
        mode: GameMode::Quick,
        cheats_enabled: false,
        tie_breaker: None,
    };
    let budget = ExecutionBudget::default();
    let setup_start = Instant::now();
    let (root, _) = LiveGame::create(&content, &setup, &budget)?;
    let setup_first_ns = setup_start.elapsed().as_nanos();
    let memory_after_setup = memory();
    let checkpoint = serde_json::to_vec(&root.checkpoint()?)?;
    let encoded: Value = serde_json::from_slice(&checkpoint)?;
    let envelope: StateEnvelope = serde_json::from_value(encoded["envelope"].clone())?;
    let mut session = NativeSession::compile(envelope)?;
    let goa2_core::data::Datum::Text(player) = session.import_selection(&json!("hero_arien"))?
    else {
        return Err("missing benchmark player".into());
    };
    let clone_work = measure(
        || {
            black_box(session.clone());
        },
        200,
    );
    let memory_after_clones = memory();
    let restore_work = measure(
        || {
            black_box(
                LiveGame::restore(serde_json::from_slice(&checkpoint).unwrap(), &budget).unwrap(),
            );
        },
        40,
    );
    let prepare_candidate = || {
        black_box(
            root.prepare_submission(
                "hero_arien",
                0,
                PlayerCommand::Commit {
                    card_id: "aspiring_duelist".into(),
                },
                &budget,
            )
            .unwrap(),
        );
    };
    const WARM_SUBMISSIONS: usize = 99;
    let start = Instant::now();
    prepare_candidate();
    let first = Instant::now();
    for _ in 0..WARM_SUBMISSIONS {
        prepare_candidate();
    }
    let end = Instant::now();
    let total_ns = (end - start).as_nanos();
    let warm_ns = (end - first).as_nanos();
    let candidate_work = json!({
        "iterations": WARM_SUBMISSIONS + 1,
        "total_ns": total_ns,
        "ns_per_operation": total_ns as f64 / (WARM_SUBMISSIONS + 1) as f64,
        "first_ns": (first - start).as_nanos(),
        "warm_iterations": WARM_SUBMISSIONS,
        "warm_total_ns": warm_ns,
        "warm_ns_per_operation": warm_ns as f64 / WARM_SUBMISSIONS as f64,
    });
    let setup_work = measure(
        || {
            black_box(LiveGame::create(&content, &setup, &budget).unwrap());
        },
        20,
    );
    let before = serde_json::to_vec(&root.checkpoint()?)?;
    let limits = SearchLimits {
        max_depth: 1,
        max_nodes: 256,
        opponent_policy: OpponentPolicy::Adversarial,
        evaluation: SearchEvaluation::MaterialPosition,
    };
    let search_start = Instant::now();
    let mut decisions = Vec::new();
    for _ in 0..4 {
        let result = session.search_sampled_planning(player, 29, 2, limits, &budget)?;
        decisions.push(json!({"commitment":format!("{:?}",result.commitment),"score":result.matrix.score,"nodes":result.matrix.nodes,"evaluated_cells":result.matrix.evaluated_cells,"completed_rows":result.matrix.completed_rows}));
    }
    let search_ns = search_start.elapsed().as_nanos();
    assert!(decisions.windows(2).all(|pair| pair[0] == pair[1]));
    let memory_after_search = memory();
    // Request cooperative cancellation during the supported bot policy search.
    // A completed-before-signal call is reported, never called an interruption.
    let cancelled = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&cancelled);
    let signal = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(10));
        let at = Instant::now();
        flag.store(true, Ordering::Relaxed);
        at
    });
    let cancel_budget = ExecutionBudget {
        cancelled: Some(&cancelled),
        ..Default::default()
    };
    let cancellation_result = root.recommend_bot("hero_arien", &cancel_budget);
    let stopped = Instant::now();
    let signalled = signal.join().unwrap();
    let completed_before_signal = stopped < signalled;
    let observed_cancelled = matches!(
        cancellation_result,
        Err(goa2_core::api::EngineError::Cancelled)
    );
    if !completed_before_signal && !observed_cancelled {
        return Err("bot cancellation did not propagate".into());
    }
    let cancellation_ns =
        observed_cancelled.then(|| stopped.saturating_duration_since(signalled).as_nanos());
    let decision_start = Instant::now();
    let timed = root
        .recommend_bot(
            "hero_arien",
            &ExecutionBudget {
                deadline: Some(Instant::now() + Duration::from_millis(5)),
                ..Default::default()
            },
        )?
        .ok_or("missing timed decision")?;
    let timed_ns = decision_start.elapsed().as_nanos();
    assert_eq!(serde_json::to_vec(&root.checkpoint()?)?, before);
    println!(
        "{}",
        json!({"schema_version":1,"workload":"maintenance-v1","content_identity":content.identity(),"setup":setup,"content_read_ns":content_read_ns,"catalog_first_prepare_ns":catalog_first_ns,"catalog_warm_prepare":catalog_warm,"setup_first_ns":setup_first_ns,"setup_work":setup_work,"clone_drop_work":clone_work,"restore_parse_work":restore_work,"prepare_commit_work":candidate_work,"checkpoint_bytes":checkpoint.len(),"fixed_search":{"iterations":4,"total_ns":search_ns,"max_nodes_per_search":256,"cap":2,"seed":29,"workers":1,"max_depth":1,"opponent_policy":"adversarial","evaluation":"material_position","result":decisions[0]},"timed_decision":{"budget_ms":5,"elapsed_ns":timed_ns,"used_fallback":timed.used_fallback,"strength_claimed":false},"cancellation":{"signal_delay_ms":10,"response_ns":cancellation_ns,"completed_before_signal":completed_before_signal,"observed_cancelled":observed_cancelled,"scope":"Cooperative bot-search response; no hard instruction preemption guarantee."},"memory_kib":{"process_start":memory_start,"after_setup":memory_after_setup,"after_clones":memory_after_clones,"after_search":memory_after_search,"final":memory()}})
    );
    Ok(())
}
