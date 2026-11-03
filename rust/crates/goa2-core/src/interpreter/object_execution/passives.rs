//! Live passive discovery reads only the context operands its predicate needs.
use super::phases::Field;
use super::*;
use crate::{
    cards::CardId,
    passives::{Offer, Predicate},
    world::{ActionKind, CardLifecycle, EntityKind},
};

pub(super) enum OfferPlan {
    Done,
    Request(NativeRequest),
    Body(super::passive_bodies::BodyPlan),
}
pub(super) enum MarkPlan {
    Card(CardId, i64),
    Effect(Vec<usize>, ObjectId, i64),
}
impl NativeSession {
    pub(super) fn passive_text_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        field: Field,
    ) -> Result<Symbol> {
        match objects.arena.field(step, objects.phases.k(field)) {
            Some(ObjectValue::Atom(Datum::Text(value))) => Ok(value),
            _ => Err(unsupported("passive text operand missing")),
        }
    }
    pub(super) fn passive_optional_text_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        field: Field,
    ) -> Result<Option<Symbol>> {
        match objects.arena.field(step, objects.phases.k(field)) {
            None | Some(ObjectValue::Atom(Datum::Null)) => Ok(None),
            Some(ObjectValue::Atom(Datum::Text(value))) => Ok(Some(value)),
            _ => Err(unsupported("passive nullable text operand invalid")),
        }
    }
    fn passive_context_is(&self, objects: &BoundObjects, field: Field, value: Symbol) -> bool {
        matches!(objects.arena.context_value(objects.phases.k(field)), Some(ObjectValue::Atom(Datum::Text(found))) if found == value)
    }
    pub(super) fn passive_primary_allowed(&self, owner: Symbol, card: CardId) -> Result<bool> {
        let row = self
            .world
            .card(card)
            .ok_or_else(|| unsupported("passive primary card missing"))?;
        let action = (!row.facedown)
            .then_some(self.world.definitions.cards[row.definition].primary)
            .flatten();
        let Some(action) = action else {
            return Ok(true);
        };
        let action = if action == ActionKind::DefenseSkill {
            ActionKind::Skill
        } else {
            action
        };
        // Source {card: target} is the explicit physical-card argument below.
        // Empty registers exclude stale action context without a context copy.
        let context = Registers::default();
        Ok(self
            .world
            .action_legality(owner, action, Some(card), &context)
            .map_err(unsupported)?
            .allowed)
    }
    fn passive_defeat_bound(&self, objects: &BoundObjects, target: Symbol) -> Result<bool> {
        let Some(marks) = objects
            .arena
            .context_value(objects.phases.k(Field::DefeatMarks))
        else {
            return Ok(false);
        };
        let ObjectValue::Object(marks) = marks else {
            return Err(unsupported("passive defeat marks must be a mapping"));
        };
        if objects.arena.fields(marks).is_none() {
            return Err(unsupported("passive defeat marks must be a mapping"));
        }
        let mark = match objects
            .arena
            .field(marks, objects.phases.k(Field::IceVictim))
        {
            None | Some(ObjectValue::Atom(Datum::Null)) => return Ok(false),
            Some(ObjectValue::Atom(Datum::Int(value))) => value,
            Some(ObjectValue::Atom(Datum::Bool(value))) => i64::from(value),
            _ => return Err(unsupported("passive defeat mark must be an integer")),
        };
        let Some(log) = objects
            .arena
            .context_value(objects.phases.k(Field::DefeatLog))
        else {
            return Ok(false);
        };
        let ObjectValue::Object(log) = log else {
            return Err(unsupported("passive defeat log must be a sequence"));
        };
        let values = objects
            .arena
            .sequence(log)
            .ok_or_else(|| unsupported("passive defeat log must be a sequence"))?;
        let start = if mark < 0 {
            (values.len() as i128 + i128::from(mark)).max(0) as usize
        } else {
            usize::try_from(mark)
                .unwrap_or(usize::MAX)
                .min(values.len())
        };
        for value in &values[start..] {
            let ObjectValue::Object(pair) = value else {
                return Err(unsupported("passive defeat log pair missing"));
            };
            let pair = objects
                .arena
                .sequence(*pair)
                .filter(|p| p.len() == 2)
                .ok_or_else(|| unsupported("passive defeat log pair invalid"))?;
            if matches!(&pair[0], ObjectValue::Atom(Datum::Text(victim)) if *victim == target) {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn passive_eligible_bound(
        &self,
        objects: &BoundObjects,
        owner: Symbol,
        card: CardId,
        effect: Symbol,
        trigger: Symbol,
    ) -> Result<bool> {
        let p = &objects.phases;
        let hero = self
            .world
            .hero(owner)
            .ok_or_else(|| unsupported("passive owner missing"))?;
        match self.passive_rules.predicate(effect) {
            Predicate::Default => Ok(true),
            Predicate::Legend => Ok(trigger == p.k(Field::AfterPrimary)
                && hero
                    .card_in_turn_slot(
                        self.world
                            .combat
                            .turn
                            .checked_sub(1)
                            .ok_or_else(|| unsupported("passive turn overflow"))?,
                    )
                    .is_some()),
            Predicate::BulletTime => Ok(trigger == p.k(Field::AfterBasic)
                && objects
                    .arena
                    .context_value(p.k(Field::BasicActionCard))
                    .is_some_and(|v| objects.arena.truthy(&v))),
            Predicate::RuneMastery => {
                let model = self
                    .hero_model_bound(objects, owner)
                    .ok_or_else(|| unsupported("passive hero model missing"))?;
                let runes = objects
                    .arena
                    .field(model, p.k(Field::RuneSlots))
                    .ok_or_else(|| unsupported("passive rune slots missing"))?;
                Ok(objects.arena.truthy(&runes)
                    && !(trigger == p.k(Field::BeforeAction)
                        && self.passive_context_is(
                            objects,
                            Field::CurrentAction,
                            p.k(Field::DefenseKind),
                        )))
            }
            Predicate::Archwizard => {
                if trigger != p.k(Field::BeforeSpellbook) || self.planning.actor != Some(owner) {
                    return Ok(false);
                }
                let performing = objects
                    .arena
                    .context_value(p.k(Field::PerformingCard))
                    .filter(|v| objects.arena.truthy(v))
                    .or_else(|| objects.arena.context_value(p.k(Field::CurrentCard)));
                if !matches!(performing, Some(ObjectValue::Atom(Datum::Text(id))) if id == p.k(Field::PrepareSpells))
                {
                    return Ok(false);
                }
                let mut owners = self.world.heroes.iter().filter(|h| !h.spells.is_empty());
                let Some(spellbook) = owners.next() else {
                    return Ok(false);
                };
                if owners.next().is_some() {
                    return Err(unsupported("multiple spellbook owners"));
                }
                Ok(spellbook.spells.iter().any(|id| {
                    self.world.card(*id).is_some_and(|c| {
                        self.spell_has_state_bound(
                            objects,
                            *id,
                            Field::SpellbookState,
                            CardLifecycle::Spellbook,
                        ) && self.world.definitions.cards[c.definition].printed_id
                            == p.k(Field::Wish)
                    })
                }))
            }
            Predicate::BattleFury => {
                if !self.passive_context_is(
                    objects,
                    Field::DiscardSource,
                    p.k(Field::PlayedContainer),
                ) || !self.passive_context_is(objects, Field::DiscardedOwner, owner)
                {
                    return Ok(false);
                }
                let discarded = hero.discard.iter().copied().find(|id| {
                    self.passive_context_is(
                        objects,
                        Field::DiscardedId,
                        self.world.definitions.cards[self.world.card(*id).unwrap().definition]
                            .printed_id,
                    )
                });
                let Some(card) = discarded else {
                    return Ok(false);
                };
                let row = self.world.card(card).unwrap();
                if row.facedown
                    || self.world.definitions.cards[row.definition]
                        .primary
                        .is_none()
                {
                    return Ok(false);
                }
                self.passive_primary_allowed(owner, card)
            }
            Predicate::DragonKnight => {
                for card in hero.played.iter().flatten() {
                    let row = self.world.card(*card).unwrap();
                    if !row.facedown
                        && self.world.definitions.cards[row.definition].primary
                            == Some(ActionKind::Skill)
                        && self.passive_primary_allowed(owner, *card)?
                    {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Predicate::Winter => {
                if trigger != p.k(Field::AfterAttack)
                    || !self.passive_context_is(objects, Field::AttackCard, p.k(Field::IceBlast))
                {
                    return Ok(false);
                }
                let Some(target) = self.truthy_symbol_bound(
                    objects,
                    objects.arena.context_value(p.k(Field::IceVictim)),
                )?
                else {
                    return Ok(false);
                };
                if !self
                    .world
                    .entity(target)
                    .is_some_and(|e| e.kind == EntityKind::Minion)
                {
                    return Ok(false);
                }
                // The boundary accepts only the frozen rule-v9 state; its
                // target-defeat-log contract supersedes legacy block flags.
                self.passive_defeat_bound(objects, target)
            }
            Predicate::Jinx => {
                let printed = self.world.definitions.cards
                    [self.world.card(card).unwrap().definition]
                    .printed_id;
                if trigger != p.k(Field::AfterDiscard)
                    || !self.passive_context_is(objects, Field::DiscardedOwner, owner)
                    || !self
                        .world
                        .effects
                        .instances
                        .iter()
                        .any(|e| e.card == Some(printed) && e.source == owner && e.active)
                {
                    return Ok(false);
                }
                let Some(origin) = self.world.resolved_position(owner) else {
                    return Ok(false);
                };
                if !self
                    .world
                    .movement_legality(owner, 1, false, None, &Registers::default())
                    .map_err(unsupported)?
                    .allowed
                {
                    return Ok(false);
                }
                let ends = self
                    .world
                    .tile_hexes()
                    .iter()
                    .copied()
                    .filter(|h| *h != origin)
                    .collect::<Vec<_>>();
                Ok(self
                    .world
                    .movement_reachable(
                        &crate::geometry::MovementQuery {
                            start: origin,
                            ends,
                            max_steps: 1,
                            ignore_obstacles: false,
                            pass_through_obstacles: false,
                        },
                        Some(owner),
                        Some(&[owner]),
                        &[],
                    )
                    .map_err(unsupported)?
                    .into_iter()
                    .any(|v| v))
            }
        }
    }
    pub(super) fn preflight_passive_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Vec<Offer>> {
        let hero = self
            .passive_optional_text_bound(objects, step, Field::HeroId)?
            .filter(|id| !self.symbols.name(*id).is_empty());
        self.sync_card_collections_bound(objects)?;
        let trigger = self.passive_text_bound(objects, step, Field::Trigger)?;
        let offers = self
            .passive_rules
            .scan_with(
                &self.world,
                self.planning.actor,
                hero,
                trigger,
                |owner, card, effect| {
                    self.passive_eligible_bound(objects, owner, card, effect, trigger)
                        .map_err(|error| error.to_string())
                },
            )
            .map_err(unsupported)?;
        self.phase_generation_check(0, offers.len() as u64)?;
        Ok(offers)
    }
    pub(super) fn passive_bound(
        &mut self,
        objects: &mut BoundObjects,
        offers: Vec<Offer>,
    ) -> Result<()> {
        let mut steps = Vec::new();
        for offer in offers {
            let step = self.phase_step(objects, "offer_passive")?;
            for (field, value) in [
                (Field::CardId, Datum::Text(offer.card)),
                (Field::Trigger, Datum::Text(offer.trigger)),
                (Field::PassiveOptional, Datum::Bool(offer.optional)),
                (Field::Prompt, Datum::Text(offer.prompt)),
                (Field::HeroId, offer.hero.map_or(Datum::Null, Datum::Text)),
                (
                    Field::PassiveProvider,
                    offer.provider.map_or(Datum::Null, Datum::Text),
                ),
            ] {
                objects
                    .arena
                    .set_field(step, objects.phases.k(field), ObjectValue::Atom(value))
                    .map_err(unsupported)?;
            }
            steps.push(step);
        }
        self.push_live_steps_bound(objects, &steps)
    }
    pub(super) fn preflight_offer_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<OfferPlan> {
        self.sync_card_collections_bound(objects)?;
        let hero = self
            .passive_optional_text_bound(objects, step, Field::HeroId)?
            .filter(|id| !self.symbols.name(*id).is_empty());
        let Some(owner) = hero
            .or(self.planning.actor)
            .and_then(|id| self.world.hero(id).map(|h| h.id))
        else {
            return Ok(OfferPlan::Done);
        };
        let card = self.passive_text_bound(objects, step, Field::CardId)?;
        let trigger = self.passive_text_bound(objects, step, Field::Trigger)?;
        let prompt = self.passive_text_bound(objects, step, Field::Prompt)?;
        let optional = match objects
            .arena
            .field(step, objects.phases.k(Field::PassiveOptional))
        {
            Some(ObjectValue::Atom(Datum::Bool(value))) => value,
            _ => return Err(unsupported("passive optional flag must be boolean")),
        };
        let offer = Offer {
            card,
            trigger,
            prompt,
            optional,
            hero,
            provider: self.passive_optional_text_bound(objects, step, Field::PassiveProvider)?,
        };
        let Some((physical, effect)) = self.passive_rules.resolve(&self.world, owner, offer) else {
            return Ok(OfferPlan::Done);
        };
        if !self.reaction_rules.registered_effect(effect) {
            return Ok(OfferPlan::Done);
        }
        self.passive_rules
            .validate_trigger(trigger)
            .map_err(unsupported)?;
        let pending = objects
            .arena
            .field(step, objects.pending)
            .filter(|v| objects.arena.truthy(v));
        if optional {
            if let Some(pending) = pending {
                if !matches!(pending, ObjectValue::Object(id) if objects.arena.fields(id).is_some())
                {
                    return Err(unsupported("passive pending input must be a mapping"));
                }
                if !matches!(objects.selected(step, self.selection_key), Some(ObjectValue::Atom(Datum::Text(choice))) if choice == self.yes)
                {
                    return Ok(OfferPlan::Done);
                }
            } else {
                if self.next_request == u64::MAX && objects.token(step)?.is_none() {
                    return Err(unsupported("request generation counter exhausted"));
                }
                let row = self.world.card(physical).unwrap();
                return Ok(OfferPlan::Request(NativeRequest {
                    controlled: None,
                    id: RequestToken::Generated(0),
                    owner,
                    prompt,
                    movement_range: None,
                    pre_action_range: None,
                    choices: RequestChoices::Passive {
                        card,
                        name: self.world.definitions.cards[row.definition].name,
                    },
                    can_skip: false,
                    discard: false,
                }));
            }
        }
        Ok(
            match self.passive_body_plan(objects, owner, physical, effect, offer)? {
                Some(plan) => OfferPlan::Body(plan),
                None => OfferPlan::Done,
            },
        )
    }
    pub(super) fn offer_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: OfferPlan,
    ) -> Result<Option<NativeRequest>> {
        match plan {
            OfferPlan::Done => Ok(None),
            OfferPlan::Request(request) => Ok(Some(request)),
            OfferPlan::Body(body) => {
                self.queue_passive_body_bound(objects, body)?;
                Ok(None)
            }
        }
    }
    pub(super) fn preflight_mark_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<MarkPlan>> {
        let printed = self.passive_text_bound(objects, step, Field::CardId)?;
        if let Some(provider) =
            self.passive_optional_text_bound(objects, step, Field::PassiveProvider)?
        {
            if let Some((index, effect)) = self
                .world
                .effects
                .instances
                .iter()
                .enumerate()
                .find(|(_, e)| e.id == provider && e.card == Some(printed))
            {
                let nodes = objects.effect_nodes()?;
                if nodes.len() != self.world.effects.instances.len() {
                    return Err(unsupported("passive effect binding drift"));
                }
                return Ok(Some(MarkPlan::Effect(
                    nodes
                        .iter()
                        .enumerate()
                        .filter_map(|(i, node)| (*node == nodes[index]).then_some(i))
                        .collect(),
                    nodes[index],
                    effect
                        .passive_uses
                        .checked_add(1)
                        .ok_or_else(|| unsupported("passive usage overflow"))?,
                )));
            }
        } else {
            self.sync_card_collections_bound(objects)?;
            if let Some(card) = self
                .planning
                .actor
                .and_then(|owner| crate::passives::PassiveRules::card(&self.world, owner, printed))
            {
                return Ok(Some(MarkPlan::Card(
                    card,
                    self.world
                        .card(card)
                        .unwrap()
                        .passive_uses
                        .checked_add(1)
                        .ok_or_else(|| unsupported("passive usage overflow"))?,
                )));
            }
        }
        Ok(None)
    }
    pub(super) fn mark_passive_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<MarkPlan>,
    ) -> Result<()> {
        match plan {
            Some(MarkPlan::Card(id, value)) => {
                let row = self.world.card_mut(id).unwrap();
                row.passive_uses = value;
                objects
                    .arena
                    .set_physical_field(
                        row.source_instance,
                        objects.phases.k(Field::PassiveUses),
                        Datum::Int(value),
                    )
                    .map_err(unsupported)?;
            }
            Some(MarkPlan::Effect(indices, node, value)) => {
                for index in indices {
                    self.world.effects.instances[index].passive_uses = value;
                }
                objects
                    .arena
                    .set_field(
                        node,
                        objects.phases.k(Field::PassiveUses),
                        ObjectValue::Atom(Datum::Int(value)),
                    )
                    .map_err(unsupported)?;
            }
            None => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(name: &str) -> NativeSession {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let index: Value = serde_json::from_reader(std::io::BufReader::new(
            crate::fixture_cache::open(base.join("object-execution.json")).unwrap(),
        ))
        .unwrap();
        let entry = index["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == name)
            .unwrap();
        let case: Value = serde_json::from_reader(std::io::BufReader::new(
            crate::fixture_cache::open(base.join(entry["path"].as_str().unwrap())).unwrap(),
        ))
        .unwrap();
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    fn provider_alias_mark_updates_all_dense_rows_but_independent_copy_stays_private() {
        for alias in [false, true] {
            let mut session = session(if alias {
                "object-execution-passive-mark-provider-alias-True"
            } else {
                "object-execution-passive-mark-provider-alias-False"
            });
            let before = session.world.effects.instances[0].passive_uses;
            session.advance_prefix(1).unwrap();
            assert_eq!(session.world.effects.instances[0].passive_uses, before + 1);
            assert_eq!(
                session.world.effects.instances[1].passive_uses,
                before + i64::from(alias)
            );
            let restored = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
            assert_eq!(
                session
                    .world
                    .effects
                    .instances
                    .iter()
                    .map(|e| e.passive_uses)
                    .collect::<Vec<_>>(),
                restored
                    .world
                    .effects
                    .instances
                    .iter()
                    .map(|e| e.passive_uses)
                    .collect::<Vec<_>>()
            );
        }
    }
    #[test]
    fn passive_usage_overflow_is_guarded_before_pop_across_restore() {
        let mut session = session("object-execution-passive-mark-played");
        let owner = session.planning.actor.unwrap();
        let id = session.world.hero(owner).unwrap().played[0].unwrap();
        session.world.card_mut(id).unwrap().passive_uses = i64::MAX;
        let objects = session.objects.as_mut().unwrap();
        objects
            .arena
            .set_physical_field(
                id.0,
                objects.phases.k(Field::PassiveUses),
                Datum::Int(i64::MAX),
            )
            .unwrap();
        let before = session.export_envelope().unwrap();
        let mut restored = NativeSession::compile(before.clone()).unwrap();
        for worker in [&mut session, &mut restored] {
            assert!(
                worker
                    .advance_prefix(1)
                    .unwrap_err()
                    .to_string()
                    .contains("passive usage overflow")
            );
            assert_eq!(worker.export_envelope().unwrap().state, before.state);
            assert_eq!(
                worker.export_envelope().unwrap().object_bindings,
                before.object_bindings
            );
        }
    }
}
