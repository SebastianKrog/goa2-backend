//! Movement macros allocate the exact source selector/filter/move structure.
use super::passive_bodies::Kind;
use super::phases::Field;
use super::*;

pub(super) struct SequencePlan {
    actor: Option<Symbol>,
    range: i64,
    through: bool,
    straight: bool,
    full: bool,
    destination: Symbol,
    already_selected: bool,
}
impl NativeSession {
    pub(super) fn preflight_move_sequence_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<SequencePlan> {
        self.sync_card_collections_bound(objects)?;
        let actor = self
            .truthy_symbol_bound(
                objects,
                objects.arena.field(step, objects.phases.k(Field::UnitId)),
            )?
            .or(self
                .planning
                .actor
                .filter(|id| !self.symbols.name(*id).is_empty()))
            .map(|id| self.world.resolved_actor(id));
        if actor.is_some() {
            self.world.ensure_move_closure(true).map_err(unsupported)?;
        }
        let range = match objects.arena.field(step, objects.phases.k(Field::Range)) {
            Some(ObjectValue::Atom(Datum::Int(range))) => range,
            _ => return Err(unsupported("movement sequence range missing")),
        };
        let mut range = if let (Some(actor), Some(stat)) = (
            actor,
            self.passive_optional_text_bound(objects, step, Field::RangeStat)?,
        ) {
            let stat = Stat::parse(self.symbols.name(stat)).map_err(unsupported)?;
            let inputs = self.spatial_inputs_bound(
                objects,
                [
                    objects.phases.k(Field::PerformingCard),
                    objects.phases.k(Field::PerformingOwner),
                ],
            );
            self.world
                .computed_stat_in_context(actor, stat, range, &inputs)
                .map_err(unsupported)?
        } else {
            range
        };
        if let Some(actor) = actor {
            range = self
                .world
                .movement_action_range(actor, range)
                .map_err(unsupported)?;
        }
        let mut through = self.spatial_bool_bound(objects, step, Field::Through)?;
        if !through && let Some(actor) = actor {
            through = self.world.movement_aura_passes(actor)
                || self
                    .world
                    .movement_zone_passes(actor)
                    .map_err(unsupported)?;
        }
        let straight = self.spatial_bool_bound(objects, step, Field::Straight)?;
        let full = self.spatial_bool_bound(objects, step, Field::FullDistance)?;
        let destination = self.passive_text_bound(objects, step, Field::DestinationKey)?;
        let already_selected = objects
            .arena
            .context_value(destination)
            .is_some_and(|v| objects.arena.truthy(&v));
        self.phase_generation_check(0, if already_selected { 1 } else { 2 })?;
        Ok(SequencePlan {
            actor,
            range,
            through,
            straight,
            full,
            destination,
            already_selected,
        })
    }
    pub(super) fn move_sequence_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: SequencePlan,
        mandatory: bool,
    ) -> Result<()> {
        let kind = match (plan.straight, plan.full) {
            (false, false) => Kind::MovePlain,
            (true, false) => Kind::MoveStraight,
            (false, true) => Kind::MoveFull,
            (true, true) => Kind::MoveStraightFull,
        };
        let program = objects.scheduler.as_ref().unwrap().clone();
        let recipes = program.passive_bodies.recipe(kind)?;
        let owner = plan.actor.map_or(Datum::Null, Datum::Text);
        let mut queued = Vec::new();
        if !plan.already_selected {
            let selector = self.allocate_source_recipe_bound(objects, &recipes[0], &owner)?;
            let prompt = format!("Select Movement Destination (Range {})", plan.range);
            let prompt = Arc::make_mut(&mut self.symbols).intern(&prompt);
            objects.arena.symbols = self.symbols.clone();
            for (field, value) in [
                (Field::Prompt, Datum::Text(prompt)),
                (Field::Output, Datum::Text(plan.destination)),
            ] {
                objects
                    .arena
                    .set_field(selector, objects.phases.k(field), ObjectValue::Atom(value))
                    .map_err(unsupported)?;
            }
            let Some(ObjectValue::Object(filters)) = objects
                .arena
                .field(selector, objects.phases.k(Field::Filters))
            else {
                return Err(unsupported("movement recipe filters missing"));
            };
            let filters = objects.arena.sequence(filters).unwrap().to_vec();
            for filter in filters {
                let ObjectValue::Object(filter) = filter else {
                    return Err(unsupported("movement filter model missing"));
                };
                let Some(ObjectValue::Atom(Datum::Text(kind))) = objects
                    .arena
                    .field(filter, objects.phases.k(Field::FilterType))
                else {
                    return Err(unsupported("movement filter type missing"));
                };
                match self.symbols.name(kind) {
                    "movement_path_filter" => {
                        objects
                            .arena
                            .set_field(
                                filter,
                                objects.phases.k(Field::Range),
                                ObjectValue::Atom(Datum::Int(plan.range)),
                            )
                            .map_err(unsupported)?;
                        objects
                            .arena
                            .set_field(
                                filter,
                                objects.phases.k(Field::Through),
                                ObjectValue::Atom(Datum::Bool(plan.through)),
                            )
                            .map_err(unsupported)?;
                    }
                    "straight_line_path_filter" => objects
                        .arena
                        .set_field(
                            filter,
                            objects.phases.k(Field::Through),
                            ObjectValue::Atom(Datum::Bool(plan.through)),
                        )
                        .map_err(unsupported)?,
                    "range_filter" => {
                        for field in [Field::MinRange, Field::MaxRange] {
                            objects
                                .arena
                                .set_field(
                                    filter,
                                    objects.phases.k(field),
                                    ObjectValue::Atom(Datum::Int(plan.range)),
                                )
                                .map_err(unsupported)?;
                        }
                    }
                    _ => {}
                }
            }
            queued.push(selector);
        }
        let movement = self.allocate_source_recipe_bound(objects, &recipes[1], &owner)?;
        for (field, value) in [
            (Field::DestinationKey, Datum::Text(plan.destination)),
            (Field::Range, Datum::Int(plan.range)),
            (Field::Through, Datum::Bool(plan.through)),
        ] {
            objects
                .arena
                .set_field(movement, objects.phases.k(field), ObjectValue::Atom(value))
                .map_err(unsupported)?;
        }
        queued.push(movement);
        for step in &queued {
            objects
                .arena
                .set_field(
                    *step,
                    objects.phases.k(Field::Mandatory),
                    ObjectValue::Atom(Datum::Bool(mandatory)),
                )
                .map_err(unsupported)?;
        }
        self.push_live_steps_bound(objects, &queued)
    }
}
