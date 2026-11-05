//! Automatic after-action returns to each minion's own lane Battle Zone.
//! Actor-aware topology, traversal, displacement and placement legality.
use crate::{
    data::{Registers, Symbol, Symbols},
    geometry::Hex,
    validation::DisplacementKind,
    world::{EntityKind, NativeWorld},
};
use std::collections::{HashSet, VecDeque};

#[derive(Clone, Debug)]
pub struct ReturnRules {
    prompts: Vec<(Symbol, Symbol, Symbol)>,
}
#[derive(Clone, Debug)]
pub struct ReturnPlan {
    pub unit: Symbol,
    pub owner: Symbol,
    pub prompt: Symbol,
    pub candidates: Vec<Hex>,
    pub distance: i32,
    pub placement: bool,
    pub more: bool,
}
#[derive(Clone, Debug, Hash)]
pub enum ReturnBinding {
    Processed(Vec<Symbol>),
    Relocate {
        unit: Symbol,
        destination: Hex,
        range: Option<i32>,
    },
}
impl ReturnRules {
    pub fn compile(world: &NativeWorld, symbols: &mut Symbols) -> Self {
        let prompts = world
            .definitions
            .entities
            .iter()
            .filter(|e| e.kind == EntityKind::Minion)
            .filter_map(|e| e.team.map(|t| (e.id, t)))
            .map(|(id, team)| {
                let owner = symbols.intern(&format!("team:{}", symbols.name(team)));
                let prompt = symbols.intern(&format!(
                    "Team {}, choose destination for {} to return to Battle Zone.",
                    symbols.name(team),
                    symbols.name(id)
                ));
                (id, owner, prompt)
            })
            .collect();
        Self { prompts }
    }
    pub fn next(
        &self,
        world: &NativeWorld,
        symbols: &Symbols,
        processed: &[Symbol],
        favored: Symbol,
        actor: Option<Symbol>,
        context: &Registers,
    ) -> Result<Option<ReturnPlan>, String> {
        let mut outside = world
            .definitions
            .entities
            .iter()
            .filter(|e| e.kind == EntityKind::Minion && !processed.contains(&e.id))
            .filter_map(|e| {
                let team = e.team?;
                let position = world.position(e.id)?;
                let zone = e.lane.and_then(|lane| {
                    world
                        .battle_zones
                        .iter()
                        .find(|(l, _)| *l == lane)
                        .map(|(_, z)| *z)
                        .filter(|z| *z != world.definitions.zones.empty)
                })?;
                let zone = world
                    .definitions
                    .zones
                    .zones
                    .iter()
                    .find(|z| z.id == zone)?;
                (!zone.hexes.contains(&position)).then_some((e.id, team, position, zone))
            })
            .collect::<Vec<_>>();
        outside.sort_by(|a, b| {
            (a.1 != favored, symbols.name(a.0)).cmp(&(b.1 != favored, symbols.name(b.0)))
        });
        let Some((unit, _, position, zone)) = outside.first() else {
            return Ok(None);
        };
        if world.has_uncompiled_auras {
            return Err("minion return card aura restrictions are not compiled".into());
        }
        let actor = actor
            .filter(|id| *id != world.definitions.effect_empty)
            .unwrap_or(*unit);
        let (mut candidates, mut distance) =
            effect_nearest(world, *position, &zone.hexes, *unit, actor, context, true)?;
        let placement = candidates.is_empty();
        if placement {
            (candidates, distance) =
                effect_nearest(world, *position, &zone.hexes, *unit, actor, context, false)?;
        }
        let (_, owner, prompt) = self
            .prompts
            .iter()
            .find(|(id, _, _)| id == unit)
            .ok_or("minion return prompt missing")?;
        Ok(Some(ReturnPlan {
            unit: *unit,
            owner: *owner,
            prompt: *prompt,
            candidates,
            distance,
            placement,
            more: outside.len() > 1,
        }))
    }
}
// Match the reference's ordered breadth-first layers. A placement search crosses
// obstacles but still requires an actual connected map path and an empty landing.
pub(crate) fn nearest(
    world: &NativeWorld,
    start: Hex,
    zone: &HashSet<Hex>,
    walk: bool,
) -> (Vec<Hex>, i32) {
    let mut queue = VecDeque::from([(start, 0)]);
    let mut visited = HashSet::from([start]);
    let mut found = None;
    let mut candidates = Vec::new();
    while let Some((hex, distance)) = queue.pop_front() {
        if found.is_some_and(|d| distance > d) {
            break;
        }
        if distance > 0 && zone.contains(&hex) && world.on_map(hex) && !world.obstacle(hex) {
            candidates.push(hex);
            found = Some(distance);
        }
        if found.is_none() {
            for next in hex.neighbors() {
                if world.on_map(next) && (!walk || !world.obstacle(next)) && visited.insert(next) {
                    queue.push_back((next, distance + 1));
                }
            }
        }
    }
    (candidates, found.unwrap_or(0))
}

// Keep selection order and the distinction between bare landing obstacles and
// actor-aware traversal. Rejected destinations must not stop the BFS layer.
fn effect_nearest(
    world: &NativeWorld,
    start: Hex,
    zone: &HashSet<Hex>,
    unit: Symbol,
    actor: Symbol,
    context: &Registers,
    walk: bool,
) -> Result<(Vec<Hex>, i32), String> {
    let mut queue = VecDeque::from([(start, 0)]);
    let mut visited = HashSet::from([start]);
    let mut found = None;
    let mut candidates = Vec::new();
    while let Some((hex, distance)) = queue.pop_front() {
        if found.is_some_and(|d| distance > d) {
            break;
        }
        if distance > 0 && zone.contains(&hex) && world.on_map(hex) && !world.obstacle(hex) {
            let allowed = if walk {
                world
                    .can_displace(unit, Some(actor), DisplacementKind::Move, None)?
                    .allowed
                    && world
                        .movement_legality(
                            unit,
                            i64::from(start.distance(hex)),
                            false,
                            None,
                            context,
                        )?
                        .allowed
            } else {
                world.effect_connected(start, hex, &[unit])?
                    && world
                        .can_displace(unit, Some(actor), DisplacementKind::Place, Some(hex))?
                        .allowed
            };
            if allowed {
                candidates.push(hex);
                found = Some(distance);
            }
        }
        if found.is_none() {
            for next in hex.neighbors() {
                if visited.contains(&next)
                    || !world.on_map(next)
                    || !world.effect_connected(hex, next, &[unit])?
                {
                    continue;
                }
                if walk
                    && (!world.effect_connected(start, next, &[unit])?
                        || (world.obstacle_for_actor(next, Some(unit))?
                            && !world.passable_token(next, &[])))
                {
                    continue;
                }
                visited.insert(next);
                queue.push_back((next, distance + 1));
            }
        }
    }
    Ok((candidates, found.unwrap_or(0)))
}
