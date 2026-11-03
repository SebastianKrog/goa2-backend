//! Copy a physical card action while preserving both heroes' card collections.
use super::passive_bodies::Recipe;
use super::phases::Field;
use super::*;
use crate::data::Registers;

pub(super) enum CopiedPlan {
    Skip,
    Prompt(NativeRequest),
    Run {
        actor: Symbol,
        owner: Symbol,
        card: Symbol,
        action: Symbol,
        substitution: Option<(Option<Symbol>, bool)>,
        recipes: Vec<(Recipe, Symbol)>,
    },
}
impl NativeSession {
    pub(super) fn preflight_perform_card_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<CopiedPlan> {
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
            return Ok(CopiedPlan::Skip);
        };
        self.sync_card_collections_bound(objects)?;
        let Some(performer) = self.world.hero(actor) else {
            return Ok(CopiedPlan::Skip);
        };
        let mut owner = performer.id;
        if let Some(key) = self
            .passive_optional_text_bound(objects, step, Field::CardOwnerKey)?
            .filter(|id| !self.symbols.name(*id).is_empty())
            && let Some(id) = self.truthy_symbol_bound(objects, objects.arena.context_value(key))?
            && let Some(hero) = self.world.hero(id)
        {
            owner = hero.id;
        }
        let hero = self.world.hero(owner).unwrap();
        let flag = |field| {
            objects
                .arena
                .field(step, objects.phases.k(field))
                .is_some_and(|value| objects.arena.truthy(&value))
        };
        let card = if flag(Field::PreviousSlot) {
            self.world
                .combat
                .turn
                .checked_sub(2)
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| hero.played.get(index).copied().flatten())
        } else {
            let key = self.passive_text_bound(objects, step, Field::CardKey)?;
            let Some(ObjectValue::Atom(Datum::Text(selected))) = objects
                .arena
                .context_value(key)
                .filter(|value| objects.arena.truthy(value))
            else {
                return Ok(CopiedPlan::Skip);
            };
            hero.current
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
                })
        };
        let Some(card) = card else {
            return Ok(CopiedPlan::Skip);
        };
        let printed =
            self.world.definitions.cards[self.world.card(card).unwrap().definition].printed_id;
        let selected = objects
            .selected(step, self.selection_key)
            .map(|value| {
                objects
                    .arena
                    .typed_value(&value, Arc::make_mut(&mut self.symbols))
                    .map_err(unsupported)
            })
            .transpose()?;
        let mut context = Registers::default();
        for (key, value) in self
            .spatial_inputs_bound(
                objects,
                [
                    objects.phases.k(Field::PerformingCard),
                    objects.phases.k(Field::PerformingOwner),
                ],
            )
            .0
        {
            context.set(key, value);
        }
        let resolution = self
            .action_rules
            .resolve_copied(
                &self.world,
                actor,
                card,
                selected.as_ref(),
                &mut context,
                (owner, flag(Field::SuppressAfterResolve)),
            )
            .map_err(unsupported)?;
        let children = match resolution {
            ActionResolution::Prompt(menu) => {
                if self.next_request == u64::MAX && objects.token(step)?.is_none() {
                    return Err(unsupported("request generation counter exhausted"));
                }
                return Ok(CopiedPlan::Prompt(NativeRequest {
                    controlled: None,
                    id: RequestToken::Generated(0),
                    owner: actor,
                    prompt: menu.name,
                    movement_range: None,
                    pre_action_range: None,
                    choices: RequestChoices::Action(menu),
                    can_skip: false,
                    discard: false,
                }));
            }
            ActionResolution::Finished(children) => children,
        };
        let action = match context.get(objects.action_keys[0]) {
            Some(Datum::Text(action)) => *action,
            _ => return Err(unsupported("copied action kind missing")),
        };
        let token = self
            .passive_optional_text_bound(objects, step, Field::TokenOverride)?
            .filter(|id| !self.symbols.name(*id).is_empty());
        let skip_markers = flag(Field::SkipMarkers);
        let substitution = (token.is_some() || skip_markers).then_some((token, skip_markers));
        let mut recipes = Vec::new();
        for child in children {
            if matches!(child, ActionStep::CardText { .. }) {
                let body = self.prepare_copied_body_bound(objects, actor, card)?;
                let owner = body
                    .owner
                    .ok_or_else(|| unsupported("copied body owner missing"))?;
                recipes.extend(body.recipes.into_iter().map(|recipe| (recipe, owner)));
            } else {
                recipes.extend(
                    self.prepare_action_recipes_bound(objects, actor, vec![child])?
                        .into_iter()
                        .map(|recipe| (recipe, actor)),
                );
            }
        }
        let rules = objects.scheduler.as_ref().unwrap().action_bodies.clone();
        recipes.extend(
            rules
                .recipes("restore_context")
                .iter()
                .cloned()
                .map(|recipe| (recipe, actor)),
        );
        let count = recipes
            .iter()
            .flat_map(|(recipe, _)| &recipe.objects)
            .filter(|(_, kind)| *kind == ObjectKind::Step)
            .count();
        self.phase_generation_check(
            0,
            u64::try_from(count).map_err(|_| unsupported("copied action generation overflow"))?,
        )?;
        if objects.arena.context_value(objects.action_stack).is_some()
            && objects.mutable_stack(objects.action_stack)?.is_none()
        {
            return Err(unsupported("action context stack must be a mutable list"));
        }
        Ok(CopiedPlan::Run {
            actor,
            owner,
            card: printed,
            action,
            substitution,
            recipes,
        })
    }
    pub(super) fn perform_card_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: CopiedPlan,
    ) -> Result<Option<NativeRequest>> {
        objects.arena.symbols = self.symbols.clone();
        match plan {
            CopiedPlan::Skip => {}
            CopiedPlan::Prompt(request) => {
                objects
                    .arena
                    .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                    .map_err(unsupported)?;
                return Ok(Some(request));
            }
            CopiedPlan::Run {
                actor,
                owner,
                card,
                action,
                substitution,
                recipes,
            } => {
                objects.push_scope(action, card, owner)?;
                if let Some((token, skip_markers)) = substitution {
                    let owner = self.world.hero(actor).unwrap().id;
                    objects.write(objects.action_keys[7], Datum::Text(owner))?;
                    objects.write(
                        objects.action_keys[5],
                        token.map_or(Datum::Null, Datum::Text),
                    )?;
                    objects.write(objects.action_keys[6], Datum::Bool(skip_markers))?;
                }
                let mut children = Vec::with_capacity(recipes.len());
                for (recipe, owner) in &recipes {
                    let child =
                        self.allocate_source_recipe_bound(objects, recipe, &Datum::Text(*owner))?;
                    Self::validate_returned_step_bound(objects, child)?;
                    children.push(child);
                }
                self.push_live_steps_bound(objects, &children)?;
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(name: &str) -> NativeSession {
        let case = fixture_case(&format!("object-execution-perform-card-edge-{name}"));
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    fn copied_generation_guard_precedes_context_stack_and_scope_mutation() {
        let mut worker = session("substitution");
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
    fn copied_request_guard_precedes_clearing_invalid_input() {
        let mut worker = session("invalid");
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
    fn missing_copied_card_needs_no_generation_capacity() {
        let mut worker = session("missing-card");
        worker.next_step = u64::MAX;
        worker.next_request = u64::MAX;
        worker.advance_prefix(1).unwrap();
        assert_eq!(worker.stack.len(), 1);
    }
    #[test]
    fn copied_owner_key_is_live_and_worker_changes_leave_root_untouched() {
        let root = session("substitution");
        let before = root.export_envelope().unwrap();
        let mut worker = root.clone();
        let absent = Arc::make_mut(&mut worker.symbols).intern("absent-key");
        let step = worker.stack.last().unwrap().object.unwrap();
        let objects = worker.objects.as_mut().unwrap();
        objects
            .arena
            .set_field(
                step,
                objects.phases.k(Field::CardOwnerKey),
                ObjectValue::Atom(Datum::Text(absent)),
            )
            .unwrap();
        worker.advance_prefix(1).unwrap();
        assert_eq!(worker.stack.len(), 1);
        assert_eq!(root.export_envelope().unwrap().state, before.state);
        assert_eq!(
            root.export_envelope().unwrap().object_bindings,
            before.object_bindings
        );
    }
    #[test]
    fn piece_copy_binds_primary_body_to_owner_and_secondary_to_piece() {
        for (name, kind, expected) in [
            ("piece-primary", "place_unit", "hero_razzle"),
            ("piece-secondary", "move_sequence", "hero_razzle_piece_1"),
        ] {
            let mut worker = session(name);
            worker.advance_prefix(1).unwrap();
            let envelope = worker.export_envelope().unwrap();
            let child = envelope.state["execution_stack"]
                .as_array()
                .unwrap()
                .iter()
                .find(|step| step["type"] == kind)
                .unwrap();
            assert_eq!(child["unit_id"], expected);
        }
    }
}
