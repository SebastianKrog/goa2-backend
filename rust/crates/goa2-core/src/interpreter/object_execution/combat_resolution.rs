//! Live combat values and attack-history queries; mutation stays in the arena.
use super::phases::Field;
use super::*;
use crate::{
    data::RegisterRead,
    spatial::{HexFilter, Origin, SpatialContext},
    targeting::{Relation, UnitCandidates, UnitFilter},
    world::EntityKind,
};

pub(super) struct CombatResolutionPlan {
    last: Option<Datum>,
    blocked: bool,
    event: Option<CombatEvent>,
    defeat: Option<DefeatBinding>,
}
pub(super) struct AdjacentSnapshotPlan {
    output: Symbol,
    ids: Vec<Symbol>,
}
impl NativeSession {
    pub(super) fn preflight_combat_resolution_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<CombatResolutionPlan> {
        let key = self.passive_text_bound(objects, step, Field::TargetKey)?;
        let target = self.truthy_symbol_bound(objects, objects.arena.context_value(key))?;
        if target.is_none_or(|id| self.world.position(id).is_none()) {
            return Ok(CombatResolutionPlan {
                last: None,
                blocked: false,
                event: None,
                defeat: None,
            });
        }
        let damage = self
            .spatial_int_bound(objects, step, Field::Damage)?
            .ok_or_else(|| unsupported("combat damage missing"))?;
        self.sync_card_collections_bound(objects)?;
        let keys = [
            Field::DefenseValue,
            Field::DefenseInvalid,
            Field::AutoBlock,
            Field::IgnoreMinionDefense,
            Field::MinionDefenseModifier,
            Field::DefenseInitiative,
            Field::DefenseBonus,
        ]
        .map(|field| objects.phases.k(field));
        let mut context = self.spatial_inputs_bound(objects, keys.into_iter().chain([key]));
        let (event, defeat) = self
            .combat_rules
            .resolve(
                &self.world,
                &mut context,
                &self.symbols,
                self.planning.actor,
                key,
                damage,
            )
            .map_err(combat_error)?;
        if defeat.is_some() {
            self.phase_generation_check(0, 1)?;
        }
        Ok(CombatResolutionPlan {
            last: context
                .get(objects.phases.k(Field::LastCombatTarget))
                .cloned(),
            blocked: matches!(
                context.get(objects.phases.k(Field::BlockSucceeded)),
                Some(Datum::Bool(true))
            ),
            event,
            defeat,
        })
    }
    pub(super) fn combat_resolution_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: CombatResolutionPlan,
    ) -> Result<Vec<NativeEvent>> {
        if let Some(last) = plan.last {
            objects
                .arena
                .set_context(
                    objects.phases.k(Field::LastCombatTarget),
                    ObjectValue::Atom(last),
                )
                .map_err(unsupported)?;
        }
        objects
            .arena
            .set_context(
                objects.phases.k(Field::BlockSucceeded),
                ObjectValue::Atom(Datum::Bool(plan.blocked)),
            )
            .map_err(unsupported)?;
        if let Some(binding) = plan.defeat {
            let child = self.phase_step(objects, "defeat")?;
            for (field, value) in [
                (Field::Victim, Datum::Text(binding.victim)),
                (
                    Field::KillerId,
                    binding.killer.map_or(Datum::Null, Datum::Text),
                ),
            ] {
                objects
                    .arena
                    .set_field(child, objects.phases.k(field), ObjectValue::Atom(value))
                    .map_err(unsupported)?;
            }
            self.push_live_steps_bound(objects, &[child])?;
        }
        Ok(plan.event.into_iter().map(NativeEvent::Combat).collect())
    }
    pub(super) fn preflight_adjacent_snapshot_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<AdjacentSnapshotPlan> {
        let output = self.passive_text_bound(objects, step, Field::Output)?;
        let key = self.passive_text_bound(objects, step, Field::TargetKey)?;
        let target = self.truthy_symbol_bound(objects, objects.arena.context_value(key))?;
        let Some(target) = target.filter(|id| self.world.resolved_position(*id).is_some()) else {
            return Ok(AdjacentSnapshotPlan {
                output,
                ids: Vec::new(),
            });
        };
        let Some(actor) = self.planning.actor.filter(|id| {
            self.world
                .entity(*id)
                .is_some_and(|entity| entity.has_team && entity.team.is_some())
        }) else {
            return Ok(AdjacentSnapshotPlan {
                output,
                ids: Vec::new(),
            });
        };
        self.sync_card_collections_bound(objects)?;
        let relation = self.passive_text_bound(objects, step, Field::Relation)?;
        let mut filters = vec![UnitFilter::HasTeam, UnitFilter::Kind(EntityKind::Hero)];
        match self.symbols.name(relation) {
            "ENEMY" => filters.push(UnitFilter::Team(Relation::Enemy)),
            "FRIENDLY" => filters.push(UnitFilter::Team(Relation::Friendly)),
            _ => {}
        }
        filters.push(UnitFilter::Range(HexFilter::Range {
            origin: Origin {
                literal: Some(target),
                key: None,
                frame: false,
            },
            hex_key: None,
            min: Some(1),
            max: Some(1),
            min_key: None,
            max_key: None,
            frame_range: false,
            geometric: false,
        }));
        filters.push(UnitFilter::Immunity);
        let inputs = self.unit_inputs_bound(objects, &filters)?;
        let context = SpatialContext {
            count_observer: Some(&self.count_observer),
            world: &self.world,
            registers: &inputs,
            symbols: &self.symbols,
            actor: Some(actor),
            frame: None,
            coordinates: self.coordinates,
            range_override: None,
            through_override: None,
        };
        // Snapshot does not use SelectStep's targeting validator: only the
        // source's explicit relation, adjacency and immunity predicates apply.
        let ids = self
            .world
            .select_units(UnitCandidates::Units, None, &filters, &context)
            .map_err(unsupported)?;
        Ok(AdjacentSnapshotPlan { output, ids })
    }
    pub(super) fn adjacent_snapshot_bound(
        &self,
        objects: &mut BoundObjects,
        plan: AdjacentSnapshotPlan,
    ) -> Result<()> {
        let values = objects.arena.list(
            plan.ids
                .into_iter()
                .map(|id| ObjectValue::Atom(Datum::Text(id)))
                .collect(),
        );
        objects
            .arena
            .set_context(plan.output, values)
            .map_err(unsupported)
    }
    fn attack_scalar_text_bound(&self, value: &ObjectValue) -> Result<String> {
        match value {
            ObjectValue::Atom(value) => value
                .python_scalar_text(&self.symbols)
                .ok_or_else(|| unsupported("attack identity coercion not bound")),
            _ => Err(unsupported("aggregate attack identity coercion not bound")),
        }
    }
    pub(super) fn preflight_target_defeated_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<(Symbol, bool)> {
        let key = self.passive_text_bound(objects, step, Field::TargetKey)?;
        let output = self.passive_text_bound(objects, step, Field::Output)?;
        let by_actor = self.spatial_bool_bound(objects, step, Field::ByActor)?;
        let Some(target) = objects
            .arena
            .context_value(key)
            .filter(|v| objects.arena.truthy(v))
        else {
            return Ok((output, false));
        };
        let mark = match objects
            .arena
            .context_value(objects.phases.k(Field::DefeatMarks))
        {
            None => return Ok((output, false)),
            Some(ObjectValue::Object(id)) if objects.arena.kind(id) == ObjectKind::Dict => {
                objects.arena.field(id, key)
            }
            _ => return Err(unsupported("attack defeat marks require a dictionary")),
        };
        let mark = match mark {
            None | Some(ObjectValue::Atom(Datum::Null)) => return Ok((output, false)),
            Some(ObjectValue::Atom(Datum::Int(value))) => value,
            Some(ObjectValue::Atom(Datum::Bool(value))) => i64::from(value),
            _ => return Err(unsupported("attack defeat mark requires an integer")),
        };
        let Some(log) = objects
            .arena
            .context_value(objects.phases.k(Field::DefeatLog))
        else {
            return Ok((output, false));
        };
        let ObjectValue::Object(log) = log else {
            return Err(unsupported("attack defeat log requires a sequence"));
        };
        let log = objects
            .arena
            .sequence(log)
            .ok_or_else(|| unsupported("attack defeat log requires a sequence"))?;
        let start = if mark < 0 {
            log.len().saturating_sub(mark.unsigned_abs() as usize)
        } else {
            usize::try_from(mark).unwrap_or(usize::MAX).min(log.len())
        };
        if start >= log.len() {
            return Ok((output, false));
        }
        let target = self.attack_scalar_text_bound(&target)?;
        let actor = self
            .planning
            .actor
            .and_then(|id| self.world.hero(id).map(|hero| hero.id).or(Some(id)))
            .map_or("", |id| self.symbols.name(id));
        for row in &log[start..] {
            let ObjectValue::Object(row) = row else {
                return Err(unsupported("attack defeat row requires a pair"));
            };
            let row = objects
                .arena
                .sequence(*row)
                .filter(|row| row.len() == 2)
                .ok_or_else(|| unsupported("attack defeat row requires a pair"))?;
            if !matches!(&row[0], ObjectValue::Atom(Datum::Text(id)) if self.symbols.name(*id) == target)
            {
                continue;
            }
            if !by_actor {
                return Ok((output, true));
            }
            if !objects.arena.truthy(&row[1]) {
                continue;
            }
            let killer = self.attack_scalar_text_bound(&row[1])?;
            let owner = self
                .symbols
                .lookup(&killer)
                .and_then(|id| self.world.hero(id))
                .map(|hero| self.symbols.name(hero.id))
                .unwrap_or(&killer);
            if owner == actor {
                return Ok((output, true));
            }
        }
        Ok((output, false))
    }
}
