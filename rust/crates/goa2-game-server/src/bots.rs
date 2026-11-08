//! One independent, bounded bot decision at a time per game.
use crate::{
    ApiError, Result, blocking,
    game::{Game, GameEvent, Server, wait_for_shutdown},
};
use axum::http::StatusCode;
use goa2_core::{
    codec::Phase,
    interpreter::{EngineError, ExecutionBudget, live_game::BotDecision},
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

const DECISION_TIME_MS: u64 = 5_000;
const SUBMISSION_MARGIN_MS: u64 = 100;

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BotSpec {
    Rust,
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
    loop {
        if *retired.borrow() || *shutdown.borrow() {
            return Ok(());
        }
        let snapshot = game.snapshot()?;
        let position = snapshot.position();
        let revision = position.revision;
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
        if game.snapshot()?.position().revision != revision {
            continue;
        }

        let stopped = Arc::new(AtomicBool::new(false));
        let actor = player.clone();
        let cancel = Arc::clone(&stopped);
        let clock_game = Arc::clone(game);
        let mut search = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let window = snapshot.bot_search_window_ms(
                &actor,
                clock_game.clock_now(),
                DECISION_TIME_MS + SUBMISSION_MARGIN_MS,
            );
            if window == 0 {
                return Ok(None);
            }
            let search = Duration::from_millis(window.saturating_sub(SUBMISSION_MARGIN_MS));
            let budget = ExecutionBudget {
                deadline: Some(Instant::now() + search),
                cancelled: Some(&cancel),
                ..ExecutionBudget::default()
            };
            snapshot.recommend_bot(&actor, &budget)
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
        let decision = match picked {
            Ok(Some(decision)) => decision,
            Ok(None) | Err(EngineError::Cancelled) => {
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
        match apply(server, game, player, revision, decision).await {
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
    player: String,
    revision: u64,
    decision: BotDecision,
) -> Result<()> {
    let (server, game) = (Arc::clone(server), Arc::clone(game));
    blocking(move || {
        let submitted = game.submit(
            &server.store,
            &player,
            Some(revision),
            None,
            decision.command,
        );
        let mut used_fallback = decision.used_fallback;
        if let Err(error) = submitted {
            if error.status != StatusCode::UNPROCESSABLE_ENTITY {
                return Err(error);
            }
            let Some(fallback) = decision.fallback else {
                return Err(error);
            };
            game.submit(&server.store, &player, Some(revision), None, fallback)?;
            used_fallback = true;
        }
        if used_fallback {
            eprintln!(
                "goa2-game-server: native legal fallback for {} revision {revision}",
                game.id()
            );
        }
        Ok(())
    })
    .await
}
