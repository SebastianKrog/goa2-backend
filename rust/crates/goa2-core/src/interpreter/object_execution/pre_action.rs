//! Live pre-action movement and reentrant disruptor processing.
use super::phases::Field;
use super::*;
use crate::{effect_rules::Kind, objects::ObjectSlot};

pub(super) struct PreMovementPlan {
    hero: Symbol,
    range: i64,
}
pub(super) struct PreDiscardPlan {
    processed: Option<(ObjectId, Vec<Symbol>)>,
    action: PreDiscardAction,
}
enum PreDiscardAction {
    Done,
    Abort,
    Discard { hero: Symbol, effect: Symbol },
    Defeat { hero: Symbol, killer: Symbol },
}
impl NativeSession {
    fn pre_action_hero_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
        coerce: bool,
    ) -> Result<Option<Symbol>> {
        let mut value = objects
            .arena
            .field(step, objects.phases.k(Field::HeroId))
            .filter(|value| objects.arena.truthy(value));
        if value.is_none()
            && let Some(key) = self.spatial_text_bound(objects, step, Field::HeroKey)?
        {
            value = objects
                .arena
                .context_value(key)
                .filter(|value| objects.arena.truthy(value));
        }
        match value {
            None => Ok(None),
            Some(ObjectValue::Atom(Datum::Text(hero))) => Ok(Some(hero)),
            // Movement compares the raw operand with textual source IDs; no
            // non-string value can match a validated effect's source_id.
            Some(_) if !coerce => Ok(None),
            Some(ObjectValue::Atom(value)) => {
                let text = value
                    .python_scalar_text(&self.symbols)
                    .ok_or_else(|| unsupported("pre-action hero coercion not bound"))?;
                Ok(Some(Arc::make_mut(&mut self.symbols).intern(&text)))
            }
            _ => Err(unsupported("aggregate pre-action hero coercion not bound")),
        }
    }
    pub(super) fn preflight_pre_movement_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<PreMovementPlan>> {
        let Some(hero) = self.pre_action_hero_bound(objects, step, false)? else {
            return Ok(None);
        };
        self.sync_card_collections_bound(objects)?;
        let Some(range) = self.world.pre_action_movement_range(hero) else {
            return Ok(None);
        };
        i32::try_from(range).map_err(|_| unsupported("pre-action movement range outside i32"))?;
        self.phase_generation_check(0, 2)?;
        Ok(Some(PreMovementPlan { hero, range }))
    }
    pub(super) fn pre_movement_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<PreMovementPlan>,
    ) -> Result<()> {
        let Some(plan) = plan else {
            return Ok(());
        };
        let prompt = format!(
            "Pre-action movement (up to {} space{})",
            plan.range,
            if plan.range > 1 { "s" } else { "" }
        );
        let prompt = Arc::make_mut(&mut self.symbols).intern(&prompt);
        objects.arena.symbols = self.symbols.clone();
        let select = self.phase_step(objects, "pre_action_select")?;
        for (field, value) in [
            (
                Field::Output,
                Datum::Text(objects.phases.k(Field::PreActionMoveHex)),
            ),
            (Field::Prompt, Datum::Text(prompt)),
        ] {
            objects
                .arena
                .set_field(select, objects.phases.k(field), ObjectValue::Atom(value))
                .map_err(unsupported)?;
        }
        let root = ObjectSlot::Field(objects.phases.k(Field::Filters));
        let obstacle = objects
            .arena
            .classify_recipe_object(
                select,
                &[root.clone(), ObjectSlot::Index(0)],
                ObjectKind::Model,
            )
            .map_err(unsupported)?;
        let path = objects
            .arena
            .classify_recipe_object(select, &[root, ObjectSlot::Index(1)], ObjectKind::Model)
            .map_err(unsupported)?;
        for (model, field, value) in [
            (obstacle, Field::ExcludeId, Datum::Text(plan.hero)),
            (path, Field::UnitId, Datum::Text(plan.hero)),
            (path, Field::Range, Datum::Int(plan.range)),
        ] {
            objects
                .arena
                .set_field(model, objects.phases.k(field), ObjectValue::Atom(value))
                .map_err(unsupported)?;
        }
        let movement = self.phase_step(objects, "move_unit")?;
        for (field, value) in [
            (Field::UnitId, Datum::Text(plan.hero)),
            (
                Field::DestinationKey,
                Datum::Text(objects.phases.k(Field::PreActionMoveHex)),
            ),
            (Field::Range, Datum::Int(plan.range)),
            (Field::Mandatory, Datum::Bool(false)),
            (Field::MoveAction, Datum::Bool(false)),
        ] {
            objects
                .arena
                .set_field(movement, objects.phases.k(field), ObjectValue::Atom(value))
                .map_err(unsupported)?;
        }
        self.push_live_steps_bound(objects, &[select, movement])
    }
    pub(super) fn preflight_pre_discard_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<PreDiscardPlan> {
        let done = || PreDiscardPlan {
            processed: None,
            action: PreDiscardAction::Done,
        };
        let Some(hero) = self.pre_action_hero_bound(objects, step, true)? else {
            return Ok(done());
        };
        self.sync_card_collections_bound(objects)?;
        let Some(owner) = self.world.hero(hero) else {
            return Ok(done());
        };
        if !self.world.has_board_presence(hero) {
            let abort = self.spatial_bool_bound(objects, step, Field::AbortOnDefeat)?;
            return Ok(PreDiscardPlan {
                processed: None,
                action: if abort {
                    PreDiscardAction::Abort
                } else {
                    PreDiscardAction::Done
                },
            });
        }
        let hand = !owner.hand.is_empty();
        let mut processed = None;
        let mut ids = Vec::new();
        let mut action = PreDiscardAction::Done;
        for effect in &self.world.effects.instances {
            if effect.definition.behavior.kind != Kind::PreDiscard {
                continue;
            }
            let list = match processed {
                Some(list) => list,
                None => {
                    let Some(ObjectValue::Object(list)) = objects
                        .arena
                        .field(step, objects.phases.k(Field::ProcessedEffects))
                    else {
                        return Err(unsupported("processed effect IDs require a sequence"));
                    };
                    objects
                        .arena
                        .sequence(list)
                        .ok_or_else(|| unsupported("processed effect IDs require a sequence"))?;
                    processed = Some(list);
                    list
                }
            };
            if ids.contains(&effect.id)
                || objects.arena.sequence(list).unwrap().iter().any(
                    |value| matches!(value,ObjectValue::Atom(Datum::Text(id)) if *id==effect.id),
                )
            {
                continue;
            }
            if !self
                .world
                .pre_action_discard_applies(effect, hero)
                .map_err(unsupported)?
            {
                continue;
            }
            ids.push(effect.id);
            if hand {
                action = PreDiscardAction::Discard {
                    hero,
                    effect: effect.id,
                };
                break;
            }
            if effect.definition.behavior.discard_or_defeat {
                action = PreDiscardAction::Defeat {
                    hero,
                    killer: effect.source,
                };
                break;
            }
        }
        if let Some(list) = processed
            && !ids.is_empty()
        {
            if objects.protected(list) {
                return Err(unsupported(
                    "processed IDs alias an unbound world/step parameter",
                ));
            }
            objects
                .arena
                .check_append(list, &ObjectValue::Atom(Datum::Text(ids[0])))
                .map_err(unsupported)?;
        }
        if matches!(
            action,
            PreDiscardAction::Discard { .. } | PreDiscardAction::Defeat { .. }
        ) {
            self.phase_generation_check(0, 1)?;
        }
        Ok(PreDiscardPlan {
            processed: processed.map(|list| (list, ids)),
            action,
        })
    }
    pub(super) fn pre_discard_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: PreDiscardPlan,
    ) -> Result<()> {
        objects.arena.symbols = self.symbols.clone();
        if let Some((list, ids)) = plan.processed {
            for id in ids {
                objects
                    .arena
                    .append(list, ObjectValue::Atom(Datum::Text(id)))
                    .map_err(unsupported)?;
            }
        }
        match plan.action {
            PreDiscardAction::Done => {}
            PreDiscardAction::Abort => self.abort_objects(objects)?,
            PreDiscardAction::Discard { hero, effect } => {
                self.remove_effects_bound(objects, &[effect])?;
                objects.write(
                    objects.phases.k(Field::PreActionDiscardVictim),
                    Datum::Text(hero),
                )?;
                let child = self.phase_step(objects, "force_discard")?;
                objects
                    .arena
                    .set_field(
                        child,
                        objects.phases.k(Field::VictimKey),
                        ObjectValue::Atom(Datum::Text(
                            objects.phases.k(Field::PreActionDiscardVictim),
                        )),
                    )
                    .map_err(unsupported)?;
                self.push_live_steps_bound(objects, &[child, step])?;
            }
            PreDiscardAction::Defeat { hero, killer } => {
                let child = self.phase_step(objects, "defeat")?;
                for (field, value) in [
                    (Field::Victim, Datum::Text(hero)),
                    (Field::KillerId, Datum::Text(killer)),
                ] {
                    objects
                        .arena
                        .set_field(child, objects.phases.k(field), ObjectValue::Atom(value))
                        .map_err(unsupported)?;
                }
                self.push_live_steps_bound(objects, &[child, step])?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pre_action_generation_guards_preserve_processed_lists_and_stack() {
        for name in [
            "object-execution-pre-action-card-mastery-first",
            "object-execution-live-targeting-pre-discard-plain-first",
        ] {
            let fixture = super::super::fixture_case(name);
            let mut session = NativeSession::compile(
                serde_json::from_value(fixture["envelope"].clone()).unwrap(),
            )
            .unwrap();
            session.next_step = u64::MAX - 1;
            let before = session.export_envelope().unwrap();
            assert!(
                matches!(session.advance_prefix(1),Err(EngineError::Unsupported(message)) if message.contains("generation counter exhausted"))
            );
            let after = session.export_envelope().unwrap();
            assert_eq!(after.state, before.state);
            assert_eq!(after.object_bindings, before.object_bindings);
        }
    }
}
