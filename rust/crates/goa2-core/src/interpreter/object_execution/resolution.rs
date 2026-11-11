//! Live resolution boundaries, preserving the actual step input and context.
use super::phases::Field;
use super::{
    BoundObjects, Datum, NativeRequest, NativeSession, ObjectId, ObjectKind, ObjectValue,
    RequestChoices, RequestToken, Result, Symbol, unsupported,
};

impl NativeSession {
    fn finalization_hero_bound(&self, objects: &BoundObjects, step: ObjectId) -> Result<Symbol> {
        match objects.arena.field(step, objects.phases.k(Field::HeroId)) {
            Some(ObjectValue::Atom(Datum::Text(hero))) => Ok(hero),
            _ => Err(unsupported("finalization hero must be text")),
        }
    }
    pub(super) fn preflight_finalize_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<()> {
        self.phase_generation_check(0, 3)?;
        let hero = self.finalization_hero_bound(objects, step)?;
        if let Some(model) = self.hero_model_bound(objects, hero) {
            self.sync_card_collections_bound(objects)?;
            if !(1..=1_000_000).contains(&self.world.combat.turn) {
                return Err(unsupported(
                    "finalization turn slot outside supported bounds",
                ));
            }
            let Some(ObjectValue::Object(played)) =
                objects.arena.field(model, objects.phases.k(Field::Played))
            else {
                return Err(unsupported("played cards missing"));
            };
            if !objects.arena.is_mutable_list(played) {
                return Err(unsupported("played cards must be a mutable list"));
            }
            if objects.effect_nodes()?.len() != self.world.effects.instances.len() {
                return Err(unsupported("finalization effect binding drift"));
            }
        }
        for field in [Field::Guess, Field::Reveal] {
            if let Some(value) = objects.arena.root_field(objects.phases.k(field))
                && objects.arena.truthy(&value)
                && !matches!(value, ObjectValue::Object(id) if objects.arena.kind(id) == ObjectKind::Dict)
            {
                return Err(unsupported("guess/reveal state must be a mapping"));
            }
        }
        Ok(())
    }
    pub(super) fn finalize_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
    ) -> Result<Symbol> {
        let hero = self.finalization_hero_bound(objects, step)?;
        if let Some(model) = self.hero_model_bound(objects, hero) {
            let current = self.model_card_bound(objects, model, Field::Current)?;
            let printed = current
                .and_then(|id| self.world.card(id))
                .map(|card| self.world.definitions.cards[card.definition].printed_id);
            self.resolve_turn_slot_bound(objects, hero, model)?;
            if let Some(printed) = printed {
                self.world.effects.activate_card(printed, true);
            }
            let row = self
                .world
                .hero(hero)
                .ok_or_else(|| unsupported("finalization hero cache missing"))?;
            let owner = row.id;
            let cards = row
                .played
                .iter()
                .flatten()
                .copied()
                .chain(row.ultimate)
                .collect::<Vec<_>>();
            for id in cards {
                let card = self
                    .world
                    .card_mut(id)
                    .ok_or_else(|| unsupported("finalization card missing"))?;
                if card.passive_uses > 0 {
                    card.passive_uses = 0;
                    objects
                        .arena
                        .set_physical_field(
                            card.source_instance,
                            objects.phases.k(Field::PassiveUses),
                            Datum::Int(0),
                        )
                        .map_err(unsupported)?;
                }
            }
            let nodes = objects.effect_nodes()?;
            for (effect, node) in self.world.effects.instances.iter_mut().zip(nodes) {
                if printed.is_some() && effect.card == printed {
                    objects
                        .arena
                        .set_field(
                            node,
                            objects.phases.k(Field::EffectActive),
                            ObjectValue::Atom(Datum::Bool(true)),
                        )
                        .map_err(unsupported)?;
                }
                if effect.source == owner && effect.passive_uses > 0 {
                    effect.passive_uses = 0;
                    objects
                        .arena
                        .set_field(
                            node,
                            objects.phases.k(Field::PassiveUses),
                            ObjectValue::Atom(Datum::Int(0)),
                        )
                        .map_err(unsupported)?;
                }
            }
            self.sync_card_collections_bound(objects)?;
        }
        for (field, owner) in [
            (Field::Guess, Field::Guesser),
            (Field::Reveal, Field::Revealer),
        ] {
            if let Some(value) = objects.arena.root_field(objects.phases.k(field))
                && objects.arena.truthy(&value)
            {
                let ObjectValue::Object(id) = value else {
                    return Err(unsupported("guess/reveal mapping missing"));
                };
                if !matches!(objects.arena.field(id, objects.phases.k(owner)), Some(ObjectValue::Atom(Datum::Text(id))) if id == hero)
                {
                    objects
                        .arena
                        .set_root(objects.phases.k(field), ObjectValue::Atom(Datum::Null))
                        .map_err(unsupported)?;
                }
            }
        }
        self.boundary.finalize(hero, &self.symbols);
        objects
            .arena
            .clear_dictionary(objects.arena.context_id())
            .map_err(unsupported)?;
        self.context.clear();
        self.world.acting_piece = None;
        self.planning.actor = None;
        self.planning.owner = None;
        for field in [
            objects.actor,
            objects.phases.k(Field::Owner),
            objects.phases.k(Field::Piece),
        ] {
            objects
                .arena
                .set_root(field, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
        }
        let mut steps = Vec::with_capacity(3);
        for name in ["return_minions", "check_lane_push", "find_actor"] {
            steps.push(self.phase_step(objects, name)?);
        }
        self.push_live_steps_bound(objects, &steps)?;
        Ok(hero)
    }
    fn confirmation_frozen_bound(&self, objects: &BoundObjects) -> bool {
        [Field::RollbackFrozen, Field::RollbackReanchor]
            .into_iter()
            .any(|field| {
                objects
                    .arena
                    .context_value(objects.phases.k(field))
                    .is_some_and(|value| objects.arena.truthy(&value))
            })
    }
    pub(super) fn preflight_confirm_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<()> {
        if !self.confirmation_frozen_bound(objects)
            && let Some(value) = objects.arena.field(step, objects.pending)
            && objects.arena.truthy(&value)
            && !matches!(value, ObjectValue::Object(id) if objects.arena.kind(id) == crate::codec::ObjectKind::Dict)
        {
            return Err(unsupported("confirmation input must be a mapping"));
        }
        Ok(())
    }
    pub(super) fn confirm_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
    ) -> Result<Option<NativeRequest>> {
        if self.confirmation_frozen_bound(objects) {
            return Ok(None);
        }
        let p = &objects.phases;
        if objects
            .arena
            .field(step, objects.pending)
            .is_some_and(|value| objects.arena.truthy(&value))
        {
            let confirmed = matches!(objects.selected(step, self.selection_key), Some(ObjectValue::Atom(Datum::Text(value))) if value == p.k(Field::Confirm));
            // Source consumes pending input but retains the finished step's
            // request ID. Clearing both would mutate retained aliases.
            objects
                .arena
                .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
            if confirmed {
                return Ok(None);
            }
        }
        let Some(ObjectValue::Atom(Datum::Text(hero))) =
            objects.arena.field(step, p.k(Field::HeroId))
        else {
            return Err(unsupported("confirmation hero must be text"));
        };
        Ok(Some(NativeRequest {
            controlled: None,
            id: RequestToken::Generated(0),
            owner: hero,
            prompt: p.k(Field::ConfirmPrompt),
            movement_range: None,
            pre_action_range: None,
            choices: RequestChoices::ConfirmAction,
            can_skip: false,
            discard: false,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixture_case;
    use super::*;

    #[test]
    fn slot_resolution_updates_shared_caches_before_followup_consumers() {
        let fixture = fixture_case("object-execution-finalization-shared-played");
        let mut session =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        session
            .with_live_objects(|session, objects| {
                let hero = session.world.heroes[0].id;
                let model = session.hero_model_bound(objects, hero).unwrap();
                session.resolve_turn_slot_bound(objects, hero, model)?;
                assert_eq!(
                    session.world.heroes[0].played,
                    session.world.heroes[1].played
                );
                assert_eq!(session.world.heroes[1].resolved_turns, 0);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn finalization_refreshes_shared_slots_but_not_the_other_heroes_turn_count() {
        let fixture = fixture_case("object-execution-finalization-shared-played");
        let mut session =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        session.advance_prefix(1).unwrap();
        assert_eq!(
            session.world.heroes[0].played,
            session.world.heroes[1].played
        );
        assert_eq!(session.world.heroes[0].resolved_turns, 1);
        assert_eq!(session.world.heroes[1].resolved_turns, 0);
        assert!(session.world.heroes[0].current.is_none());
    }

    #[test]
    fn finalization_generation_exhaustion_rejects_before_pop_or_state_mutation() {
        let fixture = fixture_case("object-execution-finalization-basic");
        let mut session =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        session.next_step = u64::MAX - 2;
        let before = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
        assert!(
            session
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("generation")
        );
        assert_eq!(
            serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
            before
        );
    }

    #[test]
    fn a_piece_finalizer_resolves_the_parent_card_but_keeps_the_event_actor() {
        let fixture = fixture_case("object-execution-finalization-piece-id");
        let mut session =
            NativeSession::compile(serde_json::from_value(fixture["envelope"].clone()).unwrap())
                .unwrap();
        let card = session.world.heroes[0].current;
        assert!(card.is_some());
        let result = session.advance_prefix(1).unwrap();
        assert!(session.world.heroes[0].current.is_none());
        assert_eq!(session.world.heroes[0].resolved_turns, 1);
        assert!(session.world.acting_piece.is_none());
        assert_eq!(
            session.event_json(&result.events[0])["actor_id"],
            "hero_razzle_piece_1"
        );
    }
}
