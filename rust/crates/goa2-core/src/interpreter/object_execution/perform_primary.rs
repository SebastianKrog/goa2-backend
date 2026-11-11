//! Re-perform the selected card's primary under a shallow, restorable action scope.
use super::passive_bodies::Recipe;
use super::phases::Field;
use super::{
    Arc, BoundObjects, Datum, NativeRequest, NativeSession, ObjectId, ObjectKind, ObjectValue,
    RequestChoices, RequestToken, Result, Symbol, unsupported,
};
use crate::{
    data::Registers, spatial::SpatialContext, targeting::UnitCandidates, world::ActionKind,
};

pub(super) enum PerformPlan {
    Skip,
    Prompt(NativeRequest),
    Run {
        actor: Symbol,
        owner: Symbol,
        body_owner: Symbol,
        card: Symbol,
        action: Symbol,
        recipes: Vec<Recipe>,
        exclude: Option<Symbol>,
        replacement: Option<ActionKind>,
        offboard_clear: bool,
    },
}
impl NativeSession {
    pub(super) fn preflight_perform_primary_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<PerformPlan> {
        let key = self.passive_text_bound(objects, step, Field::CardKey)?;
        let Some(ObjectValue::Atom(Datum::Text(selected))) = objects
            .arena
            .context_value(key)
            .filter(|v| objects.arena.truthy(v))
        else {
            return Ok(PerformPlan::Skip);
        };
        let Some(actor) = self
            .truthy_symbol_bound(
                objects,
                objects.arena.field(step, objects.phases.k(Field::HeroId)),
            )?
            .or(self
                .planning
                .actor
                .filter(|id| !self.symbols.name(*id).is_empty()))
        else {
            return Ok(PerformPlan::Skip);
        };
        self.sync_card_collections_bound(objects)?;
        let Some(hero) = self.world.hero(actor) else {
            return Ok(PerformPlan::Skip);
        };
        let owner = hero.id;
        let card = hero
            .current
            .into_iter()
            .chain(hero.played.iter().copied().flatten())
            .chain(hero.discard.iter().copied())
            .chain(hero.hand.iter().copied())
            .chain(hero.deck.iter().copied())
            .chain(hero.spells.iter().copied())
            .find(|id| {
                self.world.card(*id).is_some_and(|card| {
                    self.world.definitions.cards[card.definition].printed_id == selected
                })
            });
        let Some(card) = card else {
            return Ok(PerformPlan::Skip);
        };
        let instance = self.world.card(card).unwrap();
        let definition = self.world.definitions.cards[instance.definition].clone();
        if instance.facedown
            || definition.effect_absent
            || self.symbols.name(definition.effect).is_empty()
        {
            return Ok(PerformPlan::Skip);
        }
        let action = definition.primary.map(|kind| {
            if kind == ActionKind::DefenseSkill {
                ActionKind::Skill
            } else {
                kind
            }
        });
        if let Some(action) = action
            && !self
                .world
                .action_legality(actor, action, Some(card), &Registers::default())
                .map_err(unsupported)?
                .allowed
        {
            return Ok(PerformPlan::Skip);
        }
        let primary_rules = objects.scheduler.as_ref().unwrap().primary_bodies.clone();
        if !primary_rules.registered(definition.effect, definition.spell) {
            return Ok(PerformPlan::Skip);
        }
        if definition.spell {
            return Err(unsupported("spell re-performance body not yet bound"));
        }
        let stats = self.world.card_stats(actor, card).map_err(unsupported)?;
        let Some(action) = action else {
            return Ok(PerformPlan::Skip);
        };
        let replacement = match action {
            ActionKind::Movement => Some(ActionKind::FastTravel),
            ActionKind::Attack => Some(ActionKind::Clear),
            _ => None,
        };
        let rules = objects.scheduler.as_ref().unwrap().action_bodies.clone();
        let replacement = if let Some(replacement) = replacement {
            if !self
                .world
                .action_legality(actor, replacement, Some(card), &Registers::default())
                .map_err(unsupported)?
                .allowed
            {
                None
            } else if replacement == ActionKind::FastTravel {
                self.world
                    .has_safe_travel_zone(actor)
                    .then_some(replacement)
            } else {
                let inputs = self.unit_inputs_bound(objects, &rules.clear_filters)?;
                let context = SpatialContext {
                    count_observer: Some(&self.count_observer),
                    world: &self.world,
                    registers: &inputs,
                    symbols: &self.symbols,
                    actor: self.planning.actor,
                    frame: None,
                    coordinates: self.coordinates,
                    range_override: None,
                    through_override: None,
                };
                (!self
                    .world
                    .select_units(
                        UnitCandidates::UnitsAndTokens,
                        self.planning.actor,
                        &rules.clear_filters,
                        &context,
                    )
                    .map_err(unsupported)?
                    .is_empty())
                .then_some(replacement)
            }
        } else {
            None
        };
        let chosen = objects
            .selected(step, self.selection_key)
            .and_then(|v| match v {
                ObjectValue::Atom(Datum::Text(id)) => Some(id),
                _ => None,
            });
        let selected_replacement = if let Some(replacement) = replacement {
            let replacement_id = self.symbols.lookup(replacement.name()).unwrap();
            if chosen == Some(replacement_id) {
                Some(replacement)
            } else if chosen == self.symbols.lookup(action.name()) {
                None
            } else {
                if self.next_request == u64::MAX && objects.token(step)?.is_none() {
                    return Err(unsupported("request generation counter exhausted"));
                }
                let text = format!(
                    "Choose an action to perform on {}",
                    self.symbols.name(definition.name)
                );
                let prompt = Arc::make_mut(&mut self.symbols).intern(&text);
                return Ok(PerformPlan::Prompt(NativeRequest {
                    controlled: None,
                    id: RequestToken::Generated(0),
                    owner: actor,
                    prompt,
                    movement_range: None,
                    pre_action_range: None,
                    choices: RequestChoices::PrimaryReplacement {
                        actor,
                        card: definition.printed_id,
                        primary: action,
                        replacement,
                    },
                    can_skip: false,
                    discard: false,
                }));
            }
        } else {
            None
        };
        let offboard_clear = selected_replacement == Some(ActionKind::Clear)
            && !self.world.has_board_presence(actor);
        let (recipes, body_owner) = match selected_replacement {
            Some(ActionKind::FastTravel) => (rules.recipes("travel").to_vec(), actor),
            Some(ActionKind::Clear) => (
                rules
                    .recipes(if offboard_clear { "log" } else { "clear" })
                    .to_vec(),
                actor,
            ),
            _ => {
                let plan =
                    self.prepare_reperformed_primary_body_bound(objects, actor, card, stats)?;
                (plan.recipes, owner)
            }
        };
        let count = recipes
            .iter()
            .flat_map(|recipe| &recipe.objects)
            .filter(|(_, kind)| *kind == ObjectKind::Step)
            .count();
        self.phase_generation_check(
            0,
            u64::try_from(count)
                .map_err(|_| unsupported("re-performance generation count overflow"))?
                .checked_add(1)
                .ok_or_else(|| unsupported("re-performance generation count overflow"))?,
        )?;
        if objects.arena.context_value(objects.action_stack).is_some()
            && objects.mutable_stack(objects.action_stack)?.is_none()
        {
            return Err(unsupported("action context stack must be a mutable list"));
        }
        let exclude = if selected_replacement.is_none() {
            self.passive_optional_text_bound(objects, step, Field::ExcludeTargetKey)?
                .filter(|id| !self.symbols.name(*id).is_empty())
        } else {
            None
        };
        Ok(PerformPlan::Run {
            actor,
            owner,
            body_owner,
            card: definition.printed_id,
            action: self
                .symbols
                .lookup(selected_replacement.unwrap_or(action).name())
                .unwrap(),
            recipes,
            exclude,
            replacement: selected_replacement,
            offboard_clear,
        })
    }
    pub(super) fn perform_primary_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: PerformPlan,
    ) -> Result<Option<NativeRequest>> {
        objects.arena.symbols = self.symbols.clone();
        match plan {
            PerformPlan::Skip => {}
            PerformPlan::Prompt(request) => {
                objects
                    .arena
                    .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                    .map_err(unsupported)?;
                return Ok(Some(request));
            }
            PerformPlan::Run {
                actor,
                owner,
                body_owner,
                card,
                action,
                recipes,
                exclude,
                replacement,
                offboard_clear,
            } => {
                objects.push_scope(action, card, owner)?;
                let mut children = self.allocate_source_recipes_bound(
                    objects,
                    recipes.iter().map(|recipe| (recipe, body_owner)),
                )?;
                for child in children.iter().copied() {
                    if replacement == Some(ActionKind::FastTravel) {
                        objects
                            .arena
                            .set_field(
                                child,
                                objects.phases.k(Field::SourceCard),
                                ObjectValue::Atom(Datum::Text(card)),
                            )
                            .map_err(unsupported)?;
                    }
                    if offboard_clear {
                        let message = format!(
                            "{} attempted clear but is not on board.",
                            self.symbols.name(actor)
                        );
                        let message = Arc::make_mut(&mut self.symbols).intern(&message);
                        objects.arena.symbols = self.symbols.clone();
                        objects
                            .arena
                            .set_field(
                                child,
                                self.symbols.lookup("message").unwrap(),
                                ObjectValue::Atom(Datum::Text(message)),
                            )
                            .map_err(unsupported)?;
                    }
                    if let Some(key) = exclude {
                        Self::inject_perform_exclusion_bound(objects, child, key, 0)?;
                    }
                    Self::validate_returned_step_bound(objects, child)?;
                }
                let rules = objects.scheduler.as_ref().unwrap().action_bodies.clone();
                children.push(self.allocate_source_recipe_bound(
                    objects,
                    &rules.recipes("restore_context")[0],
                    &Datum::Text(owner),
                )?);
                self.push_live_steps_bound(objects, &children)?;
            }
        }
        Ok(None)
    }
    fn inject_perform_exclusion_bound(
        objects: &mut BoundObjects,
        step: ObjectId,
        key: Symbol,
        depth: usize,
    ) -> Result<()> {
        if depth >= 128 {
            return Err(unsupported("primary exclusion recursion exceeded"));
        }
        let symbols = objects.arena.symbols.clone();
        let kind = match objects
            .arena
            .field(step, objects.phases.k(Field::FilterType))
        {
            Some(ObjectValue::Atom(Datum::Text(id))) => symbols.name(id),
            _ => return Err(unsupported("primary exclusion step type missing")),
        };
        let target = match kind {
            "select_step" | "multi_select" => match objects
                .arena
                .field(step, objects.phases.k(Field::TargetType))
            {
                Some(ObjectValue::Atom(Datum::Text(id)))
                    if matches!(symbols.name(id), "UNIT" | "UNIT_OR_TOKEN") =>
                {
                    Some(objects.phases.k(Field::Filters))
                }
                _ => None,
            },
            "attack_sequence" => Some(objects.phases.k(Field::TargetFilters)),
            _ => None,
        };
        if let Some(target) = target {
            let Some(ObjectValue::Object(filters)) = objects.arena.field(step, target) else {
                return Err(unsupported("primary exclusion filter list missing"));
            };
            let recipe = &objects.scheduler.as_ref().unwrap().action_bodies.exclusion;
            let ObjectValue::Object(filter) = objects.arena.allocate(recipe) else {
                unreachable!()
            };
            objects
                .arena
                .classify_recipe_object(filter, &[], ObjectKind::Model)
                .map_err(unsupported)?;
            let keys = objects
                .arena
                .list(vec![ObjectValue::Atom(Datum::Text(key))]);
            objects
                .arena
                .set_field(filter, objects.phases.k(Field::ExcludeKeys), keys)
                .map_err(unsupported)?;
            objects
                .arena
                .append(filters, ObjectValue::Object(filter))
                .map_err(unsupported)?;
        }
        let nested = match kind {
            "may_repeat_once" | "for_each" => Some(objects.templates),
            "create_effect" => symbols.lookup("finishing_steps"),
            _ => None,
        };
        if let Some(nested) = nested
            && let Some(ObjectValue::Object(list)) = objects.arena.field(step, nested)
        {
            for child in objects.steps_in_list(list)? {
                Self::inject_perform_exclusion_bound(objects, child, key, depth + 1)?;
            }
        }
        if kind == "perform_primary_action"
            && !objects
                .arena
                .field(step, objects.phases.k(Field::ExcludeTargetKey))
                .is_some_and(|v| objects.arena.truthy(&v))
        {
            objects
                .arena
                .set_field(
                    step,
                    objects.phases.k(Field::ExcludeTargetKey),
                    ObjectValue::Atom(Datum::Text(key)),
                )
                .map_err(unsupported)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixture_case;
    use super::*;
    fn session(name: &str) -> NativeSession {
        let case = fixture_case(&format!("object-execution-perform-primary-{name}"));
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    fn performance_generation_guard_precedes_scope_and_stack_mutation() {
        let mut worker = session("build-noble_blade-ordinary");
        worker.next_step = u64::MAX;
        let before = worker.export_envelope().unwrap();
        assert!(
            worker
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("step generation counter exhausted")
        );
        let after = worker.export_envelope().unwrap();
        assert_eq!(before.state, after.state);
        assert_eq!(before.object_bindings, after.object_bindings);
    }
    #[test]
    fn replacement_request_guard_precedes_pending_or_scope_mutation() {
        let mut worker = session("replacement-noble_blade-prompt");
        worker.next_request = u64::MAX;
        let before = worker.export_envelope().unwrap();
        assert!(
            worker
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("request generation counter exhausted")
        );
        let after = worker.export_envelope().unwrap();
        assert_eq!(before.state, after.state);
        assert_eq!(before.object_bindings, after.object_bindings);
    }
    #[test]
    fn selected_card_key_is_live_and_worker_changes_do_not_touch_the_root() {
        let root = session("build-noble_blade-ordinary");
        let before = root.export_envelope().unwrap();
        let mut worker = root.clone();
        let absent = Arc::make_mut(&mut worker.symbols).intern("absent-key");
        let step = worker.stack.last().unwrap().object.unwrap();
        let objects = worker.objects.as_mut().unwrap();
        objects
            .arena
            .set_field(
                step,
                objects.phases.k(Field::CardKey),
                ObjectValue::Atom(Datum::Text(absent)),
            )
            .unwrap();
        worker.advance_prefix(1).unwrap();
        assert_eq!(worker.stack.len(), 1);
        let objects = worker.objects.as_ref().unwrap();
        assert!(objects.arena.context_value(objects.action_stack).is_none());
        assert_eq!(root.export_envelope().unwrap().state, before.state);
    }
    #[test]
    fn missing_selected_card_needs_no_generation_capacity() {
        let mut worker = session("edge-missing-key");
        worker.next_step = u64::MAX;
        worker.next_request = u64::MAX;
        assert!(worker.advance_prefix(1).unwrap().request.is_none());
        assert_eq!(worker.stack.len(), 1);
    }
    #[test]
    fn nested_performance_inherits_exclusion_without_overwriting_an_explicit_key() {
        for (name, expected) in [
            ("build-noble_blade-ordinary", "selected_card"),
            ("build-noble_blade-exclude", "blocked_target"),
        ] {
            let mut worker = session(name);
            let key = worker.symbols.lookup("selected_card").unwrap();
            let expected = worker.symbols.lookup(expected).unwrap();
            let step = worker.stack.last().unwrap().object.unwrap();
            let objects = worker.objects.as_mut().unwrap();
            NativeSession::inject_perform_exclusion_bound(objects, step, key, 0).unwrap();
            assert!(
                matches!(objects.arena.field(step, objects.phases.k(Field::ExcludeTargetKey)), Some(ObjectValue::Atom(Datum::Text(actual))) if actual == expected)
            );
        }
    }
}
