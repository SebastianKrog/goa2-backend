//! Snapshot-based movement for Tali's directional shifts.
use super::phases::Field;
use super::{
    BoundObjects, Datum, NativeEvent, NativeSession, ObjectId, ObjectValue, Result, unsupported,
};
use crate::{spatial::SpatialContext, validation::DisplacementKind, world::EntityKind};

impl NativeSession {
    pub(super) fn directional_move_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        events: &mut Vec<NativeEvent>,
    ) -> Result<()> {
        let key = self
            .symbols
            .lookup("direction_key")
            .ok_or_else(|| unsupported("missing direction field"))?;
        let Some(ObjectValue::Atom(Datum::Text(key))) = objects.arena.field(step, key) else {
            return Err(unsupported("missing direction key"));
        };
        let direction = match objects.arena.context_value(key) {
            Some(ObjectValue::Atom(Datum::Int(value))) => value,
            Some(ObjectValue::Atom(Datum::Bool(value))) => i64::from(value),
            Some(ObjectValue::Atom(Datum::Text(value))) => self
                .symbols
                .name(value)
                .trim()
                .parse()
                .map_err(|_| unsupported("invalid shift direction"))?,
            None | Some(ObjectValue::Atom(Datum::Null)) => return Ok(()),
            _ => return Err(unsupported("invalid shift direction")),
        };
        let radius = match self
            .symbols
            .lookup("radius")
            .and_then(|key| objects.arena.field(step, key))
        {
            Some(ObjectValue::Atom(Datum::Int(value))) => value,
            _ => return Err(unsupported("shift radius missing")),
        };
        let origin = self
            .passive_optional_text_bound(objects, step, Field::OriginId)?
            .filter(|id| !self.symbols.name(*id).is_empty())
            .or(self.planning.actor);
        let Some((origin, from, team)) = origin
            .and_then(|id| Some((id, self.world.position(id)?, self.world.entity(id)?.team?)))
        else {
            return Ok(());
        };
        let locations = objects.phases.root(&objects.arena, Field::Locations)?;
        let ordered = objects
            .arena
            .fields(locations)
            .ok_or_else(|| unsupported("shift locations missing"))?;
        let inputs = self.spatial_inputs_bound(objects, []);
        let context = SpatialContext {
            count_observer: None,
            world: &self.world,
            registers: &inputs,
            symbols: &self.symbols,
            actor: self.planning.actor,
            frame: None,
            coordinates: self.coordinates,
            range_override: None,
            through_override: None,
        };
        let mut moves = Vec::new();
        for (unit, _) in ordered {
            let Some(entity) = self.world.entity(*unit) else {
                continue;
            };
            let Some(start) = self.world.position(*unit) else {
                continue;
            };
            if !matches!(
                entity.kind,
                EntityKind::Hero | EntityKind::Piece | EntityKind::Minion
            ) || entity.team == Some(team)
                || i64::from(from.distance(start)) > radius
                || !self
                    .world
                    .effect_connected(from, start, &[origin, *unit])
                    .map_err(unsupported)?
                || !self
                    .world
                    .immune_filter(*unit, &context)
                    .map_err(unsupported)?
                || !self
                    .world
                    .can_target(Some(origin), Some(*unit))
                    .map_err(unsupported)?
                    .allowed
                || !self
                    .world
                    .can_displace(*unit, Some(origin), DisplacementKind::Move, None)
                    .map_err(unsupported)?
                    .allowed
                || !self
                    .world
                    .movement_legality(*unit, 1, false, None, &self.context)
                    .map_err(unsupported)?
                    .allowed
            {
                continue;
            }
            let to = start.neighbors()[direction.rem_euclid(6) as usize];
            if !self.world.on_map(to)
                || !self
                    .world
                    .effect_connected(start, to, &[*unit])
                    .map_err(unsupported)?
                || self.world.terrain(to)
                || (self.world.occupant(to).is_none()
                    && self
                        .world
                        .obstacle_for_actor(to, Some(origin))
                        .map_err(unsupported)?)
            {
                continue;
            }
            moves.push((*unit, start, to));
        }
        // A blocked unit stays in place and can block the rest of its chain.
        loop {
            let candidates: std::collections::HashSet<_> =
                moves.iter().map(|(id, _, _)| *id).collect();
            let previous = moves.len();
            moves.retain(|(_, _, to)| {
                self.world
                    .occupant(*to)
                    .is_none_or(|id| candidates.contains(&id))
            });
            if moves.len() == previous {
                break;
            }
        }
        let placements = moves
            .iter()
            .map(|(unit, _, to)| self.prepare_spatial_bound(objects, *unit, *to, None))
            .collect::<Result<Vec<_>>>()?;
        for (unit, _, _) in &moves {
            self.remove_entity_spatial_bound(objects, *unit)?;
        }
        for ((unit, from, to), placement) in moves.into_iter().zip(placements) {
            self.apply_spatial_bound(objects, &placement)?;
            events.push(NativeEvent::DirectionalMove {
                unit,
                from,
                to,
                direction,
            });
        }
        Ok(())
    }
}
