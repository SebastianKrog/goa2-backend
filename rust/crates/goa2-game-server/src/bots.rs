//! Parallel team planning and bounded bot decisions for live games.
use crate::{
    ApiError, Result, blocking,
    game::{Game, GameEvent, Server, wait_for_shutdown},
};
use axum::http::StatusCode;
use goa2_core::{
    api::{BotDecision, EngineError, ExecutionBudget, LiveGame},
    codec::Phase,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{broadcast::Receiver, watch};

const PLANNING_TIME_MS: u64 = 10_000;
const RESOLUTION_TIME_MS: u64 = 5_000;
const OTHER_TIME_MS: u64 = 1_750;
const SUBMISSION_MARGIN_MS: u64 = 100;
const PLANNING_SUBMISSION_MARGIN_MS: u64 = 350;
const PLANNING_STACK_BYTES: usize = 8 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BotSpec {
    Rust,
}

#[derive(Default)]
struct BotBatch {
    decisions: Vec<(String, BotDecision)>,
    await_human_until: Option<Instant>,
}

impl Server {
    pub fn start_bots(self: &Arc<Self>, game: Arc<Game>) {
        if !game.has_bots() {
            return;
        }
        let server = Arc::clone(self);
        let shutdown = self.shutdown.subscribe();
        tokio::spawn(async move {
            if let Err(error) = run(&server, &game, shutdown).await {
                eprintln!(
                    "goa2-game-server: bot loop for {} stopped: {error}",
                    game.id()
                );
            }
        });
    }
}

async fn run(
    server: &Arc<Server>,
    game: &Arc<Game>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let mut updates = game.updates.subscribe();
    let mut retired = game.retired.subscribe();
    let mut planning_window = None;
    loop {
        if *retired.borrow() || *shutdown.borrow() {
            return Ok(());
        }
        let snapshot = game.snapshot_async().await?;
        let position = snapshot.position();
        let revision = position.revision;
        if position.phase != Phase::Planning {
            planning_window = None;
        }
        if position.phase == Phase::GameOver {
            // A table correction can reopen this game without reloading it.
            drop(snapshot);
            changed(&mut updates, &mut retired, &mut shutdown, revision).await;
            continue;
        }
        let now = game.clock_now();
        let player = snapshot
            .decision_players()?
            .into_iter()
            .find(|hero| game.is_bot(hero) && snapshot.bot_search_window_ms(hero, now, 1) > 0)
            .map(str::to_owned);
        let Some(player) = player else {
            changed(&mut updates, &mut retired, &mut shutdown, revision).await;
            continue;
        };
        let permit = tokio::select! {
            permit = server.bot_workers.clone().acquire_owned() => permit.map_err(ApiError::internal)?,
            () = changed(&mut updates, &mut retired, &mut shutdown, revision) => continue,
        };
        if game.snapshot_async().await?.position().revision != revision {
            continue;
        }

        let stopped = Arc::new(AtomicBool::new(false));
        let planning_deadline = if position.phase == Phase::Planning {
            let epoch = (position.round, position.turn);
            let (_, deadline) = planning_window
                .filter(|(key, _)| *key == epoch)
                .unwrap_or_else(|| {
                    (
                        epoch,
                        Instant::now()
                            + Duration::from_millis(
                                PLANNING_TIME_MS - PLANNING_SUBMISSION_MARGIN_MS,
                            ),
                    )
                });
            planning_window = Some((epoch, deadline));
            Some(deadline)
        } else {
            None
        };
        let cancel = Arc::clone(&stopped);
        let clock_game = Arc::clone(game);
        let mut search = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let now = clock_game.clock_now();
            let roster = clock_game.bot_ids().collect::<Vec<_>>();
            if let Some(deadline) = planning_deadline {
                return plan_teams(&snapshot, &roster, now, deadline, &cancel);
            }
            let max_ms = if position.phase == Phase::Resolution {
                RESOLUTION_TIME_MS
            } else {
                OTHER_TIME_MS
            };
            let Some(search) = search_time(&snapshot, &player, now, max_ms, SUBMISSION_MARGIN_MS)
            else {
                return Ok(BotBatch::default());
            };
            let budget = ExecutionBudget {
                deadline: Some(Instant::now() + search),
                cancelled: Some(&cancel),
                ..ExecutionBudget::default()
            };
            let decision = snapshot.recommend_bot_resolution(&player, &roster, &budget)?;
            Ok(BotBatch {
                decisions: decision
                    .map(|decision| (player, decision))
                    .into_iter()
                    .collect(),
                await_human_until: None,
            })
        });
        let picked = tokio::select! {
            result = &mut search => result.map_err(ApiError::internal)?,
            () = changed(&mut updates, &mut retired, &mut shutdown, revision) => {
                stopped.store(true, Ordering::Relaxed);
                let _ = search.await.map_err(ApiError::internal)?;
                continue;
            }
        };
        if *shutdown.borrow() {
            return Ok(());
        }
        let batch = match picked {
            Ok(batch) if !batch.decisions.is_empty() => batch,
            Ok(_) | Err(EngineError::Cancelled) => {
                changed(&mut updates, &mut retired, &mut shutdown, revision).await;
                continue;
            }
            Err(error) => {
                eprintln!(
                    "goa2-game-server: bot decision for {} revision {revision}: {error}",
                    game.id()
                );
                changed(&mut updates, &mut retired, &mut shutdown, revision).await;
                continue;
            }
        };
        if let Some(deadline) = batch.await_human_until {
            tokio::select! {
                () = tokio::time::sleep_until(deadline.into()) => {}
                () = changed(&mut updates, &mut retired, &mut shutdown, revision) => continue,
            }
        }
        match apply(server, game, revision, batch.decisions).await {
            Ok(()) => {}
            Err(error) if error.status == StatusCode::CONFLICT => {}
            Err(error) => {
                eprintln!(
                    "goa2-game-server: bot submission for {} revision {revision}: {error}",
                    game.id()
                );
                if error.status.is_server_error() {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                } else {
                    changed(&mut updates, &mut retired, &mut shutdown, revision).await;
                }
            }
        }
    }
}

fn search_time(
    snapshot: &LiveGame,
    hero: &str,
    now: u64,
    max_ms: u64,
    submission_ms: u64,
) -> Option<Duration> {
    let window = snapshot.bot_search_window_ms(hero, now, max_ms);
    (window != 0).then(|| Duration::from_millis(window.saturating_sub(submission_ms)))
}

fn plan_teams(
    snapshot: &LiveGame,
    roster: &[&str],
    now: u64,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> std::result::Result<BotBatch, EngineError> {
    let eligible = snapshot.decision_players()?;
    let started = Instant::now();
    let teams = snapshot
        .bot_teams(roster)?
        .into_iter()
        .filter_map(|team| {
            let seat_window = team
                .iter()
                .filter(|id| eligible.contains(id))
                .filter_map(|id| {
                    search_time(
                        snapshot,
                        id,
                        now,
                        PLANNING_TIME_MS,
                        PLANNING_SUBMISSION_MARGIN_MS,
                    )
                })
                .min()?;
            Some((team, seat_window))
        })
        .collect::<Vec<_>>();
    // Both plans are published together. A timed seat can shorten the batch
    // so its completed command is ready before its clock expires.
    let deadline = teams
        .iter()
        .fold(deadline, |end, (_, window)| end.min(started + *window));
    let mut awaits_human = false;
    for (team, _) in &teams {
        awaits_human |= snapshot.bot_team_has_open_human(team)?;
    }
    std::thread::scope(|scope| {
        let jobs = teams
            .into_iter()
            .map(|(team, _)| {
                std::thread::Builder::new()
                    .name("live-team-planning".into())
                    .stack_size(PLANNING_STACK_BYTES)
                    .spawn_scoped(scope, move || {
                        let budget = ExecutionBudget {
                            deadline: Some(deadline),
                            cancelled: Some(cancelled),
                            ..ExecutionBudget::default()
                        };
                        snapshot.recommend_bot_team_plan(&team, &budget)
                    })
                    .map_err(|error| {
                        EngineError::Unsupported(format!("could not start bot team: {error}"))
                    })
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut decisions = Vec::new();
        for job in jobs {
            decisions.extend(job.join().map_err(|_| {
                EngineError::Unsupported("live bot planning worker panicked".into())
            })??);
        }
        Ok(BotBatch {
            decisions,
            await_human_until: awaits_human.then_some(deadline),
        })
    })
}

async fn changed(
    updates: &mut Receiver<GameEvent>,
    retired: &mut watch::Receiver<bool>,
    shutdown: &mut watch::Receiver<bool>,
    revision: u64,
) {
    loop {
        tokio::select! {
            _ = wait_for_shutdown(shutdown) => return,
            _ = retired.wait_for(|retired| *retired) => return,
            update = updates.recv() => match update {
                Ok(GameEvent::State(update)) if update.revision.position().revision <= revision => {}
                Ok(GameEvent::OverrideNotice(_) | GameEvent::PlayerLinkRevealed(_)) => {}
                _ => return,
            }
        }
    }
}

async fn apply(
    server: &Arc<Server>,
    game: &Arc<Game>,
    mut revision: u64,
    decisions: Vec<(String, BotDecision)>,
) -> Result<()> {
    let (server, game) = (Arc::clone(server), Arc::clone(game));
    blocking(move || {
        // Journal each command through the ordinary path. If a human changes
        // the revision between commands, the rest of this plan is stale.
        for (player, decision) in decisions {
            let mut changed_plan = false;
            let submission = match game.submit(
                &server.store,
                &player,
                Some(revision),
                None,
                decision.command,
            ) {
                Ok(submission) => submission,
                Err(error) if error.status == StatusCode::UNPROCESSABLE_ENTITY => {
                    let Some(fallback) = decision.fallback else {
                        return Err(error);
                    };
                    changed_plan = true;
                    game.submit(&server.store, &player, Some(revision), None, fallback)?
                }
                Err(error) => return Err(error),
            };
            if changed_plan || decision.used_fallback {
                eprintln!(
                    "goa2-game-server: native legal fallback for {} revision {revision}",
                    game.id()
                );
            }
            if changed_plan {
                // A substituted card may change the legality of the following slot.
                break;
            }
            revision = submission.reply["revision"]
                .as_u64()
                .ok_or_else(|| ApiError::internal("Bot submission has no revision"))?;
        }
        Ok(())
    })
    .await
}
