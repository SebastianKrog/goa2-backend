//! Persisted clock arithmetic. Callers supply time explicitly.
use super::{
    DecisionMove, ExecutionBudget, GameTransition, LiveGame, PlayerCommand, Result,
    bot::command_for, invalid,
};
use crate::codec::{ObjectPathPart, Phase, StateEnvelope};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn default_true() -> bool {
    true
}
fn default_automatic_limit() -> u64 {
    2
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TimeControlConfig {
    pub planning_allowance_seconds: u64,
    pub resolution_allowance_seconds: u64,
    #[serde(default)]
    pub initiative_bonus_seconds: u64,
    pub response_grant_seconds: u64,
    pub initial_time_bank_seconds: u64,
    pub time_bank_increment_seconds: u64,
    pub max_time_bank_seconds: u64,
    pub upgrade_allowance_seconds: u64,
    #[serde(default = "default_automatic_limit")]
    pub automatic_turn_limit: u64,
    #[serde(default = "default_true")]
    pub enforce_timeouts: bool,
}

impl TimeControlConfig {
    fn validate(&self) -> Result<()> {
        if [
            self.planning_allowance_seconds,
            self.resolution_allowance_seconds,
            self.initiative_bonus_seconds,
            self.response_grant_seconds,
            self.initial_time_bank_seconds,
            self.time_bank_increment_seconds,
            self.max_time_bank_seconds,
            self.upgrade_allowance_seconds,
        ]
        .into_iter()
        .any(|seconds| seconds > 86_400)
            || self.max_time_bank_seconds < self.initial_time_bank_seconds
            || self.automatic_turn_limit > 100
        {
            return Err(invalid("Invalid time-control configuration"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Status {
    WaitingForPlayers,
    SuspendedForInactivity,
    Paused,
    Running,
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Kind {
    Planning,
    Resolution,
    Response,
    LevelUp,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct PlayerClock {
    planning_allowance_ms: u64,
    resolution_allowance_ms: u64,
    initiative_bonus_ms: u64,
    response_time_ms: u64,
    upgrade_allowance_ms: u64,
    time_bank_ms: u64,
    overrun_ms: u64,
    planning_complete: bool,
    planning_locked_by_timeout: bool,
}

impl PlayerClock {
    fn usable(&self, kind: Kind) -> u64 {
        let allowance = match kind {
            Kind::Planning => self.planning_allowance_ms,
            Kind::Resolution => self
                .initiative_bonus_ms
                .saturating_add(self.resolution_allowance_ms),
            Kind::Response => self
                .response_time_ms
                .saturating_add(self.resolution_allowance_ms),
            Kind::LevelUp => self.upgrade_allowance_ms,
        };
        allowance.saturating_add(self.time_bank_ms)
    }

    fn spend(&mut self, kind: Kind, mut elapsed: u64, advisory: bool) {
        let fields: &mut [&mut u64] = match kind {
            Kind::Planning => &mut [&mut self.planning_allowance_ms, &mut self.time_bank_ms],
            Kind::Resolution => &mut [
                &mut self.initiative_bonus_ms,
                &mut self.resolution_allowance_ms,
                &mut self.time_bank_ms,
            ],
            Kind::Response => &mut [
                &mut self.response_time_ms,
                &mut self.resolution_allowance_ms,
                &mut self.time_bank_ms,
            ],
            Kind::LevelUp => &mut [&mut self.upgrade_allowance_ms, &mut self.time_bank_ms],
        };
        for field in fields {
            let spent = (**field).min(elapsed);
            **field -= spent;
            elapsed -= spent;
        }
        if advisory {
            self.overrun_ms = self.overrun_ms.saturating_add(elapsed);
        }
    }
}

#[derive(Clone, Copy)]
pub enum PauseVoteChange<'a> {
    Open(&'a str),
    Cancel,
    Approve(&'a str),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Clock {
    status: Status,
    #[serde(default = "default_true")]
    enforce_timeouts: bool,
    #[serde(default)]
    ready_hero_ids: Vec<String>,
    turn_round: i64,
    turn_number: i64,
    initiative_bonus_hero_id: Option<String>,
    level_up_round: Option<i64>,
    players: BTreeMap<String, PlayerClock>,
    active_kind: Option<Kind>,
    #[serde(default)]
    active_hero_ids: Vec<String>,
    #[serde(default)]
    active_decision_hero_ids: Vec<String>,
    active_request_id: Option<String>,
    #[serde(default)]
    credited_response_request_ids: Vec<String>,
    // Only the clock hold is durable. An interrupted vote can be safely
    // released after restart without spending the negotiation time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pause_vote_id: Option<String>,
    pause_requested_by: Option<String>,
    paused_at_ms: Option<u64>,
    #[serde(default)]
    human_action_seen_this_turn: bool,
    #[serde(default)]
    consecutive_automatic_turns: u64,
    last_settled_at_ms: Option<u64>,
    #[serde(default)]
    revision: u64,
}

#[derive(Clone, Debug)]
pub(super) struct TimeControl {
    config: TimeControlConfig,
    clock: Clock,
}

impl Clock {
    fn settle(&mut self, now: u64) {
        let elapsed = self
            .last_settled_at_ms
            .map_or(0, |before| now.saturating_sub(before));
        if elapsed > 0
            && self.status == Status::Running
            && let Some(kind) = self.active_kind
        {
            for hero in &self.active_hero_ids {
                if let Some(player) = self.players.get_mut(hero) {
                    player.spend(kind, elapsed, !self.enforce_timeouts);
                }
            }
        }
        self.last_settled_at_ms = Some(now);
    }

    fn activate(
        &mut self,
        kind: Option<Kind>,
        running: Vec<String>,
        eligible: Vec<String>,
        request: Option<String>,
    ) {
        if (kind, &running, &eligible, &request)
            != (
                self.active_kind,
                &self.active_hero_ids,
                &self.active_decision_hero_ids,
                &self.active_request_id,
            )
        {
            self.revision = self.revision.saturating_add(1);
        }
        self.active_kind = kind;
        self.active_hero_ids = running;
        self.active_decision_hero_ids = eligible;
        self.active_request_id = request;
    }

    fn clear_active(&mut self) {
        self.active_kind = None;
        self.active_hero_ids.clear();
        self.active_decision_hero_ids.clear();
        self.active_request_id = None;
    }

    fn deadline(&self, now: u64) -> Option<u64> {
        let kind = self.active_kind?;
        if !self.enforce_timeouts || self.status != Status::Running {
            return None;
        }
        let elapsed = self
            .last_settled_at_ms
            .map_or(0, |before| now.saturating_sub(before));
        self.active_hero_ids
            .iter()
            .filter_map(|hero| self.players.get(hero))
            .map(|player| player.usable(kind).saturating_sub(elapsed))
            .min()
    }

    fn public_view(&self, now: u64) -> Value {
        let mut projected = self.clone();
        projected.settle(now);
        let active = projected.active_kind.map(|kind| {
            json!({
                "kind": kind, "request_id": projected.active_request_id,
                "hero_ids": projected.active_decision_hero_ids,
                "running_hero_ids": projected.active_hero_ids,
            })
        });
        let pause = (projected.status == Status::Paused).then(|| {
            json!({
                "requested_by": projected.pause_requested_by, "since_ms": projected.paused_at_ms,
            })
        });
        let players: BTreeMap<_, _> = projected
            .players
            .iter()
            .map(|(hero, clock)| {
                let mut value = json!(clock);
                value["running"] = json!(
                    projected.status == Status::Running
                        && projected.active_kind.is_some()
                        && projected.active_hero_ids.contains(hero)
                );
                (hero, value)
            })
            .collect();
        json!({
            "status": projected.status, "enforce_timeouts": projected.enforce_timeouts,
            "server_now_ms": now, "pause": pause,
            "turn_key": {"round": projected.turn_round, "turn": projected.turn_number},
            "ready_hero_ids": projected.ready_hero_ids, "active": active, "players": players,
        })
    }
}

impl TimeControl {
    pub(super) fn take_from(envelope: &mut StateEnvelope) -> Result<Option<Self>> {
        if envelope.state["time_control"].is_null() && envelope.state["clock"].is_null() {
            return Ok(None);
        }
        let config: TimeControlConfig =
            serde_json::from_value(envelope.state["time_control"].take())
                .map_err(|error| invalid(&format!("Invalid time controls: {error}")))?;
        config.validate()?;
        let clock: Clock = serde_json::from_value(envelope.state["clock"].take())
            .map_err(|error| invalid(&format!("Invalid game clock: {error}")))?;
        let clock_path = |path: &[ObjectPathPart]| matches!(path.first(), Some(ObjectPathPart::Key(key)) if key == "clock" || key == "time_control");
        for binding in &envelope.object_bindings {
            if binding.paths.iter().any(|path| clock_path(path))
                && !binding.paths.iter().all(|path| clock_path(path))
            {
                return Err(invalid("Clock cannot share objects with rules state"));
            }
        }
        envelope
            .object_bindings
            .retain(|binding| !binding.paths.iter().any(|path| clock_path(path)));
        Ok(Some(Self { config, clock }))
    }

    pub(super) fn put_into(&self, envelope: &mut StateEnvelope, now: u64) {
        envelope.state["time_control"] = json!(self.config);
        if self.clock.last_settled_at_ms == Some(now) || now == 0 {
            envelope.state["clock"] = json!(self.clock);
        } else {
            let mut clock = self.clock.clone();
            clock.settle(now);
            envelope.state["clock"] = json!(clock);
        }
    }

    fn begin_turn(&mut self, round: i64, turn: i64) {
        let clock = &mut self.clock;
        if (round, turn) == (clock.turn_round, clock.turn_number) {
            return;
        }
        clock.turn_round = round;
        clock.turn_number = turn;
        clock.level_up_round = None;
        clock.initiative_bonus_hero_id = None;
        clock.credited_response_request_ids.clear();
        clock.human_action_seen_this_turn = false;
        for player in clock.players.values_mut() {
            let credit = self.config.time_bank_increment_seconds * 1000;
            let paid = player.overrun_ms.min(credit);
            player.overrun_ms -= paid;
            player.time_bank_ms = (self.config.max_time_bank_seconds * 1000)
                .min(player.time_bank_ms.saturating_add(credit - paid));
            player.planning_allowance_ms = self.config.planning_allowance_seconds * 1000;
            player.resolution_allowance_ms = self.config.resolution_allowance_seconds * 1000;
            player.initiative_bonus_ms = 0;
            player.response_time_ms = 0;
            player.upgrade_allowance_ms = 0;
            player.planning_complete = false;
            player.planning_locked_by_timeout = false;
        }
        clock.clear_active();
        clock.revision = clock.revision.saturating_add(1);
    }
}

impl LiveGame {
    pub fn install_time_control(&mut self, config: TimeControlConfig, now: u64) -> Result<()> {
        config.validate()?;
        if self.clock.is_some() || self.revision != 0 || self.position().phase != Phase::Planning {
            return Err(invalid(
                "Time controls can only be configured before the game",
            ));
        }
        let position = self.position();
        let players = self
            .players()
            .map(|(hero, _)| {
                (
                    hero.to_owned(),
                    PlayerClock {
                        planning_allowance_ms: config.planning_allowance_seconds * 1000,
                        resolution_allowance_ms: config.resolution_allowance_seconds * 1000,
                        time_bank_ms: config.initial_time_bank_seconds * 1000,
                        ..PlayerClock::default()
                    },
                )
            })
            .collect();
        let clock = Clock {
            status: Status::WaitingForPlayers,
            enforce_timeouts: config.enforce_timeouts,
            turn_round: position.round,
            turn_number: position.turn,
            players,
            ready_hero_ids: Vec::new(),
            active_hero_ids: Vec::new(),
            active_decision_hero_ids: Vec::new(),
            credited_response_request_ids: Vec::new(),
            active_kind: None,
            active_request_id: None,
            initiative_bonus_hero_id: None,
            level_up_round: None,
            pause_vote_id: None,
            pause_requested_by: None,
            paused_at_ms: None,
            last_settled_at_ms: Some(now),
            human_action_seen_this_turn: false,
            consecutive_automatic_turns: 0,
            revision: 0,
        };
        self.clock = Some(TimeControl { config, clock });
        Ok(())
    }

    pub(super) fn validate_clock_roster(&self) -> Result<()> {
        if let Some(timed) = &self.clock {
            let clock = &timed.clock;
            if clock.players.len() != self.players().count()
                || self
                    .players()
                    .any(|(id, _)| !clock.players.contains_key(id))
                || clock.enforce_timeouts != timed.config.enforce_timeouts
                || [
                    &clock.ready_hero_ids,
                    &clock.active_hero_ids,
                    &clock.active_decision_hero_ids,
                ]
                .iter()
                .any(|ids| ids.iter().any(|id| !clock.players.contains_key(id)))
            {
                return Err(invalid("Clock roster or mode does not match the game"));
            }
            if clock.pause_vote_id.as_ref().is_some_and(|id| {
                id.is_empty()
                    || clock.status != Status::Running
                    || clock.active_kind.is_some()
                    || !clock.active_hero_ids.is_empty()
                    || !clock.active_decision_hero_ids.is_empty()
                    || clock.active_request_id.is_some()
            }) {
                return Err(invalid("Invalid pause vote clock hold"));
            }
        }
        Ok(())
    }

    pub fn uses_clock(&self) -> bool {
        self.clock.is_some()
    }

    pub fn clock_running(&self) -> bool {
        self.clock
            .as_ref()
            .is_some_and(|timed| timed.clock.status == Status::Running)
    }

    pub fn clock_paused(&self) -> bool {
        self.clock
            .as_ref()
            .is_some_and(|timed| timed.clock.status == Status::Paused)
    }

    pub fn enforces_deadlines(&self) -> bool {
        self.clock
            .as_ref()
            .is_some_and(|timed| timed.clock.enforce_timeouts)
    }

    pub fn clock_timestamp(&self) -> u64 {
        self.clock
            .as_ref()
            .and_then(|timed| timed.clock.last_settled_at_ms)
            .unwrap_or(0)
    }

    pub(super) fn time_control_view(&self) -> Value {
        self.clock
            .as_ref()
            .map_or(Value::Null, |timed| json!(timed.config))
    }

    pub(super) fn clock_view(&self, now: u64) -> Value {
        self.clock
            .as_ref()
            .map_or(Value::Null, |timed| timed.clock.public_view(now))
    }

    pub fn pause_vote_hold(&self) -> Option<&str> {
        self.clock.as_ref()?.clock.pause_vote_id.as_deref()
    }

    /// Change only the clock and revision. The server owns the voting roster
    /// and persists this candidate before publishing the outcome.
    pub fn prepare_pause_vote(
        &self,
        change: PauseVoteChange<'_>,
        now: u64,
    ) -> Result<(Self, GameTransition)> {
        let timed = self
            .clock
            .as_ref()
            .ok_or_else(|| invalid("This match does not use time control"))?;
        if timed.clock.status != Status::Running {
            return Err(invalid("Only a running timed match can be paused"));
        }
        match change {
            PauseVoteChange::Open("") => return Err(invalid("Pause vote ID is required")),
            PauseVoteChange::Open(_) if timed.clock.pause_vote_id.is_some() => {
                return Err(invalid("A pause vote is already open"));
            }
            PauseVoteChange::Cancel if timed.clock.pause_vote_id.is_none() => {
                return Err(invalid("No pause vote holds the clock"));
            }
            PauseVoteChange::Approve(hero) => {
                self.hero(hero)?;
            }
            _ => {}
        }
        let mut next = Self {
            session: self.session.clone(),
            revision: self
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("game revision exhausted"))?,
            upgrade_history: self.upgrade_history.clone(),
            rollback: self.rollback.clone(),
            lane_progress: self.lane_progress.clone(),
            clock: self.clock.clone(),
        };
        let clock = &mut next.clock.as_mut().unwrap().clock;
        clock.settle(now);
        clock.pause_vote_id = if let PauseVoteChange::Open(id) = change {
            Some(id.to_owned())
        } else {
            None
        };
        clock.clear_active();
        clock.revision = clock.revision.saturating_add(1);
        if let PauseVoteChange::Approve(proposer) = change {
            clock.status = Status::Paused;
            clock.pause_requested_by = Some(proposer.to_owned());
            clock.paused_at_ms = Some(now);
            clock.ready_hero_ids.clear();
        } else if matches!(change, PauseVoteChange::Cancel) {
            next.reconcile_clock(now)?;
        }
        let transition = GameTransition {
            position: next.position(),
            events: Vec::new(),
            previous_phase: self.position().phase,
        };
        Ok((next, transition))
    }

    pub(super) fn prepare_clock_action(
        &mut self,
        now: u64,
        human: bool,
        gold_cheat: bool,
    ) -> Result<()> {
        let Some(timed) = &mut self.clock else {
            return Ok(());
        };
        let clock = &mut timed.clock;
        if clock.status != Status::Running {
            // Test setup can grant gold before anyone readies or while paused.
            if gold_cheat {
                return Ok(());
            }
            return Err(invalid("Timed match is not running"));
        }
        if clock.pause_vote_id.is_some() {
            return Err(invalid("A pause vote is open"));
        }
        clock.settle(now);
        if human && !clock.human_action_seen_this_turn {
            clock.human_action_seen_this_turn = true;
            clock.revision = clock.revision.saturating_add(1);
        }
        Ok(())
    }

    /// If waiting, return the seats that still need to ready, in roster order.
    pub fn waiting_ready_players(&self) -> Option<Vec<&str>> {
        let clock = &self.clock.as_ref()?.clock;
        matches!(
            clock.status,
            Status::WaitingForPlayers | Status::SuspendedForInactivity | Status::Paused
        )
        .then(|| {
            self.players()
                .map(|(hero, _)| hero)
                .filter(|hero| !clock.ready_hero_ids.iter().any(|ready| ready == hero))
                .collect()
        })
    }

    /// Prepare server-controlled seats before publishing a new or resumed game.
    pub fn ready_bot_seats<'a>(
        &mut self,
        bots: impl IntoIterator<Item = &'a str>,
        now: u64,
    ) -> Result<()> {
        let Some(waiting) = self.waiting_ready_players() else {
            return Ok(());
        };
        let ready: Vec<_> = bots
            .into_iter()
            .filter(|hero| waiting.contains(hero))
            .collect();
        for hero in ready {
            self.set_ready(hero, true, now)?;
        }
        self.reconcile_clock(now)
    }

    /// Wall-clock time a bot can still use for this decision, capped by the caller.
    pub fn bot_search_window_ms(&self, hero: &str, now: u64, cap_ms: u64) -> u64 {
        let Some(timed) = &self.clock else {
            return cap_ms;
        };
        let clock = &timed.clock;
        if clock.status != Status::Running || !clock.active_hero_ids.iter().any(|id| id == hero) {
            return 0;
        }
        if !clock.enforce_timeouts {
            return cap_ms;
        }
        let Some(kind) = clock.active_kind else {
            return 0;
        };
        let elapsed = clock
            .last_settled_at_ms
            .map_or(0, |before| now.saturating_sub(before));
        clock.players[hero]
            .usable(kind)
            .saturating_sub(elapsed)
            .min(cap_ms)
    }

    pub(super) fn set_ready(&mut self, hero: &str, ready: bool, now: u64) -> Result<()> {
        let roster = self
            .players()
            .map(|(id, _)| id.to_owned())
            .collect::<Vec<_>>();
        let position = self.position();
        let Some(timed) = &mut self.clock else {
            return Err(invalid("This match does not use time controls"));
        };
        let clock = &mut timed.clock;
        if !matches!(
            clock.status,
            Status::WaitingForPlayers | Status::SuspendedForInactivity | Status::Paused
        ) {
            return Err(invalid("The timed match is not waiting for players"));
        }
        let was_suspended = clock.status == Status::SuspendedForInactivity;
        clock.ready_hero_ids = roster
            .into_iter()
            .filter(|id| {
                if id == hero {
                    ready
                } else {
                    clock.ready_hero_ids.contains(id)
                }
            })
            .collect();
        clock.revision = clock.revision.saturating_add(1);
        if clock.ready_hero_ids.len() == clock.players.len() {
            clock.status = Status::Running;
            clock.pause_requested_by = None;
            clock.paused_at_ms = None;
            clock.last_settled_at_ms = Some(now);
            clock.revision = clock.revision.saturating_add(1);
            if was_suspended {
                clock.consecutive_automatic_turns = 0;
                timed.begin_turn(position.round, position.turn);
            }
        }
        Ok(())
    }

    pub(super) fn reconcile_clock(&mut self, now: u64) -> Result<()> {
        if self.clock.is_none() {
            return Ok(());
        }
        let position = self.position();
        let decision = if position.phase == Phase::LevelUp {
            self.session
                .round
                .pending
                .iter()
                .map(|(id, _)| self.session.symbols.name(*id).to_owned())
                .collect()
        } else {
            self.decision_players()?
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let prompt = self.session.last_request.as_ref();
        let request_id = prompt.map(|request| self.session.request_token(request.id));
        let team_request =
            prompt.is_some_and(|r| self.session.symbols.name(r.owner).starts_with("team:"));
        let owner = self
            .session
            .planning
            .owner
            .map(|hero| self.session.symbols.name(hero));
        let timed = self.clock.as_mut().unwrap();
        if timed.clock.status != Status::Running || timed.clock.pause_vote_id.is_some() {
            return Ok(());
        }
        // The previous allocation was settled when the command entered the lock.
        timed.clock.last_settled_at_ms = Some(now);
        if position.phase == Phase::GameOver {
            timed.clock.status = Status::Finished;
            timed.clock.clear_active();
            timed.clock.revision = timed.clock.revision.saturating_add(1);
            return Ok(());
        }
        let clock = &mut timed.clock;
        if (clock.turn_round, clock.turn_number) != (position.round, position.turn) {
            clock.consecutive_automatic_turns = if clock.human_action_seen_this_turn {
                0
            } else {
                clock.consecutive_automatic_turns.saturating_add(1)
            };
            if clock.enforce_timeouts
                && timed.config.automatic_turn_limit > 0
                && clock.consecutive_automatic_turns >= timed.config.automatic_turn_limit
            {
                clock.status = Status::SuspendedForInactivity;
                clock.ready_hero_ids.clear();
                clock.clear_active();
                clock.revision = clock.revision.saturating_add(1);
                return Ok(());
            }
        }
        timed.begin_turn(position.round, position.turn);
        let clock = &mut timed.clock;
        let mut running = decision.clone();
        let mut active = decision;
        let kind = match position.phase {
            Phase::Planning => {
                for (id, player) in &mut clock.players {
                    player.planning_complete = !active.contains(id);
                }
                running.retain(|id| !clock.players[id].planning_locked_by_timeout);
                active = running.clone();
                Some(Kind::Planning)
            }
            Phase::LevelUp => {
                if clock.level_up_round != Some(position.round) {
                    clock.level_up_round = Some(position.round);
                    for player in clock.players.values_mut() {
                        player.upgrade_allowance_ms = timed.config.upgrade_allowance_seconds * 1000;
                    }
                    clock.revision = clock.revision.saturating_add(1);
                }
                Some(Kind::LevelUp)
            }
            Phase::Resolution if !active.is_empty() => {
                if team_request || owner.is_some_and(|hero| hero != active[0]) {
                    if let Some(id) = &request_id
                        && !clock.credited_response_request_ids.contains(id)
                    {
                        clock.credited_response_request_ids.push(id.clone());
                        for hero in &active {
                            if let Some(player) = clock.players.get_mut(hero) {
                                player.response_time_ms = player
                                    .response_time_ms
                                    .saturating_add(timed.config.response_grant_seconds * 1000);
                            }
                        }
                        clock.revision = clock.revision.saturating_add(1);
                    }
                    if team_request {
                        running.retain(|id| clock.players[id].usable(Kind::Response) > 0);
                        if running.is_empty() {
                            running = active.clone();
                        }
                    }
                    Some(Kind::Response)
                } else {
                    if let Some(hero) = owner
                        && clock.initiative_bonus_hero_id.is_none()
                    {
                        clock.initiative_bonus_hero_id = Some(hero.to_owned());
                        if let Some(player) = clock.players.get_mut(hero) {
                            player.initiative_bonus_ms =
                                timed.config.initiative_bonus_seconds * 1000;
                        }
                        clock.revision = clock.revision.saturating_add(1);
                    }
                    Some(Kind::Resolution)
                }
            }
            _ => None,
        };
        clock.activate(kind, running, active, request_id);
        Ok(())
    }

    pub(super) fn preserve_response_credit(&mut self, from: &str, to: String) {
        if let Some(timed) = &mut self.clock
            && timed
                .clock
                .credited_response_request_ids
                .iter()
                .any(|id| id == from)
        {
            timed.clock.credited_response_request_ids.push(to);
        }
    }

    pub fn clock_deadline(&self, now: u64) -> Option<u64> {
        self.clock.as_ref()?.clock.deadline(now)
    }

    pub(super) fn planning_timed_out(&self, hero: &str) -> bool {
        self.clock
            .as_ref()
            .and_then(|timed| timed.clock.players.get(hero))
            .is_some_and(|player| player.planning_locked_by_timeout)
    }

    /// Apply at most one due decision so the host can persist and publish it before continuing.
    pub fn prepare_timeout(
        &self,
        now: u64,
        entropy: u64,
        budget: &ExecutionBudget<'_>,
    ) -> Result<Option<(Self, GameTransition, String)>> {
        if self.clock_deadline(now) != Some(0) {
            return Ok(None);
        }
        let mut next = Self {
            session: self.session.clone(),
            revision: self
                .revision
                .checked_add(1)
                .ok_or_else(|| invalid("game revision exhausted"))?,
            upgrade_history: self.upgrade_history.clone(),
            rollback: self.rollback.clone(),
            lane_progress: self.lane_progress.clone(),
            clock: self.clock.clone(),
        };
        let clock = &mut next.clock.as_mut().unwrap().clock;
        clock.settle(now);
        let kind = clock
            .active_kind
            .ok_or_else(|| invalid("No active clock"))?;
        let hero_id = clock
            .active_hero_ids
            .iter()
            .find(|id| clock.players[*id].usable(kind) == 0)
            .cloned()
            .ok_or_else(|| invalid("No exhausted clock"))?;
        let team_request = kind == Kind::Response
            && next.session.last_request.as_ref().is_some_and(|request| {
                next.session
                    .symbols
                    .name(request.owner)
                    .starts_with("team:")
            });
        if team_request
            && clock
                .active_decision_hero_ids
                .iter()
                .any(|id| clock.players[id].usable(kind) > 0)
        {
            next.reconcile_clock(now)?;
            let transition = GameTransition {
                position: next.position(),
                events: Vec::new(),
                previous_phase: self.position().phase,
            };
            return Ok(Some((next, transition, hero_id)));
        }
        if kind == Kind::Planning {
            clock
                .players
                .get_mut(&hero_id)
                .unwrap()
                .planning_locked_by_timeout = true;
        }
        let actor = next.hero(&hero_id)?;
        let command = if kind == Kind::Planning {
            let choices = next.session.planning_choices(actor)?;
            let choice = choices
                .iter()
                .find(|choice| matches!(choice, DecisionMove::FinishPlanning { .. }))
                .or_else(|| choices.get((entropy as usize).checked_rem(choices.len())?))
                .cloned()
                .ok_or_else(|| invalid("No legal planning timeout decision"))?;
            command_for(&next.session, choice)?
        } else {
            let choices = next.session.resolution_choices(actor)?;
            let commands = choices
                .into_iter()
                .map(|choice| command_for(&next.session, choice))
                .collect::<Result<Vec<_>>>()?;
            let preferred = ["HOLD", "SKIP", "PASS"].into_iter().find_map(|preferred| commands.iter()
                .position(|command| matches!(command, PlayerCommand::Input { selection, .. } if selection.as_str() == Some(preferred))));
            let index = preferred
                .or_else(|| (entropy as usize).checked_rem(commands.len()))
                .ok_or_else(|| invalid("No legal input timeout decision"))?;
            let command = commands.into_iter().nth(index).unwrap();
            if kind != Kind::LevelUp {
                next.session.freeze_rollback()?;
                next.rollback = None;
            }
            command
        };
        let action = match &command {
            PlayerCommand::Commit { .. } => "commit",
            PlayerCommand::FinishPlanning => "finish_planning",
            PlayerCommand::Pass => "pass",
            _ => "input",
        };
        let request_id = match &command {
            PlayerCommand::Input { request_id, .. } => Some(request_id.clone()),
            _ => None,
        };
        next.apply(actor, command)?;
        if kind == Kind::Planning
            && next.session.planning.phase == Phase::Planning
            && next
                .session
                .planning_choices(actor)?
                .iter()
                .any(|choice| matches!(choice, DecisionMove::FinishPlanning { .. }))
        {
            next.apply(actor, PlayerCommand::FinishPlanning)?;
        }
        let mut transition = next.advance(budget)?;
        next.update_rollback()?;
        next.reconcile_clock(now)?;
        transition.previous_phase = self.position().phase;
        transition.events.insert(0, json!({
            "event_type": "TIMER_EXPIRED", "actor_id": hero_id, "target_id": null,
            "from_hex": null, "to_hex": null,
            "metadata": {"clock_kind": kind, "automatic_action": action, "request_id": request_id},
        }));
        Ok(Some((next, transition, hero_id)))
    }

    pub(super) fn begin_table_edit_clock(&mut self, now: u64) {
        if let Some(timed) = &mut self.clock {
            timed.clock.settle(now);
            timed.clock.pause_vote_id = None;
        }
    }

    pub(super) fn ready_after_reviving(&mut self) {
        if let Some(timed) = &mut self.clock
            && timed.clock.status == Status::Finished
        {
            timed.clock.status = Status::WaitingForPlayers;
            timed.clock.ready_hero_ids.clear();
            timed.clock.clear_active();
        }
    }

    pub(super) fn resume_rewound_clock(&mut self, now: u64) -> Result<()> {
        if let Some(timed) = &mut self.clock {
            timed.clock.pause_vote_id = None;
            if timed.clock.status == Status::Running {
                timed.clock.last_settled_at_ms = Some(now);
                self.reconcile_clock(now)?;
            }
        }
        Ok(())
    }

    pub fn rebase_clock_after_processing(&mut self, now: u64) {
        if let Some(timed) = &mut self.clock {
            timed.clock.last_settled_at_ms = Some(now);
        }
    }
}
