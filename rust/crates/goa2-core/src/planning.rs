//! Shared native planning/revelation buffers and initiative/tie scheduling.
//! Planning diagnostics stop at queued continuations; the stack executor uses
//! the same phase/actor state and handles its turn-end result.
use crate::{
    cards::{CardError, CardId, reference, wire},
    codec::{Phase, StateEnvelope},
    data::{Datum, Symbol, Symbols},
    stats::Stat,
    world::NativeWorld,
};
use serde_json::{Value, json};

type Result<T> = std::result::Result<T, CardError>;
#[derive(Clone, Debug)]
pub enum Continuation {
    Retrieve(Symbol),
    FindActor,
    Tie(Vec<Symbol>),
    Respawn(Symbol),
    Resolve(Symbol),
    Confirm(Symbol),
    Finalize(Symbol),
    FlipCoin,
}
#[derive(Clone, Debug)]
pub struct TieChoice {
    pub team: Symbol,
    pub candidates: Vec<Symbol>,
}
#[derive(Clone, Debug)]
pub struct Planning {
    pub phase: Phase,
    two_card_effects: Vec<Symbol>,
    executing: bool,
    end_turn: bool,
    pending: Vec<(Symbol, Option<CardId>)>,
    second: Vec<(Symbol, CardId)>,
    done: Vec<Symbol>,
    requests: Vec<(String, Symbol, Symbol)>,
    pub(crate) actor: Option<Symbol>,
    pub(crate) owner: Option<Symbol>,
    /// Stored in actual LIFO order, exactly as on the engine stack.
    pub continuation: Vec<Continuation>,
}
fn illegal(message: &'static str) -> CardError {
    CardError::Illegal(message)
}
fn unsupported(message: &'static str) -> CardError {
    CardError::Unsupported(message)
}
impl Planning {
    pub fn compile(
        source: &StateEnvelope,
        symbols: &mut Symbols,
    ) -> std::result::Result<Self, String> {
        let header = source.validate()?;
        if !matches!(header.phase, Phase::Planning | Phase::Resolution)
            || !header.execution_stack.is_empty()
        {
            return Err("planning diagnostics require an empty planning/resolution stack".into());
        }
        let mut planning = Self::compile_flow(source, symbols)?;
        planning.executing = false;
        Ok(planning)
    }
    pub(crate) fn compile_flow(
        source: &StateEnvelope,
        symbols: &mut Symbols,
    ) -> std::result::Result<Self, String> {
        source.validate()?;
        if source.state["active_effects"]
            .as_array()
            .is_none_or(|v| !v.is_empty())
        {
            return Err("planning active-effect closure not compiled".into());
        }
        Self::compile_effect_boundary(source, symbols)
    }
    /// Parse bookkeeping for a resolution-only lifecycle diagnostic. Callers
    /// must reject all gameplay/input operations while effects are unported.
    pub(crate) fn compile_effect_boundary(
        source: &StateEnvelope,
        symbols: &mut Symbols,
    ) -> std::result::Result<Self, String> {
        let header = source.validate()?;
        let state = &source.state;
        let mut pending = Vec::new();
        for (hero, card) in state["pending_inputs"]
            .as_object()
            .ok_or("pending inputs missing")?
        {
            pending.push((symbols.intern(hero), reference(card)?));
        }
        let mut second = Vec::new();
        for (hero, card) in state["pending_second_cards"]
            .as_object()
            .ok_or("second cards missing")?
        {
            second.push((
                symbols.intern(hero),
                reference(card)?.ok_or("null second card")?,
            ));
        }
        let mut list = |key: &str| -> std::result::Result<Vec<Symbol>, String> {
            state[key]
                .as_array()
                .ok_or_else(|| format!("missing {key}"))?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(|s| symbols.intern(s))
                        .ok_or("non-string hero".into())
                })
                .collect()
        };
        let done = list("planning_done")?;
        let mut requests = Vec::new();
        for (key, req) in state["starting_position_requests"]
            .as_object()
            .ok_or("starting requests missing")?
        {
            requests.push((
                key.clone(),
                symbols.intern(req["from"].as_str().ok_or("request from missing")?),
                symbols.intern(req["to"].as_str().ok_or("request to missing")?),
            ));
        }
        let templates = crate::templates::instructions()?;
        if templates["source_sha256"] != source.source_sha256 {
            return Err("phase template identity mismatch".into());
        }
        let two_card_effects = templates["two_card_effects"]
            .as_array()
            .ok_or("planning capabilities missing")?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(|s| symbols.intern(s))
                    .ok_or("invalid capability".to_string())
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(Self {
            two_card_effects,
            executing: true,
            end_turn: false,
            phase: header.phase,
            pending,
            second,
            done,
            requests,
            actor: state["current_actor_id"]
                .as_str()
                .map(|s| symbols.intern(s)),
            owner: state["resolution_owner_id"]
                .as_str()
                .map(|s| symbols.intern(s)),
            continuation: Vec::new(),
        })
    }
    fn two_cards(&self, world: &NativeWorld, hero: Symbol) -> bool {
        world.hero(hero).is_some_and(|h| {
            h.level >= 8
                && h.ultimate.and_then(|id| world.card(id)).is_some_and(|c| {
                    self.two_card_effects
                        .contains(&world.definitions.cards[c.definition].effect)
                })
        })
    }
    fn open(&self, world: &NativeWorld, hero: Symbol) -> bool {
        self.pending.iter().any(|(h, c)| *h == hero && c.is_some())
            && self.two_cards(world, hero)
            && !self.second.iter().any(|(h, _)| *h == hero)
            && !self.done.contains(&hero)
            && world.hero(hero).is_some_and(|h| !h.hand.is_empty())
    }
    pub fn commit(&mut self, world: &mut NativeWorld, hero: Symbol, card: CardId) -> Result<()> {
        if self.phase != Phase::Planning {
            return Ok(());
        }
        // Phase API resolves heroes by exact identity; piece IDs are not players.
        let Some(index) = world.heroes.iter().position(|h| h.id == hero) else {
            return Ok(());
        };
        let is_second = if let Some((_, first)) = self.pending.iter().find(|(h, _)| *h == hero) {
            if first.is_none() || !self.two_cards(world, hero) {
                return Err(illegal("already committed"));
            }
            if self.second.iter().any(|(h, _)| *h == hero) || self.done.contains(&hero) {
                return Err(illegal("planning already closed"));
            }
            true
        } else {
            false
        };
        if !world.heroes[index].hand.contains(&card) {
            return Ok(());
        }
        match world.play_card(hero, card) {
            Ok(()) => {}
            Err(CardError::Illegal(_)) => return Ok(()),
            Err(e) => return Err(e),
        }
        if is_second {
            self.second.push((hero, card));
        } else {
            self.requests
                .retain(|(_, from, to)| *from != hero && *to != hero);
            self.pending.push((hero, Some(card)));
        }
        self.check_transition(world)
    }
    pub fn uncommit(&mut self, world: &mut NativeWorld, hero: Symbol) -> Result<()> {
        if self.phase != Phase::Planning {
            return Err(illegal("not planning"));
        }
        let index = world
            .heroes
            .iter()
            .position(|h| h.id == hero)
            .ok_or(illegal("unknown hero"))?;
        let pos = self
            .pending
            .iter()
            .position(|(h, _)| *h == hero)
            .ok_or(illegal("no commit"))?;
        let first = self.pending[pos].1.ok_or(illegal("passed"))?;
        if let Some(second) = self.second.iter().position(|(h, _)| *h == hero) {
            let card = self.second.remove(second).1;
            world.unplay_card(hero, card)?;
            world.heroes[index].current = Some(first);
        } else {
            self.pending.remove(pos);
            world.unplay_card(hero, first)?;
            world.heroes[index].current = None;
        }
        self.done.retain(|h| *h != hero);
        Ok(())
    }
    pub fn pass(&mut self, world: &mut NativeWorld, hero: Symbol) -> Result<()> {
        if self.phase != Phase::Planning {
            return Err(illegal("not planning"));
        }
        let h = world
            .heroes
            .iter()
            .find(|h| h.id == hero)
            .ok_or(illegal("unknown hero"))?;
        if self.pending.iter().any(|(h, _)| *h == hero) || !h.hand.is_empty() {
            return Err(illegal("cannot pass"));
        }
        self.requests
            .retain(|(_, from, to)| *from != hero && *to != hero);
        self.pending.push((hero, None));
        self.check_transition(world)
    }
    pub fn finish(&mut self, world: &mut NativeWorld, hero: Symbol) -> Result<()> {
        if self.phase != Phase::Planning {
            return Ok(());
        }
        if !self.pending.iter().any(|(h, _)| *h == hero) {
            return Err(illegal("must commit first"));
        }
        if !self.done.contains(&hero) {
            self.done.push(hero);
        }
        self.check_transition(world)
    }
    fn check_transition(&mut self, world: &mut NativeWorld) -> Result<()> {
        if self.transition(world)? {
            if self.executing {
                self.end_turn = true;
            } else {
                return Err(unsupported("all-pass turn-end closure not compiled"));
            }
        }
        Ok(())
    }
    pub(crate) fn transition(&mut self, world: &mut NativeWorld) -> Result<bool> {
        if self.pending.len() < world.heroes.len()
            || self.pending.iter().any(|(h, _)| self.open(world, *h))
        {
            return Ok(false);
        }
        self.phase = Phase::Revelation;
        world.combat.unresolved.clear();
        for (hero, first) in &self.pending {
            let Some(card) = first else {
                if world.heroes.iter().any(|h| h.id == *hero) {
                    world.resolve_current_card(*hero, Some(world.combat.turn))?;
                }
                continue;
            };
            let Some(index) = world.heroes.iter().position(|h| h.id == *hero) else {
                continue;
            };
            let reveal = |world: &mut NativeWorld, card: CardId| -> Result<()> {
                let instance = world
                    .card_mut(card)
                    .ok_or(unsupported("missing pending card"))?;
                instance.facedown = false;
                let definition = instance.definition;
                let printed = world.definitions.cards[definition].printed_id;
                world.history.reveal(*hero, printed);
                Ok(())
            };
            reveal(world, *card)?;
            world.heroes[index].current = Some(*card);
            if let Some((_, second)) = self.second.iter().find(|(h, _)| h == hero) {
                reveal(world, *second)?;
                world.heroes[index].extra = Some(*second);
            }
            world.combat.unresolved.push(*hero);
        }
        self.pending.clear();
        self.second.clear();
        self.done.clear();
        self.phase = Phase::Resolution;
        let dual = world
            .combat
            .unresolved
            .iter()
            .copied()
            .filter(|h| world.hero(*h).is_some_and(|h| h.extra.is_some()))
            .collect::<Vec<_>>();
        if !dual.is_empty() {
            self.continuation.push(Continuation::FindActor);
            self.continuation
                .extend(dual.into_iter().rev().map(Continuation::Retrieve));
            return Ok(false);
        }
        self.next_actor(world)
    }
    pub(crate) fn next_actor(&mut self, world: &mut NativeWorld) -> Result<bool> {
        if world.combat.unresolved.is_empty() {
            return Ok(true);
        }
        let mut candidates = Vec::new();
        for hero in &world.combat.unresolved {
            if let Some(card) = world
                .hero(*hero)
                .and_then(|h| h.current)
                .and_then(|id| world.card(id))
            {
                let base = if card.facedown {
                    0
                } else {
                    card.initiative_override
                        .unwrap_or(world.definitions.cards[card.definition].initiative)
                };
                let init = world
                    .computed_stat(*hero, Stat::Initiative, base)
                    .map_err(|_| unsupported("initiative effect/aura closure not compiled"))?;
                candidates.push((*hero, init));
            }
        }
        let Some(max) = candidates.iter().map(|(_, i)| *i).max() else {
            return Ok(false);
        };
        let tied = candidates
            .into_iter()
            .filter_map(|(h, i)| (i == max).then_some(h))
            .collect::<Vec<_>>();
        if tied.len() > 1 {
            self.continuation.push(Continuation::Tie(tied));
            return Ok(false);
        }
        let hero = tied[0];
        let presence = world.has_board_presence(hero);
        self.schedule_actor(world, hero, !presence, false);
        Ok(false)
    }
    pub(crate) fn take_end_turn(&mut self) -> bool {
        std::mem::take(&mut self.end_turn)
    }
    pub(crate) fn auto_pass(&mut self, world: &mut NativeWorld) -> Result<bool> {
        let mut any = false;
        for hero in world.heroes.iter().filter(|h| h.hand.is_empty()) {
            if let Some((_, card)) = self.pending.iter_mut().find(|(h, _)| *h == hero.id) {
                *card = None;
            } else {
                self.pending.push((hero.id, None));
            }
            any = true;
        }
        if any {
            self.transition(world)
        } else {
            Ok(false)
        }
    }
    fn schedule_actor(&mut self, world: &mut NativeWorld, hero: Symbol, respawn: bool, flip: bool) {
        self.actor = Some(hero);
        self.owner = Some(hero);
        // Python list.remove removes only the first occurrence.
        if let Some(index) = world.combat.unresolved.iter().position(|h| *h == hero) {
            world.combat.unresolved.remove(index);
        }
        self.continuation.push(Continuation::Finalize(hero));
        if flip {
            self.continuation.push(Continuation::FlipCoin);
        }
        self.continuation
            .extend([Continuation::Confirm(hero), Continuation::Resolve(hero)]);
        if respawn {
            self.continuation.push(Continuation::Respawn(hero));
        }
    }
    /// Team choice is recomputed for every retry, just like ResolveTieBreakerStep.
    /// Coin flipping is queued after the actor's confirmation, never at selection.
    pub fn resolve_tie(
        &mut self,
        world: &mut NativeWorld,
        tied: &[Symbol],
        favored: Symbol,
        selection: Option<&Datum>,
    ) -> Result<Option<TieChoice>> {
        if tied.is_empty() {
            return Ok(None);
        }
        let mut teams: Vec<(Symbol, Vec<Symbol>)> = Vec::new();
        for id in tied {
            if world.heroes.iter().any(|h| h.id == *id)
                && let Some(team) = world.entity(*id).and_then(|e| e.team)
            {
                if let Some((_, ids)) = teams.iter_mut().find(|(t, _)| *t == team) {
                    ids.push(*id);
                } else {
                    teams.push((team, vec![*id]));
                }
            }
        }
        if teams.is_empty() {
            return Err(illegal("no known tied heroes"));
        }
        let cross = teams.len() > 1;
        let index = if cross {
            teams
                .iter()
                .position(|(team, _)| *team == favored)
                .unwrap_or(0)
        } else {
            0
        };
        let (team, candidates) = &teams[index];
        let winner = if candidates.len() == 1 {
            Some(candidates[0])
        } else {
            match selection {
                Some(Datum::Text(id)) if candidates.contains(id) => Some(*id),
                _ => None,
            }
        };
        let Some(winner) = winner else {
            return Ok(Some(TieChoice {
                team: *team,
                candidates: candidates.clone(),
            }));
        };
        // Preserve the reference tie step's direct-location respawn condition;
        // the respawn instruction itself owns multi-piece presence handling.
        self.schedule_actor(world, winner, world.position(winner).is_none(), cross);
        Ok(None)
    }
    /// Boundary only: reconstruct exact phase records and frozen Python defaults.
    pub fn export(
        &self,
        target: &mut StateEnvelope,
        world: &NativeWorld,
        symbols: &Symbols,
    ) -> std::result::Result<(), String> {
        let templates = crate::templates::instructions()?;
        if templates["source_sha256"] != target.source_sha256 {
            return Err("phase template identity mismatch".into());
        }
        world.export_combat(&mut target.state, symbols);
        self.export_fields(target, symbols)?;
        let state = &mut target.state;
        let mut steps = Vec::new();
        for (i, step) in self.continuation.iter().enumerate() {
            let (key, hero) = match step {
                Continuation::Retrieve(h) => ("retrieve_unresolved", Some(h)),
                Continuation::FindActor => ("find_actor", None),
                Continuation::Tie(_) => ("tie", None),
                Continuation::Respawn(h) => ("respawn", Some(h)),
                Continuation::Resolve(h) => ("resolve_card", Some(h)),
                Continuation::Confirm(h) => ("confirm_resolution", Some(h)),
                Continuation::Finalize(h) => ("finalize_hero", Some(h)),
                Continuation::FlipCoin => ("flip_coin", None),
            };
            let mut value = templates["templates"][key].clone();
            value["step_id"] = json!(format!("native-step-phase-{i}"));
            if let Some(hero) = hero {
                value["hero_id"] = json!(symbols.name(*hero));
            }
            if let Continuation::Tie(tied) = step {
                value["tied_hero_ids"] =
                    json!(tied.iter().map(|h| symbols.name(*h)).collect::<Vec<_>>());
            }
            steps.push(value);
        }
        state["execution_stack"] = json!(steps);
        Ok(())
    }
    pub(crate) fn export_fields(
        &self,
        target: &mut StateEnvelope,
        symbols: &Symbols,
    ) -> std::result::Result<(), String> {
        let state = &mut target.state;
        state["phase"] = serde_json::to_value(self.phase).map_err(|e| e.to_string())?;
        state["pending_inputs"] = Value::Object(
            self.pending
                .iter()
                .map(|(h, c)| (symbols.name(*h).into(), wire(*c)))
                .collect(),
        );
        state["pending_second_cards"] = Value::Object(
            self.second
                .iter()
                .map(|(h, c)| (symbols.name(*h).into(), wire(Some(*c))))
                .collect(),
        );
        state["planning_done"] = json!(
            self.done
                .iter()
                .map(|h| symbols.name(*h))
                .collect::<Vec<_>>()
        );
        // Retain all request fields verbatim, deleting only cleared requests.
        state["starting_position_requests"]
            .as_object_mut()
            .ok_or("requests missing")?
            .retain(|k, _| self.requests.iter().any(|(key, _, _)| key == k));
        state["current_actor_id"] = json!(self.actor.map(|h| symbols.name(h)));
        state["resolution_owner_id"] = json!(self.owner.map(|h| symbols.name(h)));
        Ok(())
    }
}
