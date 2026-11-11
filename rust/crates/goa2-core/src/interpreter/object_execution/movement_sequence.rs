//! Movement macros allocate the exact source selector/filter/move structure.
use super::passive_bodies::Kind;
use super::phases::Field;
use super::{
    Arc, BoundObjects, Datum, Hex, NativeSession, ObjectId, ObjectKind, ObjectValue, Result, Stat,
    Symbol, unsupported,
};

pub(super) struct SequencePlan {
    actor: Option<Symbol>,
    range: i64,
    through: bool,
    straight: bool,
    full: bool,
    destination: Symbol,
    already_selected: bool,
}
pub(super) struct TowardPlan {
    unit: Symbol,
    destination: Hex,
    range: i64,
}
impl NativeSession {
    pub(super) fn preflight_move_toward_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<TowardPlan>> {
        let actor = self
            .spatial_text_bound(objects, step, Field::UnitId)?
            .or(self
                .planning
                .actor
                .filter(|id| !self.symbols.name(*id).is_empty()));
        let Some(unit) = actor.map(|id| self.world.resolved_actor(id)) else {
            return Ok(None);
        };
        let Some(origin) = self.world.position(unit) else {
            return Ok(None);
        };
        let key = self.passive_text_bound(objects, step, Field::TargetHexKey)?;
        let Some(target) = objects.arena.context_value(key) else {
            return Ok(None);
        };
        if matches!(target, ObjectValue::Atom(Datum::Null)) {
            return Ok(None);
        }
        let (target, _) = self.spatial_hex_bound(objects, &target)?;
        if !origin.straight(target)
            || !self
                .world
                .effect_connected(origin, target, &[unit])
                .map_err(unsupported)?
        {
            return Ok(None);
        }
        let distance = origin.distance(target);
        if distance < 2 {
            return Ok(None);
        }
        let destination = origin
            .line_points(target)
            .and_then(|mut path| path.nth((distance - 2) as usize))
            .ok_or_else(|| unsupported("straight pull destination missing"))?;
        self.phase_generation_check(0, 1)?;
        Ok(Some(TowardPlan {
            unit,
            destination,
            range: i64::from(distance - 1),
        }))
    }

    pub(super) fn move_toward_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: TowardPlan,
    ) -> Result<()> {
        let key = objects.phases.k(Field::TowardDestination);
        self.coordinates_bound(
            objects,
            super::coordinates::CoordinatePlan::Hex(key, plan.destination),
        )?;
        let movement = self.phase_step(objects, "move_unit")?;
        for (field, value) in [
            (Field::UnitId, Datum::Text(plan.unit)),
            (Field::DestinationKey, Datum::Text(key)),
            (Field::Range, Datum::Int(plan.range)),
            (Field::Straight, Datum::Bool(true)),
        ] {
            objects
                .arena
                .set_field(movement, objects.phases.k(field), ObjectValue::Atom(value))
                .map_err(unsupported)?;
        }
        self.push_live_steps_bound(objects, &[movement])
    }

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

pub(super) struct ForcedDefenseMovement {
    defender: Symbol,
    card: Symbol,
    secondary_range: Option<i64>,
    recipes: Vec<super::passive_bodies::Recipe>,
}

impl NativeSession {
    pub(super) fn preflight_force_defense_movement_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<ForcedDefenseMovement>> {
        use crate::{data::Registers, world::ActionKind};

        let key = self.symbols.lookup("defender_key").unwrap();
        let defender_key = self.truthy_symbol_bound(objects, objects.arena.field(step, key))?;
        let defender = match defender_key {
            Some(key) => objects.symbol_from_context(key, &self.symbols)?,
            None => None,
        }
        .or(objects.symbol_from_context(objects.phases.k(Field::DefenderId), &self.symbols)?);
        let card =
            objects.symbol_from_context(objects.phases.k(Field::DefenseCardId), &self.symbols)?;
        let (Some(defender), Some(card)) = (defender, card) else {
            return Ok(None);
        };
        self.sync_card_collections_bound(objects)?;
        let Some(hero) = self.world.hero(defender) else {
            return Ok(None);
        };
        let Some(id) = hero
            .discard
            .iter()
            .copied()
            .chain(hero.played.iter().copied().flatten())
            .find(|id| {
                self.world.card(*id).is_some_and(|item| {
                    self.world.definitions.cards[item.definition].printed_id == card
                })
            })
        else {
            return Ok(None);
        };
        let item = self.world.card(id).unwrap();
        let definition = &self.world.definitions.cards[item.definition];
        if item.facedown
            || !self
                .world
                .action_legality(
                    defender,
                    ActionKind::Movement,
                    Some(id),
                    &Registers::default(),
                )
                .map_err(unsupported)?
                .allowed
        {
            return Ok(None);
        }
        let (secondary_range, recipes) = if definition.primary == Some(ActionKind::Movement) {
            let rules = &objects.scheduler.as_ref().unwrap().primary_bodies;
            let recipes = if rules.registered(definition.effect, definition.spell) {
                let stats = self.world.card_stats(defender, id).map_err(unsupported)?;
                self.prepare_primary_body_bound(objects, defender, id, Some(stats))?
                    .recipes
            } else {
                Vec::new()
            };
            (None, recipes)
        } else if let Some((_, range)) = definition
            .secondary
            .iter()
            .find(|(action, _)| *action == ActionKind::Movement)
        {
            let rules = &objects.scheduler.as_ref().unwrap().action_bodies;
            (Some(*range), rules.recipes("movement").to_vec())
        } else {
            return Ok(None);
        };
        let generated = recipes
            .iter()
            .flat_map(|recipe| &recipe.objects)
            .filter(|(_, kind)| *kind == ObjectKind::Step)
            .count();
        self.phase_generation_check(generated, 3)?;
        Ok(Some(ForcedDefenseMovement {
            defender,
            card,
            secondary_range,
            recipes,
        }))
    }

    pub(super) fn force_defense_movement_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: ForcedDefenseMovement,
    ) -> Result<()> {
        let symbols = Arc::make_mut(&mut self.symbols);
        let saved = symbols.intern("sj_saved_actor");
        let unused = symbols.intern("sj_saved_actor_unused");
        let destination = symbols.intern("sj_forced_dest");
        let actor_id = symbols.intern("actor_id");
        let action = symbols.lookup("MOVEMENT").unwrap();
        objects.arena.symbols = self.symbols.clone();
        objects.push_scope(action, plan.card, plan.defender)?;
        let enter = self.phase_step(objects, "set_actor")?;
        let mut queued = vec![enter];
        let steps = self.allocate_source_recipes_bound(
            objects,
            plan.recipes.iter().map(|recipe| (recipe, plan.defender)),
        )?;
        for step in steps {
            let kind = self.passive_text_bound(objects, step, Field::FilterType)?;
            if self.symbols.name(kind) == "move_sequence" {
                for field in [Field::Straight, Field::FullDistance, Field::Mandatory] {
                    objects
                        .arena
                        .set_field(
                            step,
                            objects.phases.k(field),
                            ObjectValue::Atom(Datum::Bool(true)),
                        )
                        .map_err(unsupported)?;
                }
                if let Some(range) = plan.secondary_range {
                    for (field, value) in [
                        (Field::Range, Datum::Int(range)),
                        (Field::RangeStat, Datum::Null),
                        (Field::DestinationKey, Datum::Text(destination)),
                    ] {
                        objects
                            .arena
                            .set_field(step, objects.phases.k(field), ObjectValue::Atom(value))
                            .map_err(unsupported)?;
                    }
                }
            }
            queued.push(step);
        }
        let rules = objects.scheduler.as_ref().unwrap().action_bodies.clone();
        let restore = self.allocate_source_recipe_bound(
            objects,
            &rules.recipes("restore_other_context")[0],
            &Datum::Text(plan.defender),
        )?;
        let exit = self.phase_step(objects, "set_actor")?;
        for (step, fields) in [
            (
                enter,
                [
                    (actor_id, plan.defender),
                    (objects.phases.k(Field::SaveKey), saved),
                ],
            ),
            (
                exit,
                [
                    (objects.phases.k(Field::ActorKey), saved),
                    (objects.phases.k(Field::SaveKey), unused),
                ],
            ),
        ] {
            for (field, value) in fields {
                objects
                    .arena
                    .set_field(step, field, ObjectValue::Atom(Datum::Text(value)))
                    .map_err(unsupported)?;
            }
        }
        queued.extend([restore, exit]);
        self.push_live_steps_bound(objects, &queued)
    }
}
