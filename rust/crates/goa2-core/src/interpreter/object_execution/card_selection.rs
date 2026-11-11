//! Card selectors read live operands and share the typed candidate rules.
use super::phases::Field;
use super::{
    BoundObjects, Datum, NativeRequest, NativeSession, ObjectId, ObjectKind, ObjectValue,
    RequestChoices, RequestToken, Result, Symbol, unsupported,
};
use crate::cards::CardId;
use crate::{
    card_selection::{CardSelectSpec, Container},
    world::{ActionKind, CardLifecycle},
};

pub(super) struct CardSelectionPlan {
    spec: CardSelectSpec,
    owner: Option<Symbol>,
    choices: Vec<CardId>,
    selected: Option<Symbol>,
}
impl NativeSession {
    fn card_field_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        field: Field,
    ) -> Option<ObjectValue> {
        objects.arena.field(step, objects.phases.k(field))
    }
    fn card_text_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        field: Field,
    ) -> Result<Option<Symbol>> {
        self.truthy_symbol_bound(objects, self.card_field_bound(objects, step, field))
    }
    fn card_bool_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        field: Field,
    ) -> Result<Option<bool>> {
        match self.card_field_bound(objects, step, field) {
            None | Some(ObjectValue::Atom(Datum::Null)) => Ok(None),
            Some(ObjectValue::Atom(Datum::Bool(value))) => Ok(Some(value)),
            _ => Err(unsupported("card selector boolean binding invalid")),
        }
    }
    fn card_list_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        field: Field,
    ) -> Result<Option<Vec<Symbol>>> {
        match self.card_field_bound(objects, step, field) {
            None | Some(ObjectValue::Atom(Datum::Null)) => Ok(None),
            Some(ObjectValue::Object(id)) => objects
                .arena
                .sequence(id)
                .ok_or_else(|| unsupported("card selector list binding invalid"))?
                .iter()
                .map(|v| match v {
                    ObjectValue::Atom(Datum::Text(value)) => Ok(*value),
                    _ => Err(unsupported("card selector list needs text")),
                })
                .collect::<Result<Vec<_>>>()
                .map(Some),
            _ => Err(unsupported("card selector list binding invalid")),
        }
    }
    pub(super) fn card_spec_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<CardSelectSpec> {
        let Some(ObjectValue::Object(filters)) =
            self.card_field_bound(objects, step, Field::Filters)
        else {
            return Err(unsupported("card selector filters missing"));
        };
        if !objects
            .arena
            .sequence(filters)
            .is_some_and(|filters| filters.is_empty())
        {
            return Err(unsupported("live generic CARD filters not yet bound"));
        }
        let required = |field| match self.card_field_bound(objects, step, field) {
            Some(ObjectValue::Atom(Datum::Text(value))) => Ok(value),
            _ => Err(unsupported("card selector required text missing")),
        };
        let container = |id| match self.symbols.name(id) {
            "HAND" => Ok(Container::Hand),
            "PLAYED" => Ok(Container::Played),
            "DISCARD" => Ok(Container::Discard),
            "DECK" => Ok(Container::Deck),
            _ => Err(unsupported("card container invalid")),
        };
        let lifecycle = |id| match self.symbols.name(id) {
            "HAND" => Ok(CardLifecycle::Hand),
            "DECK" => Ok(CardLifecycle::Deck),
            "DISCARD" => Ok(CardLifecycle::Discard),
            "UNRESOLVED" => Ok(CardLifecycle::Unresolved),
            "RESOLVED" => Ok(CardLifecycle::Resolved),
            "ITEM" => Ok(CardLifecycle::Item),
            "PASSIVE" => Ok(CardLifecycle::Passive),
            "RETIRED" => Ok(CardLifecycle::Retired),
            "SPELLBOOK" => Ok(CardLifecycle::Spellbook),
            "OUTSIDE_SPELLBOOK" => Ok(CardLifecycle::OutsideSpellbook),
            _ => Err(unsupported("card state invalid")),
        };
        let action = |id| match self.symbols.name(id) {
            "MOVEMENT" => Ok(ActionKind::Movement),
            "ATTACK" => Ok(ActionKind::Attack),
            "SKILL" => Ok(ActionKind::Skill),
            "DEFENSE" => Ok(ActionKind::Defense),
            "DEFENSE_SKILL" => Ok(ActionKind::DefenseSkill),
            "HOLD" => Ok(ActionKind::Hold),
            "CLEAR" => Ok(ActionKind::Clear),
            "FAST_TRAVEL" => Ok(ActionKind::FastTravel),
            _ => Err(unsupported("card action invalid")),
        };
        let mut containers = self
            .card_list_bound(objects, step, Field::Containers)?
            .unwrap_or_default()
            .into_iter()
            .map(container)
            .collect::<Result<Vec<_>>>()?;
        if containers.is_empty() {
            containers.push(container(required(Field::Container)?)?);
        }
        let colors = self
            .card_list_bound(objects, step, Field::CardColors)?
            .unwrap_or_default();
        for color in &colors {
            crate::card_selection::validate_color(self.symbols.name(*color))
                .map_err(unsupported)?;
        }
        Ok(CardSelectSpec {
            output: required(Field::Output)?,
            prompt: required(Field::Prompt)?,
            auto: self
                .card_bool_bound(objects, step, Field::Auto)?
                .unwrap_or(false),
            discard: self.card_text_bound(objects, step, Field::DecisionPurpose)?
                == Some(objects.phases.k(Field::DiscardPurpose)),
            hero: self.card_text_bound(objects, step, Field::ContextHero)?,
            hero_key: self.card_text_bound(objects, step, Field::ContextHeroKey)?,
            owner: self.card_text_bound(objects, step, Field::OverridePlayer)?,
            owner_key: self.card_text_bound(objects, step, Field::OverridePlayerKey)?,
            containers,
            shields: self
                .card_bool_bound(objects, step, Field::ShieldsOnly)?
                .unwrap_or(false),
            facedown: self
                .card_bool_bound(objects, step, Field::IncludeFacedown)?
                .unwrap_or(false),
            actions: self
                .card_list_bound(objects, step, Field::CardActions)?
                .map(|v| v.into_iter().map(action).collect::<Result<Vec<_>>>())
                .transpose()?,
            colors,
            color_key: self.card_text_bound(objects, step, Field::CardColorKey)?,
            tier_key: self.card_text_bound(objects, step, Field::CardTierKey)?,
            basic: self.card_bool_bound(objects, step, Field::Basic)?,
            active: self.card_bool_bound(objects, step, Field::CardActive)?,
            item: self.card_bool_bound(objects, step, Field::CardItem)?,
            allowed: self.card_list_bound(objects, step, Field::AllowedCards)?,
            exclude: self
                .card_list_bound(objects, step, Field::ExcludeCardKeys)?
                .unwrap_or_default(),
            states: self
                .card_list_bound(objects, step, Field::CardStates)?
                .map(|v| v.into_iter().map(lifecycle).collect::<Result<Vec<_>>>())
                .transpose()?,
            exclude_states: self
                .card_list_bound(objects, step, Field::ExcludeCardStates)?
                .unwrap_or_default()
                .into_iter()
                .map(lifecycle)
                .collect::<Result<Vec<_>>>()?,
            color_output: self.card_text_bound(objects, step, Field::SelectedColor)?,
            tier_output: self.card_text_bound(objects, step, Field::SelectedTier)?,
        })
    }
    pub(super) fn preflight_card_selection_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
        mandatory: bool,
    ) -> Result<CardSelectionPlan> {
        self.sync_card_collections_bound(objects)?;
        let spec = self.card_spec_bound(objects, step)?;
        let read = |key: Option<Symbol>| {
            self.truthy_symbol_bound(
                objects,
                key.and_then(|key| objects.arena.context_value(key)),
            )
            .map_err(|error| error.to_string())
        };
        let owner = spec
            .owner_with(self.planning.actor, None, read)
            .map_err(unsupported)?;
        let choices = spec
            .choices_with(&self.world, &self.symbols, owner, None, read)
            .map_err(unsupported)?;
        let mut selected = None;
        if !choices.is_empty() {
            if spec.auto && choices.len() == 1 && mandatory {
                selected = Some(
                    self.world.definitions.cards[self.world.card(choices[0]).unwrap().definition]
                        .printed_id,
                );
            } else if objects
                .arena
                .field(step, objects.pending)
                .is_some_and(|v| objects.arena.truthy(&v))
            {
                if !matches!(objects.arena.field(step, objects.pending), Some(ObjectValue::Object(id)) if objects.arena.kind(id) == ObjectKind::Dict)
                {
                    return Err(unsupported("card pending input must be a mapping"));
                }
                if let Some(ObjectValue::Atom(Datum::Text(id))) =
                    objects.selected(step, self.selection_key)
                    && ((id == self.skip && !mandatory)
                        || choices.iter().any(|card| {
                            self.world.definitions.cards[self.world.card(*card).unwrap().definition]
                                .printed_id
                                == id
                        }))
                {
                    selected = Some(id);
                }
            }
        }
        Ok(CardSelectionPlan {
            spec,
            owner,
            choices,
            selected,
        })
    }
    pub(super) fn card_selection_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: CardSelectionPlan,
        mandatory: bool,
    ) -> Result<Option<NativeRequest>> {
        if plan.choices.is_empty() {
            if mandatory {
                self.abort_objects(objects)?;
            } else {
                objects
                    .arena
                    .remove_context(plan.spec.output)
                    .map_err(unsupported)?;
            }
            return Ok(None);
        }
        if let Some(selected) = plan.selected {
            if selected == self.skip && !mandatory {
                objects
                    .arena
                    .remove_context(plan.spec.output)
                    .map_err(unsupported)?;
            } else {
                objects
                    .arena
                    .set_context(plan.spec.output, ObjectValue::Atom(Datum::Text(selected)))
                    .map_err(unsupported)?;
                let id = plan
                    .choices
                    .iter()
                    .find(|id| {
                        self.world.definitions.cards[self.world.card(**id).unwrap().definition]
                            .printed_id
                            == selected
                    })
                    .unwrap();
                let definition =
                    &self.world.definitions.cards[self.world.card(*id).unwrap().definition];
                for (key, value) in [
                    (
                        plan.spec.color_output,
                        definition.color.map_or(Datum::Null, Datum::Text),
                    ),
                    (plan.spec.tier_output, Datum::Text(definition.tier)),
                ] {
                    if let Some(key) = key {
                        objects
                            .arena
                            .set_context(key, ObjectValue::Atom(value))
                            .map_err(unsupported)?;
                    }
                }
            }
            objects.clear_pending(step)?;
            return Ok(None);
        }
        Ok(Some(NativeRequest {
            controlled: None,
            id: RequestToken::Generated(0),
            owner: self.normalized_owner(plan.owner.unwrap_or(self.no_actor)),
            prompt: plan.spec.prompt,
            movement_range: None,
            pre_action_range: None,
            choices: RequestChoices::CardIds {
                values: plan
                    .choices
                    .iter()
                    .map(|id| {
                        self.world.definitions.cards[self.world.card(*id).unwrap().definition]
                            .printed_id
                    })
                    .collect(),
                forced_victim: None,
                defeat_victim: None,
            },
            can_skip: !mandatory,
            discard: plan.spec.discard,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixture_case;
    use super::*;
    fn session() -> NativeSession {
        let case = fixture_case("object-execution-card-selector-hand");
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    fn selector_workers_read_private_live_allowed_lists_and_metadata() {
        let root = session();
        let before = serde_json::to_value(root.export_envelope().unwrap()).unwrap();
        let hero = root.world.hero(root.planning.actor.unwrap()).unwrap();
        let ids = hero.hand[..2]
            .iter()
            .map(|id| {
                root.world.definitions.cards[root.world.card(*id).unwrap().definition].printed_id
            })
            .collect::<Vec<_>>();
        let mut a = root.clone();
        let mut b = root.clone();
        for (worker, id) in [(&mut a, ids[0]), (&mut b, ids[1])] {
            let step = worker.stack.last().unwrap().object.unwrap();
            let objects = worker.objects.as_mut().unwrap();
            let list = objects.arena.list(vec![ObjectValue::Atom(Datum::Text(id))]);
            objects
                .arena
                .set_field(step, objects.phases.k(Field::AllowedCards), list)
                .unwrap();
            objects
                .arena
                .set_field(
                    step,
                    objects.phases.k(Field::Auto),
                    ObjectValue::Atom(Datum::Bool(true)),
                )
                .unwrap();
            assert!(worker.advance_prefix(1).unwrap().request.is_none());
            assert_eq!(
                worker.semantic_state().unwrap()["execution_context"]["chosen"],
                worker.symbols.name(id)
            );
        }
        assert_ne!(
            a.semantic_state().unwrap()["execution_context"]["chosen"],
            b.semantic_state().unwrap()["execution_context"]["chosen"]
        );
        assert_eq!(
            serde_json::to_value(root.export_envelope().unwrap()).unwrap(),
            before
        );
    }
    #[test]
    fn selector_refreshes_private_hand_lists_before_candidate_scan() {
        let root = session();
        let mut empty = root.clone();
        let model = empty.objects.as_ref().unwrap().phases.heroes[0].1;
        let objects = empty.objects.as_mut().unwrap();
        let Some(ObjectValue::Object(hand)) =
            objects.arena.field(model, objects.phases.k(Field::Hand))
        else {
            panic!("hand missing");
        };
        objects.arena.clear_list(hand).unwrap();
        empty.advance_prefix(1).unwrap();
        assert!(empty.last_request.is_none());
        assert!(empty.world.heroes[0].hand.is_empty());
        assert!(!root.world.heroes[0].hand.is_empty());
    }
    #[test]
    fn generic_card_predicates_guard_before_stack_or_arena_mutation() {
        let mut session = session();
        let step = session.stack.last().unwrap().object.unwrap();
        let objects = session.objects.as_mut().unwrap();
        let filters = objects
            .arena
            .list(vec![ObjectValue::Atom(Datum::Bool(true))]);
        objects
            .arena
            .set_field(step, objects.phases.k(Field::Filters), filters)
            .unwrap();
        let before = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
        assert!(
            session
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("generic CARD filters")
        );
        assert_eq!(
            serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
            before
        );
    }
}
