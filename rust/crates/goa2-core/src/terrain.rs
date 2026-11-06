//! Stateful obstacles and one-traversal movement queries. No serialized state in rules.
use crate::{
    data::Symbol,
    effect_rules::Kind,
    geometry::{Hex, MovementQuery},
    world::{EntityKind, NativeWorld},
};
use std::collections::VecDeque;

#[derive(Hash, PartialEq, Eq)]
struct TraversalKey {
    start: Hex,
    range: i32,
    through: bool,
    occupancy: Option<crate::geometry::Occupancy>,
    passable: Vec<usize>,
}

#[derive(Hash, PartialEq, Eq)]
struct TravelKey {
    team: Symbol,
    start_zone: Symbol,
    change: bool,
    occupancy: crate::geometry::Occupancy,
    blockers: Vec<Hex>,
}

#[derive(Default)]
pub(crate) struct SearchReuse {
    pub movement_hits: usize,
    pub movement_misses: usize,
    pub travel_hits: usize,
    pub travel_misses: usize,
}

type Traversal = (Vec<bool>, Vec<bool>);

struct TraversalCache {
    // Keep the immutable geometry alive and restrict reuse to this search.
    definitions: std::sync::Arc<crate::world::WorldDefinitions>,
    values: std::collections::HashMap<TraversalKey, Traversal>,
    travel: std::collections::HashMap<TravelKey, Vec<bool>>,
    reuse: SearchReuse,
}

thread_local! {
    static SEARCH_TRAVERSALS: std::cell::RefCell<Option<TraversalCache>> =
        const { std::cell::RefCell::new(None) };
}

pub(crate) fn with_search_traversals<T>(
    definitions: std::sync::Arc<crate::world::WorldDefinitions>,
    operation: impl FnOnce() -> T,
) -> (T, SearchReuse) {
    struct Restore(Option<TraversalCache>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SEARCH_TRAVERSALS.with(|cache| cache.replace(self.0.take()));
        }
    }
    let previous = SEARCH_TRAVERSALS.with(|cache| {
        cache.replace(Some(TraversalCache {
            definitions,
            values: Default::default(),
            travel: Default::default(),
            reuse: SearchReuse::default(),
        }))
    });
    let _restore = Restore(previous);
    let result = operation();
    let reuse = SEARCH_TRAVERSALS
        .with(|cache| std::mem::take(&mut cache.borrow_mut().as_mut().unwrap().reuse));
    (result, reuse)
}

impl NativeWorld {
    /// `actor` is already resolved by the caller (including current-actor fallback).
    pub fn obstacle_for_actor(&self, hex: Hex, actor: Option<Symbol>) -> Result<bool, String> {
        let Some(index) = self.tile_index(hex) else {
            return Ok(true);
        };
        self.obstacle_for_actor_at(hex, index, actor)
    }

    fn obstacle_for_actor_at(
        &self,
        hex: Hex,
        index: usize,
        actor: Option<Symbol>,
    ) -> Result<bool, String> {
        if self.obstacle_at(index) {
            return Ok(true);
        }
        let Some(actor) = actor.and_then(|id| self.entity(id)) else {
            return Ok(false);
        };
        for effect in &self.effects.instances {
            let behavior = &effect.definition.behavior;
            if behavior.kind != Kind::EmptyHexObstacle || !self.effect_active(effect) {
                continue;
            }
            let Some((source_team, actor_team)) = self
                .entity(effect.source)
                .and_then(|e| e.team)
                .zip(actor.team)
            else {
                continue;
            };
            if source_team == actor_team || self.effect_ignored(effect, actor.id)? {
                continue;
            }
            let origin = effect
                .scope_origin
                .filter(|id| *id != self.definitions.effect_empty)
                .unwrap_or(effect.source);
            if let Some(origin) = self.resolved_position(origin)
                && self.effect_connected(origin, hex, &[])?
                && i64::from(origin.distance(hex)) <= behavior.range
            {
                return Ok(true);
            }
        }
        if !matches!(actor.kind, EntityKind::Hero | EntityKind::Piece) {
            return Ok(false);
        }
        for effect in &self.effects.instances {
            let behavior = &effect.definition.behavior;
            if behavior.kind != Kind::StaticBarrier || !self.effect_active(effect) {
                continue;
            }
            let Some((source_team, actor_team)) = self
                .entity(effect.source)
                .and_then(|e| e.team)
                .zip(actor.team)
            else {
                continue;
            };
            if source_team == actor_team || self.effect_ignored(effect, actor.id)? {
                continue;
            }
            let origin = effect
                .barrier_origin
                .filter(|id| *id != self.definitions.effect_empty)
                .unwrap_or(effect.source);
            let Some((origin, actor_hex)) = self
                .resolved_position(origin)
                .zip(self.resolved_position(actor.id))
            else {
                continue;
            };
            let inside = |target: Hex| -> Result<bool, String> {
                Ok(self.effect_connected(origin, target, &[])?
                    && i64::from(origin.distance(target)) <= behavior.barrier_radius)
            };
            if inside(actor_hex)? != inside(hex)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub fn passable_token(&self, hex: Hex, moved: &[Symbol]) -> bool {
        // All admitted rule versions include TOKENS_CANNOT_CROSS_MINES.
        if moved.iter().any(|id| {
            self.entity(*id)
                .is_some_and(|e| e.kind == EntityKind::Token)
        }) {
            return false;
        }
        let Some(id) = self.occupant(hex) else {
            return false;
        };
        if let Some(entity) = self.entity(id) {
            return entity.kind == EntityKind::Token && entity.passable;
        }
        self.definitions
            .supply_passable
            .iter()
            .find(|(candidate, _)| *candidate == id)
            .is_some_and(|(_, passable)| *passable)
    }
    pub fn terrain_hex(&self, hex: Hex) -> Result<bool, String> {
        // Board.get_tile supplies an out-of-map terrain tile in the reference.
        if !self.on_map(hex) {
            return Ok(true);
        }
        if self.terrain(hex) {
            return Ok(true);
        }
        let Some(occupant) = self.occupant(hex) else {
            return Ok(false);
        };
        for effect in &self.effects.instances {
            if effect.definition.behavior.kind != Kind::Petrify || !self.effect_active(effect) {
                continue;
            }
            if self.position(occupant).is_none() {
                return Ok(false);
            }
            if !self.effect_ignored(effect, occupant)?
                && self.effect_in_scope(effect, occupant, true)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// Shared BFS for all destination checks, retaining final-obstacle exemptions.
    pub fn movement_paths(
        &self,
        query: &MovementQuery,
        actor: Option<Symbol>,
        topology_units: Option<&[Symbol]>,
        moved: &[Symbol],
    ) -> Result<Vec<bool>, String> {
        if query.max_steps <= 0 {
            return Ok(vec![false; query.ends.len()]);
        }
        let (queued, terminal) = self.movement_traversal(
            query.start,
            query.max_steps,
            query.pass_through_obstacles,
            actor,
            topology_units,
            moved,
        )?;
        query
            .ends
            .iter()
            .map(|&end| {
                let Some(index) = self.tile_index(end) else {
                    return Ok(false);
                };
                Ok(end != query.start
                    && (queued[index] || (query.ignore_obstacles && terminal[index]))
                    && (query.ignore_obstacles
                        || !self.obstacle_for_actor_at(end, index, actor)?))
            })
            .collect()
    }
    pub fn movement_reachable(
        &self,
        query: &MovementQuery,
        actor: Option<Symbol>,
        topology_units: Option<&[Symbol]>,
        moved: &[Symbol],
    ) -> Result<Vec<bool>, String> {
        let (queued, _) = self.movement_traversal(
            query.start,
            query.max_steps,
            query.pass_through_obstacles,
            actor,
            topology_units,
            moved,
        )?;
        query
            .ends
            .iter()
            .map(|&end| {
                if end == query.start {
                    return Ok(true);
                }
                let Some(index) = self.tile_index(end) else {
                    return Ok(false);
                };
                Ok(queued[index]
                    && (query.ignore_obstacles
                        || query.pass_through_obstacles
                        || !self.obstacle_for_actor_at(end, index, actor)?))
            })
            .collect()
    }
    fn can_reuse_spatial(&self, cache: &TraversalCache) -> bool {
        std::sync::Arc::ptr_eq(&cache.definitions, &self.definitions)
            && !self.effects.instances.iter().any(|effect| {
                matches!(
                    effect.definition.behavior.kind,
                    Kind::Split | Kind::Isolation | Kind::EmptyHexObstacle | Kind::StaticBarrier
                )
            })
    }

    pub(crate) fn reuse_fast_travel_mask(
        &self,
        team: Symbol,
        start_zone: Symbol,
        change: bool,
        candidates: &[Hex],
        compute: impl FnOnce() -> Result<Vec<bool>, String>,
    ) -> Result<Vec<bool>, String> {
        SEARCH_TRAVERSALS.with(|slot| {
            let mut slot = slot.borrow_mut();
            let Some(cache) = slot.as_mut().filter(|cache| self.can_reuse_spatial(cache)) else {
                return compute();
            };
            let mut blockers: Vec<_> = self
                .locations
                .iter()
                .filter_map(|(id, hex)| {
                    self.entity(*id)
                        .is_some_and(|entity| entity.has_team && entity.team != Some(team))
                        .then_some(*hex)
                })
                .collect();
            blockers.sort_unstable_by_key(|hex| hex.coordinates());
            blockers.dedup();
            let key = TravelKey {
                team,
                start_zone,
                change,
                occupancy: self.movement_occupancy().clone(),
                blockers,
            };
            if let Some(mask) = cache.travel.get(&key) {
                cache.reuse.travel_hits += 1;
                return Ok(candidates
                    .iter()
                    .map(|&hex| self.tile_index(hex).is_some_and(|index| mask[index]))
                    .collect());
            }
            let mask = compute()?;
            // A cold execution check for one hex stays a one-hex query.
            if candidates == self.tile_hexes() {
                cache.reuse.travel_misses += 1;
                if cache.travel.len() == 64 {
                    cache.travel.clear();
                }
                cache.travel.insert(key, mask.clone());
            }
            Ok(mask)
        })
    }

    fn movement_traversal(
        &self,
        start: Hex,
        range: i32,
        through: bool,
        actor: Option<Symbol>,
        units: Option<&[Symbol]>,
        moved: &[Symbol],
    ) -> Result<Traversal, String> {
        SEARCH_TRAVERSALS.with(|slot| {
            let mut slot = slot.borrow_mut();
            let cache = slot.as_mut().filter(|cache| self.can_reuse_spatial(cache));
            let Some(cache) = cache else {
                return self.compute_movement_traversal(start, range, through, actor, units, moved);
            };
            let mut passable = if through {
                Vec::new()
            } else {
                self.locations
                    .iter()
                    .filter_map(|&(_, hex)| {
                        self.passable_token(hex, moved)
                            .then(|| self.tile_index(hex))
                            .flatten()
                    })
                    .collect::<Vec<_>>()
            };
            passable.sort_unstable();
            let key = TraversalKey {
                start,
                range,
                through,
                occupancy: (!through).then(|| self.movement_occupancy().clone()),
                passable,
            };
            if let Some(traversal) = cache.values.get(&key) {
                cache.reuse.movement_hits += 1;
                return Ok(traversal.clone());
            }
            cache.reuse.movement_misses += 1;
            let traversal =
                self.compute_movement_traversal(start, range, through, actor, units, moved)?;
            if cache.values.len() == 128 {
                cache.values.clear();
            }
            cache.values.insert(key, traversal.clone());
            Ok(traversal)
        })
    }

    fn compute_movement_traversal(
        &self,
        start: Hex,
        range: i32,
        through: bool,
        actor: Option<Symbol>,
        units: Option<&[Symbol]>,
        moved: &[Symbol],
    ) -> Result<(Vec<bool>, Vec<bool>), String> {
        let mut queued = vec![false; self.tile_hexes().len()];
        let mut terminal = queued.clone();
        let start_index = self.tile_index(start);
        if let Some(index) = start_index {
            queued[index] = true;
        }
        let units = units.unwrap_or(actor.as_slice());
        let mut queue = VecDeque::from([(start, start_index, 0)]);
        while let Some((hex, index, distance)) = queue.pop_front() {
            if distance >= range {
                continue;
            }
            let neighbors = hex.neighbors();
            let indices = index.map_or_else(
                || neighbors.map(|neighbor| self.tile_index(neighbor)),
                |index| self.neighbor_indices(index),
            );
            for (next, index) in neighbors.into_iter().zip(indices) {
                if !self.effect_connected(start, next, units)?
                    || !self.effect_connected(hex, next, units)?
                {
                    continue;
                }
                let Some(index) = index else {
                    continue;
                };
                // Topology depends on the edge, but obstacle and token
                // passability are fixed for this tile within the traversal.
                if std::mem::replace(&mut terminal[index], true) || queued[index] {
                    continue;
                }
                if !through
                    && self.obstacle_for_actor_at(next, index, actor)?
                    && !self.passable_token(next, moved)
                {
                    continue;
                }
                queued[index] = true;
                queue.push_back((next, Some(index), distance + 1));
            }
        }
        Ok((queued, terminal))
    }
}

impl NativeWorld {
    pub(crate) fn movement_action_range(
        &self,
        unit: Symbol,
        mut range: i64,
    ) -> Result<i64, String> {
        if self.position(unit).is_none() {
            return Ok(range);
        }
        for effect in &self.effects.instances {
            let behavior = &effect.definition.behavior;
            if behavior.kind == Kind::MovementZone
                && self.effect_active(effect)
                && self.effect_in_scope(effect, unit, true)?
                && !self.effect_ignored(effect, unit)?
                && let Some(maximum) = behavior.maximum
            {
                range = range.min(maximum);
            }
        }
        Ok(range)
    }
    pub(crate) fn movement_zone_passes(&self, unit: Symbol) -> Result<bool, String> {
        if self.position(unit).is_none() {
            return Ok(false);
        }
        for effect in &self.effects.instances {
            let behavior = &effect.definition.behavior;
            if behavior.kind == Kind::MovementAura
                && behavior.grants_through
                && self.effect_active(effect)
                && self.effect_in_scope(effect, unit, true)?
                && !self.effect_ignored(effect, unit)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(crate) fn tide_of_darkness(&self, actor: Option<Symbol>) -> bool {
        actor.and_then(|id| self.hero(id)).is_some_and(|hero| {
            hero.level >= 8
                && hero
                    .ultimate
                    .and_then(|id| self.card(id))
                    .is_some_and(|card| {
                        self.definitions.cards[card.definition].effect
                            == self.definitions.tide_of_darkness
                    })
        })
    }
    pub(crate) fn action_controller(
        &self,
        player: Symbol,
        actor: Option<Symbol>,
        owner: Option<Symbol>,
    ) -> Option<Symbol> {
        if actor != Some(player) || owner != Some(player) {
            return None;
        }
        let card = self.hero(player)?.current.and_then(|id| self.card(id))?;
        let printed = self.definitions.cards[card.definition].printed_id;
        self.effects
            .instances
            .iter()
            .find(|effect| {
                effect.definition.behavior.kind == Kind::ControlAction
                    && effect.active
                    && effect.scope_origin == Some(player)
                    && effect.definition.behavior.controlled_card == Some(printed)
            })
            .map(|effect| effect.source)
    }
}
