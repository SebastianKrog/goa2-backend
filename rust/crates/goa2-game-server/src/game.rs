mod voting;
pub use voting::cancelled_notice;

use crate::{
    ApiError, Result,
    bots::BotSpec,
    history::{History, HistoryContext},
    replay::{self, Archive, Decision, Operation},
    store::{AcceptedAction, Credentials, PlayerToken, SavedGame, Store},
};
use axum::http::StatusCode;
use goa2_core::api::{
    ExecutionBudget, GameCheckpoint, GameContent, GameSetup, GameTransition, LiveGame,
    PauseVoteChange, PlayerCommand, TimeControlConfig,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Semaphore, broadcast, watch};
use uuid::Uuid;

fn wall_clock_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub async fn wait_for_shutdown(receiver: &mut watch::Receiver<bool>) {
    // The watch borrow must be dropped before any caller awaits further work.
    let _ = receiver.wait_for(|stopping| *stopping).await;
}

pub struct Server {
    pub content: GameContent,
    pub identity: Value,
    pub store: Store,
    games: RwLock<HashMap<String, Arc<Game>>>,
    lab_token: Option<String>,
    hosted_token: Option<String>,
    hosted_creations: Mutex<()>,
    pub bot_workers: Arc<Semaphore>,
    pub share_baker: Arc<Semaphore>,
    pub shutdown: watch::Sender<bool>,
}

pub struct Game {
    id: String,
    credentials: Credentials,
    current: Mutex<Arc<LiveGame>>,
    voting: Mutex<voting::Voting>,
    pub updates: broadcast::Sender<GameEvent>,
    pub presence: broadcast::Sender<crate::presence::Notice>,
    pub retired: watch::Sender<bool>,
}

#[derive(Clone)]
pub enum GameEvent {
    State(Update),
    OverrideNotice(Arc<Value>),
    PlayerLinkRevealed(String),
}

#[derive(Clone)]
pub struct Update {
    pub revision: Arc<LiveGame>,
    transition: Arc<GameTransition>,
    actor: String,
    action_id: Option<String>,
}

pub struct Submission {
    pub reply: Value,
    pub replayed: bool,
}

impl Server {
    pub fn new(
        content: GameContent,
        identity: Value,
        store: Store,
        lab_token: Option<String>,
        hosted_token: Option<String>,
    ) -> Result<Self> {
        let mut games = HashMap::new();
        for saved in store.load()? {
            let mut engine = LiveGame::restore(saved.checkpoint, &ExecutionBudget::default())
                .map_err(ApiError::internal)?;
            if engine.position().revision != saved.revision {
                return Err(ApiError::internal(
                    "Stored game revision does not match its checkpoint",
                ));
            }
            let id = saved.id;
            let interrupted = engine.pause_vote_hold().map(str::to_owned);
            if interrupted.is_some() {
                let now = saved
                    .credentials
                    .fixed_clock_ms
                    .unwrap_or_else(wall_clock_ms);
                let (resumed, _) = engine.prepare_pause_vote(PauseVoteChange::Cancel, now)?;
                store.commit(
                    &id,
                    saved.revision,
                    resumed.position().revision,
                    &resumed.checkpoint()?,
                    None,
                    Decision::pause(now, PauseVoteChange::Cancel),
                    HistoryContext::new(&engine.position(), None),
                )?;
                engine = resumed;
            }
            games.insert(
                id.clone(),
                Arc::new(Game::new(
                    id,
                    saved.credentials,
                    engine,
                    interrupted.as_deref(),
                )),
            );
        }
        Ok(Self {
            content,
            identity,
            store,
            games: RwLock::new(games),
            lab_token,
            hosted_token,
            hosted_creations: Mutex::new(()),
            bot_workers: Arc::new(Semaphore::new(2)),
            share_baker: Arc::new(Semaphore::new(1)),
            shutdown: watch::channel(false).0,
        })
    }

    fn find_game(&self, id: &str) -> Result<Option<Arc<Game>>> {
        Ok(self
            .games
            .read()
            .map_err(ApiError::internal)?
            .get(id)
            .cloned())
    }

    pub fn game(&self, id: &str) -> Result<Arc<Game>> {
        self.find_game(id)?
            .ok_or_else(|| ApiError::not_found("Game not found"))
    }

    pub fn staff_replay(&self, id: &str) -> Result<Archive> {
        match self.find_game(id)? {
            Some(game) => game.replay(&self.store),
            None => self.store.reported_replay(id),
        }
    }

    pub fn staff_share(&self, id: &str) -> Result<String> {
        if let Some(game) = self.find_game(id)? {
            return game.share(&self.store);
        }
        if let Some(token) = self.store.share_for_game(id)? {
            return Ok(token);
        }
        let share = crate::shares::Share::bake(id, self.store.reported_replay(id)?)?;
        self.store.save_share(id, share)
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        let game = self.game(id)?;
        let _guard = game.current.lock().map_err(ApiError::internal)?;
        game.check_active()?;
        self.store.remove(id)?;
        game.retired.send_replace(true);
        self.games.write().map_err(ApiError::internal)?.remove(id);
        Ok(())
    }

    pub fn require_lab(&self, token: Option<&str>) -> Result<()> {
        if self.lab_token.is_none() {
            return Err(ApiError::not_found("Lab routes are disabled"));
        }
        if self.lab_token.as_deref() != token {
            return Err(ApiError::unauthorized("Invalid lab token"));
        }
        Ok(())
    }

    pub fn require_hosted(&self, token: Option<&str>) -> Result<()> {
        let expected = self
            .hosted_token
            .as_deref()
            .ok_or_else(|| ApiError::not_found("Hosted provisioning is disabled"))?;
        if token != Some(expected) {
            return Err(ApiError::unauthorized("Invalid provisioning token"));
        }
        Ok(())
    }

    pub fn require_local_creation(&self) -> Result<()> {
        if self.hosted_token.is_some() {
            return Err(ApiError::forbidden(
                "Hosted games are created through a draft",
            ));
        }
        Ok(())
    }

    pub fn resume_workers(self: &Arc<Self>) -> Result<()> {
        for game in self.games.read().map_err(ApiError::internal)?.values() {
            self.start_bots(Arc::clone(game));
            self.start_deadlines(Arc::clone(game))?;
        }
        Ok(())
    }

    fn start_deadlines(self: &Arc<Self>, game: Arc<Game>) -> Result<()> {
        if game.credentials.fixed_clock_ms.is_some() || !game.snapshot()?.enforces_deadlines() {
            return Ok(());
        }
        let server = Arc::clone(self);
        let mut retired = game.retired.subscribe();
        let mut shutdown = self.shutdown.subscribe();
        let mut updates = game.updates.subscribe();
        tokio::spawn(async move {
            while !*retired.borrow() && !*shutdown.borrow() {
                let Ok(engine) = game.snapshot_async().await else {
                    break;
                };
                let delay = engine.clock_deadline(game.clock_now());
                drop(engine);
                let wake = async {
                    match delay {
                        Some(delay) => tokio::time::sleep(Duration::from_millis(delay)).await,
                        None => std::future::pending().await,
                    }
                };
                tokio::select! {
                    _ = wait_for_shutdown(&mut shutdown) => break,
                    _ = retired.changed() => break,
                    _ = updates.recv() => {},
                    _ = wake => {
                        let server = Arc::clone(&server);
                        let game = Arc::clone(&game);
                        let result = crate::blocking(move || game.drive_deadline(&server.store)).await;
                        if let Err(error) = result {
                            eprintln!("goa2-game-server: timer failed: {error}");
                            tokio::time::sleep(Duration::from_millis(250)).await;
                        }
                    }
                }
            }
        });
        Ok(())
    }

    pub fn create(
        self: &Arc<Self>,
        setup: &GameSetup,
        names: BTreeMap<String, String>,
        bots: BTreeMap<String, BotSpec>,
        time_control: Option<TimeControlConfig>,
        provisioning: Option<(String, String)>,
    ) -> Result<Value> {
        // Keep the existing-ID check and SQLite insertion in the same critical section.
        let _creation = if let Some((id, hash)) = &provisioning {
            let guard = self.hosted_creations.lock().map_err(ApiError::internal)?;
            let games = self.games.read().map_err(ApiError::internal)?;
            if let Some(game) = games.get(id) {
                if game.credentials.provisioning_hash.as_ref() != Some(hash) {
                    return Err(ApiError::conflict("Provisioning ID is already in use"));
                }
                return Ok(game.creation_reply());
            }
            Some(guard)
        } else {
            None
        };
        let (mut engine, _) = LiveGame::create(&self.content, setup, &ExecutionBudget::default())?;
        if let Some(config) = time_control {
            engine.install_time_control(config, wall_clock_ms())?;
        }
        self.register(engine, names, bots, None, provisioning)
    }

    pub fn restore(
        self: &Arc<Self>,
        checkpoint: GameCheckpoint,
        names: BTreeMap<String, String>,
        bots: BTreeMap<String, BotSpec>,
        freeze_clock: bool,
    ) -> Result<Value> {
        let mut engine = Self::restore_engine(checkpoint)?;
        let fixed = (freeze_clock && engine.uses_clock()).then(|| engine.clock_timestamp());
        if engine.pause_vote_hold().is_some() {
            engine = engine
                .prepare_pause_vote(PauseVoteChange::Cancel, fixed.unwrap_or_else(wall_clock_ms))?
                .0;
        }
        self.register(engine, names, bots, fixed, None)
    }

    pub fn restore_engine(checkpoint: GameCheckpoint) -> Result<LiveGame> {
        Ok(LiveGame::restore(checkpoint, &ExecutionBudget::default())?)
    }

    fn register(
        self: &Arc<Self>,
        mut engine: LiveGame,
        names: BTreeMap<String, String>,
        bots: BTreeMap<String, BotSpec>,
        fixed_clock_ms: Option<u64>,
        provisioning: Option<(String, String)>,
    ) -> Result<Value> {
        if fixed_clock_ms.is_some() && !bots.is_empty() {
            return Err(ApiError::invalid(
                "Frozen timed fixtures require manual seats",
            ));
        }
        if bots
            .keys()
            .any(|id| !engine.players().any(|(hero, _)| id == hero))
        {
            return Err(ApiError::invalid("A bot seat is outside the game roster"));
        }
        if !bots.is_empty() {
            engine.ready_bot_seats(bots.keys().map(String::as_str), wall_clock_ms())?;
        }
        let (id, provisioning_hash) = match provisioning {
            Some((id, hash)) => (id, Some(hash)),
            None => (format!("rust-{}", Uuid::new_v4().simple()), None),
        };
        let player_tokens = engine
            .players()
            .map(|(hero_id, _)| PlayerToken {
                hero_id: hero_id.into(),
                token: Uuid::new_v4().simple().to_string(),
            })
            .collect::<Vec<_>>();
        let names = engine
            .players()
            .filter_map(|(id, hero)| {
                let name = names.get(hero).or_else(|| names.get(id))?;
                let cleaned: String = name.trim().chars().take(20).collect();
                (!cleaned.is_empty()).then(|| (id.to_owned(), cleaned))
            })
            .collect();
        let credentials = Credentials {
            player_tokens,
            spectator_token: Uuid::new_v4().simple().to_string(),
            hero_names: names,
            bots,
            fixed_clock_ms,
            provisioning_hash,
        };
        let revision = engine.position().revision;
        let saved = SavedGame {
            id: id.clone(),
            revision,
            credentials,
            checkpoint: engine.checkpoint()?,
        };
        self.store.create(&saved)?;
        let game = Arc::new(Game::new(id.clone(), saved.credentials, engine, None));
        self.games
            .write()
            .map_err(ApiError::internal)?
            .insert(id, Arc::clone(&game));
        self.start_deadlines(Arc::clone(&game))?;
        let reply = game.game_links(revision);
        self.start_bots(game);
        Ok(reply)
    }
}

impl Game {
    fn new(
        id: String,
        credentials: Credentials,
        engine: LiveGame,
        interrupted: Option<&str>,
    ) -> Self {
        let (updates, _) = broadcast::channel(32);
        let (presence, _) = broadcast::channel(32);
        let (retired, _) = watch::channel(false);
        Self {
            id,
            credentials,
            current: Mutex::new(Arc::new(engine)),
            voting: Mutex::new(voting::Voting::new(interrupted)),
            updates,
            presence,
            retired,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn has_bots(&self) -> bool {
        !self.credentials.bots.is_empty()
    }

    pub fn is_bot(&self, hero: &str) -> bool {
        self.credentials.bots.contains_key(hero)
    }

    pub fn player_link(&self, hero: &str) -> Option<Value> {
        self.credentials.player_tokens.iter().find(|seat| seat.hero_id == hero).map(|seat| {
            json!({"type": "PLAYER_LINK_REVEALED", "hero_id": hero, "token": seat.token})
        })
    }

    pub fn has_seat(&self, hero: &str) -> bool {
        self.credentials
            .player_tokens
            .iter()
            .any(|seat| seat.hero_id == hero)
    }

    pub fn viewer(&self, token: &str) -> Result<Option<String>> {
        if let Some(seat) = self
            .credentials
            .player_tokens
            .iter()
            .find(|seat| seat.token == token)
        {
            return Ok(Some(seat.hero_id.clone()));
        }
        if self.credentials.spectator_token == token {
            return Ok(None);
        }
        Err(ApiError::unauthorized("Invalid game token"))
    }

    fn creation_reply(&self) -> Value {
        // Provisioned games start at revision zero. Retries return the original links.
        self.game_links(0)
    }

    fn game_links(&self, revision: u64) -> Value {
        json!({
            "game_id": self.id,
            "player_tokens": self.credentials.player_tokens,
            "spectator_token": self.credentials.spectator_token,
            "revision": revision,
        })
    }

    pub fn spectator_link(&self) -> Value {
        json!({"game_id": self.id, "spectator_token": self.credentials.spectator_token})
    }

    fn check_active(&self) -> Result<()> {
        if *self.retired.borrow() {
            return Err(ApiError::not_found("Game not found"));
        }
        Ok(())
    }

    // A command holds current through its durable SQLite commit. Async workers
    // must not block a Tokio executor thread while waiting for that mutex.
    pub async fn snapshot_async(self: &Arc<Self>) -> Result<Arc<LiveGame>> {
        let game = Arc::clone(self);
        crate::blocking(move || game.snapshot()).await
    }

    pub fn snapshot(&self) -> Result<Arc<LiveGame>> {
        let current = self.current.lock().map_err(ApiError::internal)?;
        self.check_active()?;
        Ok(Arc::clone(&current))
    }

    pub fn clock_now(&self) -> u64 {
        self.credentials
            .fixed_clock_ms
            .unwrap_or_else(wall_clock_ms)
    }

    pub fn view(&self, engine: &LiveGame, viewer: Option<&str>) -> Result<Value> {
        let mut view = engine.client_view_at(viewer, self.clock_now())?;
        view["hero_names"] = json!(self.credentials.hero_names);
        view["revision"] = json!(engine.position().revision);
        Ok(view)
    }

    pub fn state_update(&self, engine: &LiveGame, viewer: Option<&str>) -> Result<Value> {
        let mut view = self.view(engine, viewer)?;
        view["type"] = json!("STATE_UPDATE");
        Ok(view)
    }

    pub fn lab_comparison(
        &self,
        action: Option<(String, u64, PlayerCommand)>,
    ) -> Result<(u64, Value)> {
        let current = self.snapshot()?;
        let revision = current.position().revision;
        let spectator = self.view(&current, None)?;
        if self.has_bots() || !spectator["view"]["clock"].is_null() {
            return Err(ApiError::invalid(
                "Cross-engine comparisons require untimed manual games",
            ));
        }
        let next = if let Some((actor, expected, command)) = action {
            if expected != revision {
                return Err(ApiError::conflict("Stale game revision"));
            }
            let (next, _) = current.prepare_submission_at(
                &actor,
                revision,
                command,
                &ExecutionBudget::default(),
                self.clock_now(),
            )?;
            Some(next)
        } else {
            None
        };
        let engine = next.as_ref().unwrap_or(&current);
        let mut views = serde_json::Map::new();
        views.insert(
            "spectator".into(),
            if next.is_some() {
                self.view(engine, None)?
            } else {
                spectator
            },
        );
        for seat in &self.credentials.player_tokens {
            views.insert(
                seat.hero_id.clone(),
                self.view(engine, Some(&seat.hero_id))?,
            );
        }
        Ok((revision, Value::Object(views)))
    }

    pub fn lab_snapshot(&self) -> Result<Value> {
        let engine = self.snapshot()?;
        Ok(json!({
            "hash": self.lab_revision(engine.position().revision),
            "checkpoint": portable_checkpoint_at(&engine, &self.credentials.hero_names, self.clock_now())?,
        }))
    }

    pub fn lab_revision(&self, revision: u64) -> String {
        format!("{}:{revision}", self.id)
    }

    pub fn replay(&self, store: &Store) -> Result<Archive> {
        self.check_active()?;
        store.replay(
            &self.id,
            self.credentials.hero_names.clone(),
            self.credentials.bots.keys().cloned().collect(),
        )
    }

    pub fn report_position(&self, store: &Store, viewer: Option<&str>) -> Result<Value> {
        let (position, archive) = {
            let current = self.current.lock().map_err(ApiError::internal)?;
            (current.position(), self.replay(store)?)
        };
        let decision_index = position
            .revision
            .checked_sub(archive.start_revision)
            .filter(|_| position.revision == archive.revision)
            .ok_or_else(|| ApiError::internal("Reported revision differs from the journal"))?;
        store.pin_report_replay(&self.id, &archive)?;
        Ok(json!({
            "reporter_hero": viewer, "decision_index": decision_index,
            "round": position.round, "turn": position.turn,
        }))
    }

    pub fn require_finished(&self) -> Result<()> {
        if self.snapshot()?.position().phase != goa2_core::codec::Phase::GameOver {
            return Err(crate::shares::unfinished());
        }
        Ok(())
    }

    pub fn share(&self, store: &Store) -> Result<String> {
        if let Some(token) = store.share_for_game(&self.id)? {
            return Ok(token);
        }
        self.require_finished()?;
        let share = crate::shares::Share::bake(&self.id, self.replay(store)?)?;
        store.save_share(&self.id, share)
    }

    pub fn history(&self, store: &Store, viewer: Option<&str>) -> Result<History> {
        let engine = self.snapshot()?;
        let revision = engine.position().revision;
        let (start, entries) = store.history(&self.id, revision)?;
        let scoped = self.view(&engine, viewer)?;
        Ok(History::from_entries(start, revision, entries, &scoped))
    }

    pub fn checkpoint(&self) -> Result<Value> {
        let engine = self.snapshot()?;
        Ok(json!({
            "revision": engine.position().revision,
            "checkpoint": engine.checkpoint()?,
            "hero_names": self.credentials.hero_names,
            "bots": self.credentials.bots,
        }))
    }

    fn drive_deadline(&self, store: &Store) -> Result<()> {
        let mut current = self.current.lock().map_err(ApiError::internal)?;
        self.check_active()?;
        self.apply_due_timeout(store, &mut current, self.clock_now())?;
        Ok(())
    }

    fn apply_due_timeout(
        &self,
        store: &Store,
        current: &mut Arc<LiveGame>,
        now: u64,
    ) -> Result<bool> {
        if current.clock_deadline(now) != Some(0) {
            return Ok(false);
        }
        let entropy = Uuid::new_v4().as_u128() as u64;
        let Some((mut next, transition, actor)) =
            current.prepare_timeout(now, entropy, &ExecutionBudget::default())?
        else {
            return Ok(false);
        };
        let rebase_ms = self.clock_now();
        next.rebase_clock_after_processing(rebase_ms);
        store.commit(
            &self.id,
            current.position().revision,
            next.position().revision,
            &next.checkpoint()?,
            None,
            Decision {
                at_ms: now,
                rebase_ms: Some(rebase_ms),
                operation: Operation::Timeout { entropy },
            },
            HistoryContext::new(&current.position(), Some(&actor)),
        )?;
        let next = Arc::new(next);
        *current = Arc::clone(&next);
        let _ = self.updates.send(GameEvent::State(Update {
            revision: next,
            transition: Arc::new(transition),
            actor,
            action_id: None,
        }));
        Ok(true)
    }

    pub fn submit(
        &self,
        store: &Store,
        hero: &str,
        expected: Option<u64>,
        action_id: Option<String>,
        command: PlayerCommand,
    ) -> Result<Submission> {
        // The guard covers candidate construction, durable commit and publication.
        let mut current = self.current.lock().map_err(ApiError::internal)?;
        self.check_active()?;
        let command_body = serde_json::to_value(&command).map_err(ApiError::internal)?;
        if let Some(id) = action_id.as_deref()
            && let Some(reply) = store.accepted(&self.id, hero, id, &command_body)?
        {
            return Ok(Submission {
                reply,
                replayed: true,
            });
        }
        if let PlayerCommand::CheatGold { hero_id, amount } = &command {
            if !current.cheats_enabled() {
                return Err(ApiError::forbidden("Cheats are not enabled for this game"));
            }
            if !self.has_seat(hero_id) {
                return Err(ApiError::not_found("Hero not found"));
            }
            if *amount <= 0 {
                return Err(ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "Amount must be a positive integer",
                ));
            }
        }
        let now = self.clock_now();
        if self.apply_due_timeout(store, &mut current, now)? {
            return Err(ApiError::conflict(
                "Clock advanced before the action. Refresh the current decision.",
            ));
        }
        let previous = current.position().revision;
        if expected.is_some_and(|expected| expected != previous) {
            return Err(ApiError::conflict("Stale game revision"));
        }
        let ready = matches!(command, PlayerCommand::Ready { .. });
        let operation = Operation::Command {
            actor: hero.into(),
            command: command.clone(),
        };
        let (mut next, transition) = replay::prepare_command(
            &current,
            hero,
            command,
            now,
            self.credentials.bots.keys().map(String::as_str),
        )?;
        let rebase_ms = next.uses_clock().then(|| self.clock_now());
        if let Some(rebase_ms) = rebase_ms {
            next.rebase_clock_after_processing(rebase_ms);
        }
        let mut reply = if ready {
            self.view(&next, Some(hero))?
        } else {
            next.client_result(&transition, Some(hero))?
        };
        let revision = next.position().revision;
        reply["revision"] = json!(revision);
        let accepted = action_id.as_deref().map(|id| AcceptedAction {
            hero_id: hero,
            action_id: id,
            command: &command_body,
            reply: &reply,
        });
        store.commit(
            &self.id,
            previous,
            revision,
            &next.checkpoint()?,
            accepted,
            Decision {
                at_ms: now,
                rebase_ms,
                operation,
            },
            HistoryContext::new(&current.position(), None),
        )?;

        let next = Arc::new(next);
        *current = Arc::clone(&next);
        let _ = self.updates.send(GameEvent::State(Update {
            revision: next,
            transition: Arc::new(transition),
            actor: hero.to_owned(),
            action_id,
        }));
        Ok(Submission {
            reply,
            replayed: false,
        })
    }
}

impl Update {
    pub fn message(&self, game: &Game, viewer: Option<&str>) -> Result<Value> {
        let mut message = game.state_update(&self.revision, viewer)?;
        let result = self.revision.client_result(&self.transition, viewer)?;
        if result["events"]
            .as_array()
            .is_some_and(|events| !events.is_empty())
        {
            message["events"] = result["events"].clone();
        }
        if viewer == Some(&self.actor)
            && let Some(id) = &self.action_id
        {
            message["client_action_id"] = json!(id);
        }
        Ok(message)
    }
}

/// Private Lab payload shared by live capture and the offline library port.
pub fn portable_checkpoint(
    engine: &LiveGame,
    hero_names: &BTreeMap<String, String>,
) -> Result<Value> {
    portable_checkpoint_at(engine, hero_names, engine.clock_timestamp())
}

fn portable_checkpoint_at(
    engine: &LiveGame,
    hero_names: &BTreeMap<String, String>,
    now_ms: u64,
) -> Result<Value> {
    let position = engine.position();
    let view = engine.client_view(None)?;
    let teams = view["view"]["teams"]
        .as_object()
        .ok_or_else(|| ApiError::internal("Missing client teams"))?;
    let roster = teams
        .iter()
        .map(|(color, team)| {
            let heroes = team["heroes"]
                .as_array()
                .ok_or_else(|| ApiError::internal("Missing client heroes"))?
                .iter()
                .map(|hero| json!({"id": hero["id"], "name": hero["name"]}))
                .collect::<Vec<_>>();
            Ok((color.clone(), json!({"color": color, "heroes": heroes})))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let prompt = engine
        .decision_players()?
        .into_iter()
        .find_map(|id| engine.request_for(id));
    let summary = json!({
        "phase": position.phase,
        "round": position.round,
        "decision": prompt.as_ref().map(|value| &value["request_type"]),
        "responder": prompt.as_ref().map(|value| &value["player_id"]),
    });
    let mut checkpoint = json!({
        "engine": "rust",
        "native_checkpoint_json": serde_json::to_string(&engine.checkpoint_at(now_ms)?)
            .map_err(ApiError::internal)?,
        "hero_names": hero_names,
        "state": {"teams": roster, "phase": position.phase, "round": position.round},
        "summary": summary,
    });
    if engine.uses_clock() {
        checkpoint["timed"] = Value::Bool(true);
    }
    Ok(checkpoint)
}

#[cfg(test)]
mod blocking_tests {
    use super::*;
    #[tokio::test(flavor = "current_thread")]
    async fn contended_snapshot_does_not_block_the_async_executor() {
        use std::sync::mpsc;
        let content =
            GameContent::read(include_bytes!("../../../content/smoke-game.json").as_slice())
                .unwrap();
        let setup = GameSetup {
            map: "forgotten_island".into(),
            red_heroes: vec!["Arien".into()],
            blue_heroes: vec!["Brogan".into()],
            seed: 29,
            mode: goa2_core::api::GameMode::Quick,
            cheats_enabled: false,
            tie_breaker: None,
        };
        let engine = LiveGame::create(&content, &setup, &ExecutionBudget::default())
            .unwrap()
            .0;
        let credentials = Credentials {
            player_tokens: Vec::new(),
            spectator_token: "synthetic".into(),
            hero_names: BTreeMap::new(),
            bots: BTreeMap::new(),
            fixed_clock_ms: None,
            provisioning_hash: None,
        };
        let game = Arc::new(Game::new("synthetic".into(), credentials, engine, None));
        let (ready_send, ready) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let held = Arc::clone(&game);
        let thread = std::thread::spawn(move || {
            let _guard = held.current.lock().unwrap();
            ready_send.send(()).unwrap();
            let _ = wait.recv_timeout(Duration::from_secs(2));
        });
        ready.recv().unwrap();
        let started = std::time::Instant::now();
        let snapshot = tokio::spawn(async move { game.snapshot_async().await });
        tokio::time::sleep(Duration::from_millis(25)).await;
        release.send(()).unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "mutex wait stalled current-thread runtime"
        );
        assert_eq!(snapshot.await.unwrap().unwrap().position().revision, 0);
        thread.join().unwrap();
    }
}
