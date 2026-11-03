//! Live spellbook operations preserve physical spell and continuation identities.
use super::phases::Field;
use super::*;
use crate::{
    cards::CardId,
    spells::{SpellEvent, SpellOp},
    world::CardLifecycle,
};

pub(super) enum SpellPlan {
    Done {
        clear: bool,
    },
    Prompt {
        request: NativeRequest,
        clear: bool,
    },
    Cast {
        owner: Symbol,
        caster: Symbol,
        card: CardId,
        wish: bool,
    },
    WishStart {
        model: ObjectId,
        caster: Symbol,
        count: i64,
        required: i64,
        perform: bool,
        allowed: Vec<Symbol>,
    },
    WishFinish {
        cleanup: bool,
        winner: Option<Symbol>,
    },
    Remove {
        owner: Symbol,
        caster: Symbol,
        card: CardId,
        list: ObjectId,
        request: Option<NativeRequest>,
    },
    Prepare {
        actor: Symbol,
        returned: Vec<Symbol>,
        cards: Vec<CardId>,
        count: usize,
    },
}
impl NativeSession {
    fn spell_ids_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        field: Field,
    ) -> Result<(ObjectId, Vec<Symbol>)> {
        let Some(ObjectValue::Object(list)) = objects.arena.field(step, objects.phases.k(field))
        else {
            return Err(unsupported("spell ID list missing"));
        };
        let values = objects
            .arena
            .sequence(list)
            .ok_or_else(|| unsupported("spell IDs require a sequence"))?
            .iter()
            .map(|v| match v {
                ObjectValue::Atom(Datum::Text(id)) => Ok(*id),
                _ => Err(unsupported("spell ID must be text")),
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((list, values))
    }
    pub(super) fn spellbook_owner_bound(&self) -> Result<Option<Symbol>> {
        let mut owners = self.world.heroes.iter().filter(|h| !h.spells.is_empty());
        let owner = owners.next().map(|h| h.id);
        if owners.next().is_some() {
            return Err(unsupported(
                "multiple spellbook owners require source ValueError semantics",
            ));
        }
        Ok(owner)
    }
    fn spell_caster_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
        valid: bool,
    ) -> Result<Option<Symbol>> {
        let mut value = objects
            .arena
            .field(step, objects.phases.k(Field::CasterId))
            .filter(|v| !matches!(v, ObjectValue::Atom(Datum::Null)));
        if let Some(key) = self
            .passive_optional_text_bound(objects, step, Field::CasterKey)?
            .filter(|id| !self.symbols.name(*id).is_empty())
            && let Some(found) = objects
                .arena
                .context_value(key)
                .filter(|v| objects.arena.truthy(v))
        {
            value = Some(found);
        }
        let value = value.or_else(|| {
            self.planning
                .actor
                .map(|id| ObjectValue::Atom(Datum::Text(id)))
        });
        let Some(value) = value else { return Ok(None) };
        let caster = self.resource_identity_bound(objects, value)?;
        Ok((!valid || self.world.hero(caster).is_some()).then_some(caster))
    }
    pub(super) fn spell_has_state_bound(
        &self,
        objects: &BoundObjects,
        id: CardId,
        state: Field,
        fallback: CardLifecycle,
    ) -> bool {
        let Some(card) = self.world.card(id) else {
            return false;
        };
        match objects
            .arena
            .physical_override(card.source_instance, objects.phases.k(Field::CardState))
        {
            Some(Some(Datum::Text(value))) => *value == objects.phases.k(state),
            Some(_) => false,
            None => card.lifecycle == fallback,
        }
    }
    pub(super) fn prepared_spells_bound(
        &self,
        objects: &BoundObjects,
        owner: Symbol,
    ) -> Vec<CardId> {
        self.world
            .hero(owner)
            .unwrap()
            .spells
            .iter()
            .copied()
            .filter(|id| {
                self.spell_has_state_bound(
                    objects,
                    *id,
                    Field::SpellbookState,
                    CardLifecycle::Spellbook,
                )
            })
            .collect()
    }
    fn spell_prompt_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        caster: Symbol,
        choices: RequestChoices,
    ) -> Result<NativeRequest> {
        if self.next_request == u64::MAX && objects.token(step)?.is_none() {
            return Err(unsupported("request generation counter exhausted"));
        }
        let can_skip = matches!(&choices, RequestChoices::SpellCards { cast: false, .. });
        Ok(NativeRequest {
            controlled: None,
            id: RequestToken::Generated(0),
            owner: caster,
            prompt: objects.phases.k(Field::CastSpell),
            movement_range: None,
            pre_action_range: None,
            choices,
            can_skip,
            discard: false,
        })
    }
    pub(super) fn preflight_spell_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
        op: SpellOp,
    ) -> Result<SpellPlan> {
        self.sync_card_collections_bound(objects)?;
        if op == SpellOp::Wish {
            let raw = self.passive_text_bound(objects, step, Field::CasterId)?;
            let Some(caster) = self.world.hero(raw).map(|hero| hero.id) else {
                return Ok(SpellPlan::Done { clear: false });
            };
            let Some(model) = self.hero_model_bound(objects, caster) else {
                return Ok(SpellPlan::Done { clear: false });
            };
            let started = objects
                .arena
                .field(step, objects.phases.k(Field::Started))
                .is_some_and(|v| objects.arena.truthy(&v));
            let count = self
                .spatial_int_bound(objects, model, Field::WishCount)?
                .ok_or_else(|| unsupported("Wish count missing"))?;
            let required = self
                .spatial_int_bound(objects, step, Field::RequiredCasts)?
                .ok_or_else(|| unsupported("Wish required count missing"))?;
            if !started {
                let count = count
                    .checked_add(1)
                    .ok_or_else(|| unsupported("Wish count outside i64"))?;
                let perform = objects
                    .arena
                    .field(step, objects.phases.k(Field::PerformWish))
                    .is_some_and(|v| objects.arena.truthy(&v));
                let allowed = if perform {
                    Vec::new()
                } else if let Some(owner) = self.spellbook_owner_bound()? {
                    self.prepared_spells_bound(objects, owner)
                        .into_iter()
                        .map(|id| {
                            self.world.definitions.cards[self.world.card(id).unwrap().definition]
                                .printed_id
                        })
                        .filter(|id| *id != objects.phases.k(Field::Wish))
                        .collect()
                } else {
                    Vec::new()
                };
                self.phase_generation_check(0, 1)?;
                return Ok(SpellPlan::WishStart {
                    model,
                    caster,
                    count,
                    required,
                    perform,
                    allowed,
                });
            }
            let cleanup = matches!(objects.arena.context_value(objects.phases.k(Field::WishCountedCaster)),Some(ObjectValue::Atom(Datum::Text(id))) if id==caster);
            let winner = if count >= required {
                self.passive_optional_text_bound(objects, model, Field::ResourceTeam)?
            } else {
                None
            };
            if winner.is_some() {
                self.phase_generation_check(0, 1)?;
            }
            return Ok(SpellPlan::WishFinish { cleanup, winner });
        }
        let Some(owner) = self.spellbook_owner_bound()? else {
            return Ok(SpellPlan::Done { clear: false });
        };
        if op == SpellOp::Prepare {
            let mut returned = self
                .world
                .hero(owner)
                .unwrap()
                .spells
                .iter()
                .filter(|id| {
                    self.spell_has_state_bound(
                        objects,
                        **id,
                        Field::OutsideSpellbookState,
                        CardLifecycle::OutsideSpellbook,
                    )
                })
                .map(|id| {
                    self.world.definitions.cards[self.world.card(*id).unwrap().definition]
                        .printed_id
                })
                .collect::<Vec<_>>();
            returned.sort_by(|a, b| self.symbols.name(*a).cmp(self.symbols.name(*b)));
            let cards = returned
                .iter()
                .filter_map(|id| self.world.card_for_hero_printed(owner, *id))
                .collect::<Vec<_>>();
            if objects.effect_nodes()?.len() != self.world.effects.instances.len() {
                return Err(unsupported("spell preparation effect collection drift"));
            }
            let count = self
                .world
                .hero(owner)
                .unwrap()
                .spells
                .iter()
                .filter(|id| {
                    cards.contains(id)
                        || self.spell_has_state_bound(
                            objects,
                            **id,
                            Field::SpellbookState,
                            CardLifecycle::Spellbook,
                        )
                })
                .count();
            return Ok(SpellPlan::Prepare {
                actor: self.planning.actor.unwrap_or(owner),
                returned,
                cards,
                count,
            });
        }
        let Some(caster) = self.spell_caster_bound(objects, step, op == SpellOp::Cast)? else {
            return Ok(SpellPlan::Done { clear: false });
        };
        let mut eligible = self.prepared_spells_bound(objects, owner);
        if op == SpellOp::Cast {
            let (_, allowed) = self.spell_ids_bound(objects, step, Field::AllowedSpellIds)?;
            eligible.retain(|id| {
                allowed.contains(
                    &self.world.definitions.cards[self.world.card(*id).unwrap().definition]
                        .printed_id,
                )
            });
            if eligible.is_empty() {
                return Ok(SpellPlan::Done { clear: false });
            }
            let selected = objects.selected(step, self.selection_key);
            let card=eligible.iter().copied().find(|id|matches!(selected,Some(ObjectValue::Atom(Datum::Text(printed))) if printed==self.world.definitions.cards[self.world.card(*id).unwrap().definition].printed_id));
            if let Some(card) = card {
                self.phase_generation_check(0, 1)?;
                let wish = self.world.definitions.cards[self.world.card(card).unwrap().definition]
                    .printed_id
                    == objects.phases.k(Field::Wish);
                return Ok(SpellPlan::Cast {
                    owner,
                    caster,
                    card,
                    wish,
                });
            }
            return Ok(SpellPlan::Prompt {
                request: self.spell_prompt_bound(
                    objects,
                    step,
                    caster,
                    RequestChoices::SpellCards {
                        values: eligible,
                        cast: true,
                        removed: 0,
                        max: 0,
                    },
                )?,
                clear: false,
            });
        }
        let (list, removed) = self.spell_ids_bound(objects, step, Field::RemovedSpellIds)?;
        if !objects.arena.is_mutable_list(list) {
            return Err(unsupported("removed spell IDs require a mutable list"));
        }
        let max = self
            .spatial_int_bound(objects, step, Field::MaxRemovals)?
            .ok_or_else(|| unsupported("spell removal limit missing"))?;
        if i128::try_from(removed.len()).unwrap() >= i128::from(max) {
            return Ok(SpellPlan::Done { clear: false });
        }
        eligible.retain(|id| {
            !removed.contains(
                &self.world.definitions.cards[self.world.card(*id).unwrap().definition].printed_id,
            )
        });
        if eligible.is_empty() {
            return Ok(SpellPlan::Done { clear: false });
        }
        let selected = objects.selected(step, self.selection_key);
        if matches!(selected,Some(ObjectValue::Atom(Datum::Text(id))) if id==self.skip) {
            return Ok(SpellPlan::Done { clear: true });
        }
        let card=eligible.iter().copied().find(|id|matches!(selected,Some(ObjectValue::Atom(Datum::Text(printed))) if printed==self.world.definitions.cards[self.world.card(*id).unwrap().definition].printed_id));
        if let Some(card) = card {
            let next = removed
                .len()
                .checked_add(1)
                .ok_or_else(|| unsupported("spell removal count overflow"))?;
            let choices = self
                .prepared_spells_bound(objects, owner)
                .into_iter()
                .filter(|id| *id != card)
                .collect::<Vec<_>>();
            let request = if i128::try_from(next).unwrap() < i128::from(max) && !choices.is_empty()
            {
                Some(self.spell_prompt_bound(
                    objects,
                    step,
                    caster,
                    RequestChoices::SpellCards {
                        values: choices,
                        cast: false,
                        removed: next,
                        max,
                    },
                )?)
            } else {
                None
            };
            return Ok(SpellPlan::Remove {
                owner,
                caster,
                card,
                list,
                request,
            });
        }
        Ok(SpellPlan::Prompt {
            request: self.spell_prompt_bound(
                objects,
                step,
                caster,
                RequestChoices::SpellCards {
                    values: eligible,
                    cast: false,
                    removed: removed.len(),
                    max,
                },
            )?,
            clear: true,
        })
    }
    fn set_spell_state_bound(
        &mut self,
        objects: &mut BoundObjects,
        card: CardId,
        prepared: bool,
    ) -> Result<()> {
        let row = self
            .world
            .card_mut(card)
            .ok_or_else(|| unsupported("spell physical card missing"))?;
        row.lifecycle = if prepared {
            CardLifecycle::Spellbook
        } else {
            CardLifecycle::OutsideSpellbook
        };
        row.facedown = prepared;
        let physical = row.source_instance;
        for (field, value) in [
            (
                Field::CardState,
                Datum::Text(objects.phases.k(if prepared {
                    Field::SpellbookState
                } else {
                    Field::OutsideSpellbookState
                })),
            ),
            (Field::Facedown, Datum::Bool(prepared)),
        ] {
            objects
                .arena
                .set_physical_field(physical, objects.phases.k(field), value)
                .map_err(unsupported)?;
        }
        Ok(())
    }
    fn performed_spell_bound(
        &mut self,
        objects: &mut BoundObjects,
        caster: Symbol,
    ) -> Result<ObjectId> {
        let child = self.phase_step(objects, "perform_card")?;
        for (field, value) in [
            (
                Field::CardKey,
                Datum::Text(objects.phases.k(Field::CastSpell)),
            ),
            (
                Field::CardOwnerKey,
                Datum::Text(objects.phases.k(Field::SpellOwner)),
            ),
            (Field::HeroId, Datum::Text(caster)),
            (Field::SuppressAfterResolve, Datum::Bool(true)),
        ] {
            objects
                .arena
                .set_field(child, objects.phases.k(field), ObjectValue::Atom(value))
                .map_err(unsupported)?;
        }
        Ok(child)
    }
    pub(super) fn spell_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: SpellPlan,
        events: &mut Vec<NativeEvent>,
    ) -> Result<Option<NativeRequest>> {
        match plan {
            SpellPlan::Done { clear } => {
                if clear {
                    objects
                        .arena
                        .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                        .map_err(unsupported)?;
                }
            }
            SpellPlan::Prompt { request, clear } => {
                if clear {
                    objects
                        .arena
                        .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                        .map_err(unsupported)?;
                }
                return Ok(Some(request));
            }
            SpellPlan::Cast {
                owner,
                caster,
                card,
                wish,
            } => {
                let printed = self.world.definitions.cards
                    [self.world.card(card).unwrap().definition]
                    .printed_id;
                self.set_spell_state_bound(objects, card, false)?;
                for (key, value) in [
                    (Field::SpellOwner, owner),
                    (Field::SpellCaster, caster),
                    (Field::CastSpell, printed),
                ] {
                    objects.write(objects.phases.k(key), Datum::Text(value))?;
                }
                let child = if wish {
                    let child = self.phase_step(objects, "wish_sequence")?;
                    for (field, value) in [
                        (Field::CasterId, Datum::Text(caster)),
                        (Field::PerformWish, Datum::Bool(true)),
                    ] {
                        objects
                            .arena
                            .set_field(child, objects.phases.k(field), ObjectValue::Atom(value))
                            .map_err(unsupported)?;
                    }
                    child
                } else {
                    self.performed_spell_bound(objects, caster)?
                };
                Self::validate_returned_step_bound(objects, child)?;
                self.push_live_steps_bound(objects, &[child])?;
                events.push(NativeEvent::Spell(SpellEvent::Cast {
                    caster,
                    owner,
                    spell: printed,
                }));
            }
            SpellPlan::WishStart {
                model,
                caster,
                count,
                required,
                perform,
                allowed,
            } => {
                objects
                    .arena
                    .set_field(
                        step,
                        objects.phases.k(Field::Started),
                        ObjectValue::Atom(Datum::Bool(true)),
                    )
                    .map_err(unsupported)?;
                objects
                    .arena
                    .set_field(
                        model,
                        objects.phases.k(Field::WishCount),
                        ObjectValue::Atom(Datum::Int(count)),
                    )
                    .map_err(unsupported)?;
                let child = if perform {
                    objects.write(
                        objects.phases.k(Field::WishCountedCaster),
                        Datum::Text(caster),
                    )?;
                    self.performed_spell_bound(objects, caster)?
                } else {
                    let child = self.phase_step(objects, "cast_spell")?;
                    let values = objects.arena.list(
                        allowed
                            .into_iter()
                            .map(|id| ObjectValue::Atom(Datum::Text(id)))
                            .collect(),
                    );
                    objects
                        .arena
                        .set_field(child, objects.phases.k(Field::AllowedSpellIds), values)
                        .map_err(unsupported)?;
                    objects
                        .arena
                        .set_field(
                            child,
                            objects.phases.k(Field::CasterId),
                            ObjectValue::Atom(Datum::Text(caster)),
                        )
                        .map_err(unsupported)?;
                    child
                };
                Self::validate_returned_step_bound(objects, child)?;
                self.push_live_steps_bound(objects, &[child, step])?;
                events.push(NativeEvent::Spell(SpellEvent::WishCount {
                    caster,
                    count,
                    required,
                }));
            }
            SpellPlan::WishFinish { cleanup, winner } => {
                if cleanup {
                    objects
                        .arena
                        .remove_context(objects.phases.k(Field::WishCountedCaster))
                        .map_err(unsupported)?;
                }
                if let Some(winner) = winner {
                    let child = self.phase_step(objects, "game_over")?;
                    objects
                        .arena
                        .set_field(
                            child,
                            objects.phases.k(Field::Winner),
                            ObjectValue::Atom(Datum::Text(winner)),
                        )
                        .map_err(unsupported)?;
                    objects
                        .arena
                        .set_field(
                            child,
                            objects.phases.k(Field::Condition),
                            ObjectValue::Atom(Datum::Text(
                                objects.phases.k(Field::WishVictoryCondition),
                            )),
                        )
                        .map_err(unsupported)?;
                    Self::validate_returned_step_bound(objects, child)?;
                    self.push_live_steps_bound(objects, &[child])?;
                }
            }
            SpellPlan::Remove {
                owner,
                caster,
                card,
                list,
                request,
            } => {
                objects
                    .arena
                    .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                    .map_err(unsupported)?;
                let printed = self.world.definitions.cards
                    [self.world.card(card).unwrap().definition]
                    .printed_id;
                self.set_spell_state_bound(objects, card, false)?;
                objects
                    .arena
                    .append(list, ObjectValue::Atom(Datum::Text(printed)))
                    .map_err(unsupported)?;
                events.push(NativeEvent::Spell(SpellEvent::Removed {
                    caster,
                    owner,
                    spell: printed,
                }));
                return Ok(request);
            }
            SpellPlan::Prepare {
                actor,
                returned,
                cards,
                count,
            } => {
                for card in cards {
                    let printed = self.world.definitions.cards
                        [self.world.card(card).unwrap().definition]
                        .printed_id;
                    let removed = self
                        .world
                        .effects
                        .instances
                        .iter()
                        .enumerate()
                        .filter_map(|(i, e)| (e.card == Some(printed)).then_some(i))
                        .collect::<Vec<_>>();
                    let before = self
                        .world
                        .cards
                        .iter()
                        .map(|c| c.active)
                        .collect::<Vec<_>>();
                    objects.remove_effect_rows(&removed)?;
                    self.world.expire_effects_by_card(printed);
                    self.sync_effect_card_flags(objects, &before)?;
                    self.set_spell_state_bound(objects, card, true)?;
                }
                events.push(NativeEvent::Spell(SpellEvent::Prepared {
                    actor,
                    returned,
                    count,
                }));
            }
        }
        Ok(None)
    }
}
