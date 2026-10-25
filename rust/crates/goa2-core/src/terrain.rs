//! Stateful obstacles and one-traversal movement queries. No serialized state in rules.
use crate::{
    data::Symbol,
    effect_rules::Kind,
    geometry::{Hex, MovementQuery},
    world::{EntityKind, NativeWorld},
};
use std::collections::VecDeque;

impl NativeWorld {
    /// `actor` is already resolved by the caller (including current-actor fallback).
    pub fn obstacle_for_actor(&self, hex: Hex, actor: Option<Symbol>) -> Result<bool, String> {
        if self.obstacle(hex) {
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
                    && (query.ignore_obstacles || !self.obstacle_for_actor(end, actor)?))
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
                        || !self.obstacle_for_actor(end, actor)?))
            })
            .collect()
    }
    fn movement_traversal(
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
        if let Some(index) = self.tile_index(start) {
            queued[index] = true;
        }
        let actor_units = actor.into_iter().collect::<Vec<_>>();
        let units = units.unwrap_or(&actor_units);
        let mut queue = VecDeque::from([(start, 0)]);
        while let Some((hex, distance)) = queue.pop_front() {
            if distance >= range {
                continue;
            }
            for next in hex.neighbors() {
                if !self.effect_connected(start, next, units)?
                    || !self.effect_connected(hex, next, units)?
                {
                    continue;
                }
                let Some(index) = self.tile_index(next) else {
                    continue;
                };
                terminal[index] = true;
                if queued[index] {
                    continue;
                }
                if !through
                    && self.obstacle_for_actor(next, actor)?
                    && !self.passable_token(next, moved)
                {
                    continue;
                }
                queued[index] = true;
                queue.push_back((next, distance + 1));
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
