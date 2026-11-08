//! Table votes for clock holds and private seat-link disclosure.
use super::{Game, GameEvent, Server, Update, wall_clock_ms};
use crate::{
    ApiError, Result, blocking,
    history::HistoryContext,
    replay::{Decision, Operation},
    store::Store,
};
use axum::http::StatusCode;
use goa2_core::interpreter::{
    EngineError, ExecutionBudget,
    live_game::{GameTransition, LiveGame, PatchCommand, PauseVoteChange, UnstickCommand},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::task::AbortHandle;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Voting {
    sockets: BTreeMap<String, usize>,
    pending: Option<Proposal>,
    last_resolution: Option<Value>,
    expiry_task: Option<AbortHandle>,
}

impl Voting {
    pub(super) fn new(interrupted: Option<&str>) -> Self {
        Self {
            last_resolution: interrupted.map(cancelled_notice),
            ..Self::default()
        }
    }
}

pub fn cancelled_notice(id: &str) -> Value {
    json!({"type": "OVERRIDE_RESOLVED", "proposal_id": id,
           "outcome": "cancelled", "tally": {"yes": [], "no": []}})
}

#[derive(Clone)]
enum Action {
    Pause,
    RevealPlayer(String),
    Rewind(u64),
    Unstick(UnstickCommand),
    Patch(PatchCommand),
}

impl Action {
    fn from_request(game: &Game, body: &Value) -> Result<Self> {
        if matches!(body["family"].as_str(), Some("unstick" | "patch")) {
            let mut arguments = match &body["args"] {
                Value::Null => serde_json::Map::new(),
                Value::Object(fields) => fields.clone(),
                _ => return Err(ApiError::invalid("Recovery arguments must be an object")),
            };
            arguments.insert("op".into(), body["op"].clone());
            if body["family"] == "patch" {
                let command = serde_json::from_value(Value::Object(arguments)).map_err(|_| {
                    ApiError::invalid("Unsupported table correction or invalid arguments")
                })?;
                return Ok(Self::Patch(command));
            }
            let command = serde_json::from_value(Value::Object(arguments))
                .map_err(|_| ApiError::invalid("Unknown recovery command or invalid arguments"))?;
            if let UnstickCommand::ForceActor { hero_id } = &command
                && !game.has_seat(hero_id)
            {
                return Err(ApiError::invalid("Choose a hero in this game"));
            }
            return Ok(Self::Unstick(command));
        }
        if !body["op"].is_null() && body["op"] != "" {
            return Err(ApiError::invalid("This proposal does not take an op"));
        }
        match body["family"].as_str() {
            Some("pause") if body["args"].is_null() || body["args"] == json!({}) => Ok(Self::Pause),
            Some("pause") => Err(ApiError::invalid("Pause proposals take no args")),
            Some("reveal_player") => {
                let hero = body["args"]["hero_id"]
                    .as_str()
                    .filter(|hero| game.has_seat(hero) && !game.is_bot(hero))
                    .ok_or_else(|| ApiError::invalid("Choose a human seat in this game"))?;
                Ok(Self::RevealPlayer(hero.to_owned()))
            }
            Some("rewind") => body["to"]
                .as_u64()
                .map(Self::Rewind)
                .ok_or_else(|| ApiError::invalid("Rewind requires a non-negative decision index")),
            _ => Err(ApiError::invalid("Unsupported table vote")),
        }
    }
}

#[derive(Clone)]
struct Proposal {
    action: Action,
    id: String,
    proposer: String,
    eligible: Vec<String>,
    votes: BTreeMap<String, bool>,
    deadline: Instant,
    expires_at: f64,
}

impl Proposal {
    fn threshold(&self) -> usize {
        self.eligible.len() / 2 + 1
    }

    fn outcome(&self) -> Option<&'static str> {
        let yes = self.votes.values().filter(|&&vote| vote).count();
        let no = self.votes.len() - yes;
        if yes >= self.threshold() {
            Some("applied")
        } else if self.eligible.len() - no < self.threshold() {
            Some("rejected")
        } else {
            None
        }
    }

    fn tally(&self) -> Value {
        let voters = |approve| {
            self.votes
                .iter()
                .filter_map(move |(hero, &vote)| (vote == approve).then_some(hero))
                .collect::<Vec<_>>()
        };
        json!({"yes": voters(true), "no": voters(false)})
    }

    fn proposed(&self) -> Value {
        let (family, op, args, to, summary) = match &self.action {
            Action::Pause => ("pause", None, json!({}), None, "Pause the match".to_owned()),
            Action::RevealPlayer(hero) => (
                "reveal_player",
                None,
                json!({"hero_id": hero}),
                None,
                "Share a player link".to_owned(),
            ),
            Action::Rewind(to) => (
                "rewind",
                None,
                json!({}),
                Some(*to),
                format!("Rewind to decision {to}"),
            ),
            Action::Patch(command) => {
                let mut args = json!(command);
                args.as_object_mut()
                    .expect("patch command is an object")
                    .remove("op");
                ("patch", Some(command.name()), args, None, command.summary())
            }
            Action::Unstick(command) => {
                let (args, summary) = match command {
                    UnstickCommand::SkipInput => (json!({}), "Skip the pending input".into()),
                    UnstickCommand::AbortAction => (json!({}), "Abort the current action".into()),
                    UnstickCommand::EndTurn => (json!({}), "End the current hero turn".into()),
                    UnstickCommand::ForceActor { hero_id } => (
                        json!({"hero_id": hero_id}),
                        format!("Set the current actor to {hero_id}"),
                    ),
                };
                ("unstick", Some(command.name()), args, None, summary)
            }
        };
        json!({
            "type": "OVERRIDE_PROPOSED", "proposal_id": self.id,
            "proposer_hero_id": self.proposer, "family": family,
            "op": op, "args": args, "to": to, "summary": summary,
            "eligible_voters": self.eligible, "threshold": self.threshold(),
            "tally": self.tally(), "expires_at": self.expires_at,
        })
    }

    fn resolved(&self, outcome: &str, rejection: Option<&str>) -> Value {
        let mut message = json!({"type": "OVERRIDE_RESOLVED", "proposal_id": self.id,
               "outcome": outcome, "tally": self.tally()});
        if let Some(rejection) = rejection {
            message["reason"] = json!({"code": "invalid_op", "message": rejection});
        }
        message
    }
}

pub struct VoterSocket {
    game: Arc<Game>,
    hero: String,
}

impl Drop for VoterSocket {
    fn drop(&mut self) {
        if let Ok(mut voting) = self.game.voting.lock()
            && let Some(count) = voting.sockets.get_mut(&self.hero)
        {
            *count -= 1;
            if *count == 0 {
                voting.sockets.remove(&self.hero);
            }
        }
    }
}

impl Game {
    pub fn connect_voter(self: &Arc<Self>, viewer: Option<&str>) -> Result<Option<VoterSocket>> {
        let Some(hero) = viewer.filter(|hero| !self.is_bot(hero)) else {
            return Ok(None);
        };
        let mut voting = self.voting.lock().map_err(ApiError::internal)?;
        *voting.sockets.entry(hero.to_owned()).or_default() += 1;
        Ok(Some(VoterSocket {
            game: Arc::clone(self),
            hero: hero.to_owned(),
        }))
    }

    pub fn override_notice(&self) -> Result<Option<Value>> {
        let voting = self.voting.lock().map_err(ApiError::internal)?;
        Ok(voting
            .pending
            .as_ref()
            .map(Proposal::proposed)
            .or_else(|| voting.last_resolution.clone()))
    }

    pub fn override_command(
        &self,
        server: &Server,
        actor: &str,
        kind: &str,
        body: Value,
    ) -> Result<Option<(String, Instant)>> {
        let store = &server.store;
        let mut current = self.current.lock().map_err(ApiError::internal)?;
        self.check_active()?;
        let mut voting = self.voting.lock().map_err(ApiError::internal)?;
        if !voting.sockets.contains_key(actor) || self.is_bot(actor) {
            return Err(ApiError::forbidden("Only connected human players can vote"));
        }
        if let Some(proposal) = voting
            .pending
            .as_ref()
            .filter(|p| p.deadline <= Instant::now())
            .cloned()
        {
            self.finish_vote(server, &mut current, &mut voting, proposal, "expired")?;
        }
        if kind == "PROPOSE_OVERRIDE" {
            let action = Action::from_request(self, &body)?;
            if voting.pending.is_some() {
                return Err(ApiError::conflict(
                    "Another override proposal is already open",
                ));
            }
            if let Action::Patch(command) = &action {
                current.validate_patch(command)?;
                current.validate_patch_proposer(command, actor)?;
            }
            if matches!(action, Action::Unstick(_)) && current.clock_paused() {
                return Err(ApiError::conflict(
                    "The match is paused. Ready up before using recovery.",
                ));
            }
            if let Action::Rewind(to) = action {
                if current.clock_paused() {
                    return Err(ApiError::conflict(
                        "The match is paused. Ready up before rewinding.",
                    ));
                }
                let start = store.replay_start_revision(&self.id)?;
                let total = current.position().revision - start;
                if to > total {
                    return Err(ApiError::invalid(format!(
                        "Rewind target {to} out of range 0..{total}"
                    )));
                }
            }
            let seconds = std::env::var("GOA2_OVERRIDE_TIMEOUT_SECONDS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|seconds| (1..=3600).contains(seconds))
                .unwrap_or(120);
            let proposal = Proposal {
                action,
                id: Uuid::new_v4().simple().to_string(),
                proposer: actor.to_owned(),
                eligible: voting.sockets.keys().cloned().collect(),
                votes: BTreeMap::from([(actor.to_owned(), true)]),
                deadline: Instant::now() + Duration::from_secs(seconds),
                expires_at: wall_clock_ms() as f64 / 1000.0 + seconds as f64,
            };
            let immediate = proposal.threshold() == 1;
            if immediate && matches!(proposal.action, Action::Unstick(_) | Action::Patch(_)) {
                let (update, rejection) =
                    self.commit_table_edit(server, &mut current, &proposal)?;
                self.publish_notice(proposal.proposed());
                let outcome = if rejection.is_some() {
                    "rejected"
                } else {
                    "applied"
                };
                self.publish_resolution(&mut voting, &proposal, outcome, rejection.as_deref());
                if let Some(update) = update {
                    let _ = self.updates.send(GameEvent::State(update));
                }
                return Ok(None);
            }

            let change = match &proposal.action {
                Action::Pause if immediate => Some(PauseVoteChange::Approve(actor)),
                Action::Pause => Some(PauseVoteChange::Open(&proposal.id)),
                Action::RevealPlayer(_)
                | Action::Rewind(_)
                | Action::Unstick(_)
                | Action::Patch(_)
                    if !immediate && current.clock_running() =>
                {
                    Some(PauseVoteChange::Open(&proposal.id))
                }
                Action::RevealPlayer(_)
                | Action::Rewind(_)
                | Action::Unstick(_)
                | Action::Patch(_) => None,
            };
            let update = if immediate && let Action::Rewind(to) = proposal.action {
                Some(self.commit_rewind(server, &mut current, actor, to)?)
            } else if let Some(change) = change {
                let now = self.clock_now();
                if self.apply_due_timeout(store, &mut current, now)? {
                    return Err(ApiError::conflict("Clock advanced. Retry the proposal."));
                }
                Some(self.commit_vote_clock(store, &mut current, actor, change, now)?)
            } else {
                None
            };
            self.publish_notice(proposal.proposed());
            let schedule = if immediate {
                self.publish_resolution(&mut voting, &proposal, "applied", None);
                None
            } else {
                let schedule = (proposal.id.clone(), proposal.deadline);
                voting.pending = Some(proposal);
                Some(schedule)
            };
            if let Some(update) = update {
                let _ = self.updates.send(GameEvent::State(update));
            }
            return Ok(schedule);
        }

        let mut proposal = voting
            .pending
            .as_ref()
            .filter(|p| body["proposal_id"] == p.id)
            .cloned()
            .ok_or_else(|| ApiError::conflict("No matching open proposal"))?;
        if kind == "CANCEL_OVERRIDE" {
            if actor != proposal.proposer {
                return Err(ApiError::forbidden("Only the proposer may cancel the vote"));
            }
            self.finish_vote(server, &mut current, &mut voting, proposal, "cancelled")?;
        } else {
            if !proposal.eligible.iter().any(|hero| hero == actor) {
                return Err(ApiError::forbidden(
                    "Only players connected at proposal time may vote",
                ));
            }
            let approve = body["approve"]
                .as_bool()
                .ok_or_else(|| ApiError::invalid("approve must be a boolean"))?;
            proposal.votes.insert(actor.to_owned(), approve);
            if let Some(outcome) = proposal.outcome() {
                self.finish_vote(server, &mut current, &mut voting, proposal, outcome)?;
            } else {
                self.publish_notice(json!({"type": "OVERRIDE_UPDATED",
                    "proposal_id": proposal.id, "tally": proposal.tally()}));
                voting.pending = Some(proposal);
            }
        }
        Ok(None)
    }

    fn commit_vote_clock(
        &self,
        store: &Store,
        current: &mut Arc<LiveGame>,
        actor: &str,
        change: PauseVoteChange<'_>,
        now: u64,
    ) -> Result<Update> {
        let (next, transition) = current.prepare_pause_vote(change, now)?;
        self.save_vote_transition(
            store,
            current,
            actor,
            next,
            transition,
            Decision::pause(now, change),
        )
    }

    fn commit_rewind(
        &self,
        server: &Server,
        current: &mut Arc<LiveGame>,
        actor: &str,
        to: u64,
    ) -> Result<Update> {
        let _permit = server.share_baker.try_acquire().map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "Replay worker is busy. Retry the vote.",
            )
        })?;
        let archive = self.replay(&server.store)?;
        let revision = archive.start_revision + to;
        let (target, _) = archive.restore(Some(revision))?;
        let now = self.clock_now();
        let (next, transition) = current.prepare_table_rewind(target, now)?;
        let decision = Decision {
            at_ms: now,
            rebase_ms: None,
            operation: Operation::Rewind {
                proposer: actor.to_owned(),
                to,
            },
        };
        self.save_vote_transition(&server.store, current, actor, next, transition, decision)
    }

    fn commit_table_edit(
        &self,
        server: &Server,
        current: &mut Arc<LiveGame>,
        proposal: &Proposal,
    ) -> Result<(Option<Update>, Option<String>)> {
        let now = self.clock_now();
        let budget = ExecutionBudget::default();
        let prepared = match &proposal.action {
            Action::Unstick(command) => current.prepare_unstick(command, now, &budget),
            Action::Patch(command) => current
                .validate_patch_proposer(command, &proposal.proposer)
                .and_then(|()| current.prepare_patch(command, now, &budget)),
            _ => unreachable!(),
        };
        match prepared {
            Ok((next, transition)) => {
                let proposer = proposal.proposer.clone();
                let operation = match &proposal.action {
                    Action::Unstick(command) => Operation::Unstick {
                        proposer,
                        command: command.clone(),
                    },
                    Action::Patch(command) => Operation::Patch {
                        proposer,
                        command: command.clone(),
                    },
                    _ => unreachable!(),
                };
                let decision = Decision {
                    at_ms: now,
                    rebase_ms: None,
                    operation,
                };
                let update = self.save_vote_transition(
                    &server.store,
                    current,
                    &proposal.proposer,
                    next,
                    transition,
                    decision,
                )?;
                Ok((Some(update), None))
            }
            Err(error) => {
                let update = if current.pause_vote_hold() == Some(proposal.id.as_str()) {
                    Some(self.commit_vote_clock(
                        &server.store,
                        current,
                        &proposal.proposer,
                        PauseVoteChange::Cancel,
                        now,
                    )?)
                } else {
                    None
                };
                let reason = match error {
                    EngineError::InvalidInput(message)
                        if matches!(
                            message.as_str(),
                            "The match is paused"
                                | "No action is in progress"
                                | "No pending input request to skip"
                                | "The pending input does not accept SKIP"
                                | "This recovery requires the resolution phase"
                                | "Unknown lane"
                                | "Level must be between 1 and 8"
                                | "Patch values must be non-negative"
                                | "unknown game hero"
                                | "Unknown entity"
                                | "Specify a concrete hero piece"
                                | "Entity is not on the board"
                                | "Destination is outside the board"
                                | "Destination is occupied"
                                | "Invalid active effect payload"
                                | "Unsupported effect finishing steps"
                                | "An effect with that ID is already active"
                                | "Effect ID is reserved for a future generated effect"
                                | "No active effect matches that ID"
                                | "Only the owner can propose a correction to a private card"
                                | "No movable card matches the supplied ID"
                                | "No available played slot for a finished turn"
                                | "No item and matching upgrade in this deck"
                                | "The matching upgrade cannot be swapped from its current zone"
                        ) =>
                    {
                        message
                    }
                    _ => "The selected operation could not be applied".into(),
                };
                Ok((update, Some(reason)))
            }
        }
    }

    fn save_vote_transition(
        &self,
        store: &Store,
        current: &mut Arc<LiveGame>,
        actor: &str,
        next: LiveGame,
        transition: GameTransition,
        decision: Decision,
    ) -> Result<Update> {
        store.commit(
            &self.id,
            current.position().revision,
            next.position().revision,
            &next.checkpoint()?,
            None,
            decision,
            HistoryContext::new(&current.position(), Some(actor)),
        )?;
        let next = Arc::new(next);
        *current = Arc::clone(&next);
        Ok(Update {
            revision: next,
            transition: Arc::new(transition),
            actor: actor.to_owned(),
            action_id: None,
        })
    }

    fn publish_notice(&self, value: Value) {
        let _ = self
            .updates
            .send(GameEvent::OverrideNotice(Arc::new(value)));
    }

    fn publish_resolution(
        &self,
        voting: &mut Voting,
        proposal: &Proposal,
        outcome: &str,
        rejection: Option<&str>,
    ) {
        voting.pending = None;
        if let Some(task) = voting.expiry_task.take() {
            task.abort();
        }
        let resolved = proposal.resolved(outcome, rejection);
        voting.last_resolution = Some(resolved.clone());
        self.publish_notice(resolved);
        if outcome == "applied"
            && let Action::RevealPlayer(hero) = &proposal.action
        {
            // Broadcast only a seat ID. The socket supplies its credential
            // after checking the recipient, and reconnect notices omit it.
            let _ = self
                .updates
                .send(GameEvent::PlayerLinkRevealed(hero.clone()));
        }
    }

    fn finish_vote(
        &self,
        server: &Server,
        current: &mut Arc<LiveGame>,
        voting: &mut Voting,
        proposal: Proposal,
        outcome: &str,
    ) -> Result<()> {
        if outcome == "applied" && matches!(proposal.action, Action::Unstick(_) | Action::Patch(_))
        {
            let (update, rejection) = self.commit_table_edit(server, current, &proposal)?;
            let outcome = if rejection.is_some() {
                "rejected"
            } else {
                "applied"
            };
            self.publish_resolution(voting, &proposal, outcome, rejection.as_deref());
            if let Some(update) = update {
                let _ = self.updates.send(GameEvent::State(update));
            }
            return Ok(());
        }
        let update = match &proposal.action {
            Action::Rewind(to) if outcome == "applied" => {
                Some(self.commit_rewind(server, current, &proposal.proposer, *to)?)
            }
            action => {
                let change = match action {
                    Action::Pause if outcome == "applied" => {
                        Some(PauseVoteChange::Approve(&proposal.proposer))
                    }
                    Action::Pause => Some(PauseVoteChange::Cancel),
                    _ if current.pause_vote_hold() == Some(proposal.id.as_str()) => {
                        Some(PauseVoteChange::Cancel)
                    }
                    _ => None,
                };
                change
                    .map(|change| {
                        self.commit_vote_clock(
                            &server.store,
                            current,
                            &proposal.proposer,
                            change,
                            self.clock_now(),
                        )
                    })
                    .transpose()?
            }
        };
        self.publish_resolution(voting, &proposal, outcome, None);
        if let Some(update) = update {
            let _ = self.updates.send(GameEvent::State(update));
        }
        Ok(())
    }

    pub fn schedule_vote_expiry(
        self: &Arc<Self>,
        server: &Arc<super::Server>,
        id: String,
        deadline: Instant,
    ) {
        let game = Arc::downgrade(self);
        let server = Arc::clone(server);
        let expiry_id = id.clone();
        let mut retired = self.retired.subscribe();
        let mut shutdown = server.shutdown.subscribe();
        let task = tokio::spawn(async move {
            tokio::select! {
                _ = super::wait_for_shutdown(&mut shutdown) => return,
                _ = retired.wait_for(|retired| *retired) => return,
                _ = tokio::time::sleep_until(deadline.into()) => {}
            }
            loop {
                let Some(game) = game.upgrade() else {
                    return;
                };
                let id = expiry_id.clone();
                let server = Arc::clone(&server);
                if let Err(error) = blocking(move || game.expire_vote(&server, &id)).await {
                    eprintln!("goa2-game-server: table vote expiry failed: {error}");
                    tokio::select! {
                        _ = super::wait_for_shutdown(&mut shutdown) => return,
                        _ = retired.wait_for(|retired| *retired) => return,
                        _ = tokio::time::sleep(Duration::from_millis(250)) => {}
                    }
                } else {
                    return;
                }
            }
        });
        if let Ok(mut voting) = self.voting.lock()
            && voting.pending.as_ref().is_some_and(|p| p.id == id)
        {
            voting.expiry_task = Some(task.abort_handle());
        } else {
            task.abort();
        }
    }

    fn expire_vote(&self, server: &Server, id: &str) -> Result<()> {
        let mut current = self.current.lock().map_err(ApiError::internal)?;
        if *self.retired.borrow() {
            return Ok(());
        }
        let mut voting = self.voting.lock().map_err(ApiError::internal)?;
        if let Some(proposal) = voting.pending.as_ref().filter(|p| p.id == id).cloned() {
            self.finish_vote(server, &mut current, &mut voting, proposal, "expired")?;
        }
        Ok(())
    }
}
