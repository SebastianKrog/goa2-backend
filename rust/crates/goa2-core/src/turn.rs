//! Action boundaries and turn metadata. No serialization occurs during play.
use crate::{
    codec::StateEnvelope,
    data::{Datum, Symbol, Symbols},
    geometry::Hex,
    world::{EntityKind, NativeWorld},
};
use serde_json::{Value, json};
#[derive(Clone, Debug)]
pub enum FlowStep {
    Next,
    Finalize(Symbol),
    Confirm(Symbol),
    Respawn(Symbol),
    Tie(Vec<Symbol>),
    Advance,
    EndPhase,
    Cleanup,
    Upgrades,
    Reset,
}
#[derive(Clone, Debug)]
pub struct Boundary {
    guess: Datum,
    reveal: Datum,
    positions: Vec<(Symbol, Hex)>,
}
impl Boundary {
    pub fn compile(source: &StateEnvelope, s: &mut Symbols) -> Result<Self, String> {
        let state = &source.state;
        Ok(Self {
            guess: Datum::from_json(&state["card_guess"], s)?,
            reveal: Datum::from_json(&state["card_reveal"], s)?,
            positions: state["last_turn_positions"]
                .as_object()
                .ok_or("turn positions missing")?
                .iter()
                .map(|(id, h)| {
                    Ok((
                        s.intern(id),
                        serde_json::from_value(h.clone())
                            .map_err(|e| format!("turn snapshot hex: {e}"))?,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?,
        })
    }
    pub fn finalize(&mut self, hero: Symbol, s: &Symbols) {
        for (value, key) in [
            (&mut self.guess, "guesser_id"),
            (&mut self.reveal, "revealer_id"),
        ] {
            if value.truthy(s)
                && !matches!(value,Datum::Object(fields) if fields.iter().any(|(k,v)|s.name(*k)==key && *v==Datum::Text(hero)))
            {
                *value = Datum::Null;
            }
        }
    }
    pub fn snapshot(&mut self, world: &NativeWorld) {
        self.positions.clone_from(&world.locations);
    }
    pub fn export(&self, state: &mut Value, s: &Symbols) {
        state["card_guess"] = self.guess.to_json(s);
        state["card_reveal"] = self.reveal.to_json(s);
        state["last_turn_positions"] = Value::Object(
            self.positions
                .iter()
                .map(|(id, h)| (s.name(*id).into(), json!(h)))
                .collect(),
        );
    }
}
#[derive(Clone, Debug)]
pub struct TurnRules {
    pub frozen: Symbol,
    pub reanchor: Symbol,
    pub confirm: Symbol,
    pub confirm_prompt: Symbol,
    pub skipped: Symbol,
    pub owners: [Symbol; 2],
    pub teams: [Symbol; 2],
    pub prompts: Vec<(Symbol, Symbol, Symbol)>,
}
impl TurnRules {
    pub fn compile(world: &NativeWorld, s: &mut Symbols) -> Self {
        let teams = [s.intern("RED"), s.intern("BLUE")];
        Self {
            frozen: s.intern("rollback_frozen"),
            reanchor: s.intern("rollback_reanchor_pending"),
            confirm: s.intern("CONFIRM"),
            confirm_prompt: s.intern("Confirm your action."),
            skipped: s.intern("skipped_respawn"),
            owners: teams.map(|t| s.intern(&format!("team:{}", s.name(t)))),
            teams,
            prompts: world
                .heroes
                .iter()
                .map(|h| {
                    let choice = s.intern(&format!(
                        "Hero {} is defeated. Respawn at an empty spawn point?",
                        s.name(h.id)
                    ));
                    let hex = s.intern(&format!("Select spawn location for {}", s.name(h.id)));
                    (h.id, choice, hex)
                })
                .collect(),
        }
    }
    pub fn respawn_candidates(
        &self,
        world: &NativeWorld,
        hero: Symbol,
    ) -> Result<Vec<Hex>, String> {
        let Some(team) = world.entity(hero).and_then(|h| h.team) else {
            return Ok(vec![]);
        };
        let spawns = world
            .definitions
            .zones
            .hero_spawns
            .iter()
            .filter_map(|(t, h)| (*t == team).then_some(*h))
            .collect::<Vec<_>>();
        let mut valid = Vec::new();
        for hex in &spawns {
            if !world.obstacle_for_actor(*hex, Some(hero))? || Self::token(world, *hex).is_some() {
                valid.push(*hex);
            }
        }
        if !valid.is_empty() {
            return Ok(valid);
        }
        for origin in spawns {
            if let Some(zone) = world
                .zone_at(origin)
                .filter(|z| *z != world.definitions.zones.empty)
                .and_then(|id| world.definitions.zones.zones.iter().find(|z| z.id == id))
            {
                let candidates = crate::minion_return::nearest(world, origin, &zone.hexes, false).0;
                if !candidates.is_empty() {
                    return Ok(candidates);
                }
            }
        }
        Ok(vec![])
    }
    pub fn token(world: &NativeWorld, hex: Hex) -> Option<Symbol> {
        (!world.terrain(hex))
            .then(|| world.occupant(hex))
            .flatten()
            .filter(|id| {
                world
                    .entity(*id)
                    .is_some_and(|e| e.kind == EntityKind::Token)
            })
    }
}
