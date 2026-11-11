//! Card corrections reuse live physical instances and preserve the deck registry.
use super::{
    BoundObjects, Datum, EngineError, NativeSession, ObjectValue, Phase, Result, Symbol,
    phases::Field, unsupported,
};
use crate::{
    cards::{CardId, HeroRecord},
    interpreter::live_game::CardPatchZone,
    objects::{Mutation, Target},
    world::CardLifecycle,
};

impl NativeSession {
    fn patch_card_instance(
        &self,
        hero: &HeroRecord,
        name: &str,
        (first, second): (Option<CardId>, Option<CardId>),
    ) -> Option<CardId> {
        let printed = self.symbols.lookup(name)?;
        hero.hand
            .iter()
            .copied()
            .chain(hero.discard.iter().copied())
            .chain(hero.played.iter().flatten().copied())
            .chain(hero.current)
            .chain(hero.extra)
            .chain(first)
            .chain(second)
            .chain(hero.deck.iter().copied())
            .find(|id| {
                self.world.card(*id).is_some_and(|row| {
                    let definition = &self.world.definitions.cards[row.definition];
                    !matches!(row.lifecycle, CardLifecycle::Item | CardLifecycle::Passive)
                        && !definition.spell
                        && definition.printed_id == printed
                })
            })
    }

    pub(in crate::interpreter) fn patch_card_is_public(
        &self,
        owner: Symbol,
        name: &str,
    ) -> Result<bool> {
        let Some(hero) = self.world.hero(owner) else {
            return Ok(false);
        };
        let commits = if self.planning.phase == Phase::Planning {
            self.observed_planning_cards(owner)?
        } else {
            (None, None)
        };
        let Some(id) = self.patch_card_instance(hero, name, commits) else {
            return Ok(false);
        };
        Ok(!hero.hand.contains(&id)
            && self.world.card(id).is_some_and(|row| !row.facedown)
            && (hero.discard.contains(&id)
                || hero.played.contains(&Some(id))
                || hero.current == Some(id)
                || hero.extra == Some(id)))
    }

    pub(in crate::interpreter) fn patch_move_card(
        &mut self,
        hero: Symbol,
        printed_id: &str,
        zone: CardPatchZone,
        slot: Option<u8>,
    ) -> Result<()> {
        self.with_live_objects(|session, objects| {
            session.sync_card_collections_bound(objects)?;
            let model = session
                .hero_model_bound(objects, hero)
                .ok_or_else(|| unsupported("card correction hero model missing"))?;
            let row = session.world.hero(hero)
                .ok_or_else(|| unsupported("card correction hero missing"))?;
            let commits = if session.planning.phase == Phase::Planning {
                session.observed_planning_cards_bound(objects, hero)?
            } else {
                (None, None)
            };
            let (first, second) = commits;
            let card = session.patch_card_instance(row, printed_id, commits)
                .ok_or_else(|| EngineError::InvalidInput("No movable card matches the supplied ID".into()))?;
            let played_slot = if zone == CardPatchZone::Played {
                let slot = if let Some(slot) = slot {
                    Some(i64::from(slot))
                } else {
                    (1..=row.resolved_turns.min(4)).find(|turn| {
                        row.card_in_turn_slot(*turn)
                            .is_none_or(|other| other == card)
                    })
                };
                Some(
                    slot.filter(|turn| {
                        *turn <= row.resolved_turns
                            && row
                                .card_in_turn_slot(*turn)
                                .is_none_or(|other| other == card)
                    })
                    .ok_or_else(|| {
                        EngineError::InvalidInput(
                            "No available played slot for a finished turn".into(),
                        )
                    })? as usize
                        - 1,
                )
            } else {
                None
            };
            // Uncommit via the regular path, so the clock, planning choices and
            // subsequent reveal agree with the physical card's new location.
            let keep = if first == Some(card) { second } else { None };
            if keep.is_some() {
                session.uncommit_card_bound(objects, hero)?;
            }
            if first == Some(card) || second == Some(card) {
                session.uncommit_card_bound(objects, hero)?;
            }

            let list = |objects: &BoundObjects, field| match objects
                .arena
                .field(model, objects.phases.k(field))
            {
                Some(ObjectValue::Object(list)) => Ok(list),
                _ => Err(unsupported("card correction list missing")),
            };
            let value = ObjectValue::Atom(Datum::Instance(card.0));
            let empty = ObjectValue::Atom(Datum::Null);
            for field in [Field::Hand, Field::Discard, Field::Played] {
                let cards = list(objects, field)?;
                let index = objects
                    .arena
                    .sequence(cards)
                    .ok_or_else(|| unsupported("card correction list invalid"))?
                    .iter()
                    .position(|other| matches!(other, ObjectValue::Atom(Datum::Instance(id)) if *id == card.0));
                if let Some(index) = index {
                    if matches!(field, Field::Played) {
                        objects
                            .arena
                            .set_index(cards, index, empty.clone())
                            .map_err(unsupported)?;
                    } else {
                        objects
                            .arena
                            .apply(&Mutation::Pop {
                                target: Target::Object(cards),
                                index: index as i64,
                            })
                            .map_err(unsupported)?;
                    }
                }
            }
            for field in [Field::Current, Field::Extra] {
                let key = objects.phases.k(field);
                if matches!(objects.arena.field(model, key), Some(ObjectValue::Atom(Datum::Instance(id))) if id == card.0) {
                    objects
                        .arena
                        .set_field(model, key, empty.clone())
                        .map_err(unsupported)?;
                }
            }

            session.swap_card_flags_bound(
                objects,
                card,
                zone.lifecycle(),
                false,
                zone == CardPatchZone::Played,
            )?;
            match zone {
                CardPatchZone::Hand | CardPatchZone::Discard => {
                    let field = if zone == CardPatchZone::Hand {
                        Field::Hand
                    } else {
                        Field::Discard
                    };
                    let cards = list(objects, field)?;
                    objects.arena.append(cards, value).map_err(unsupported)?;
                }
                CardPatchZone::Played => {
                    let played = list(objects, Field::Played)?;
                    let slot = played_slot.expect("played slot was validated");
                    let len = objects
                        .arena
                        .sequence(played)
                        .ok_or_else(|| unsupported("card correction played list invalid"))?
                        .len();
                    for _ in len..=slot {
                        objects
                            .arena
                            .append(played, empty.clone())
                            .map_err(unsupported)?;
                    }
                    objects
                        .arena
                        .set_index(played, slot, value)
                        .map_err(unsupported)?;
                }
                CardPatchZone::Deck => {
                    objects
                        .arena
                        .set_physical_field(card.0, objects.active, Datum::Bool(false))
                        .map_err(unsupported)?;
                    session
                        .world
                        .card_mut(card)
                        .ok_or_else(|| unsupported("card correction instance missing"))?
                        .active = false;
                }
            }
            session.sync_card_collections_bound(objects)?;
            session.world.refresh_card_auras();
            objects.readonly.take();
            if let Some(keep) = keep {
                session.commit_card_bound(objects, hero, keep)?;
            }
            Ok(())
        })
    }

    pub(in crate::interpreter) fn patch_swap_item_card(
        &mut self,
        hero: Symbol,
        color: &str,
        tier: &str,
    ) -> Result<()> {
        self.with_live_objects(|session, objects| {
            session.sync_card_collections_bound(objects)?;
            let row = session
                .world
                .hero(hero)
                .ok_or_else(|| unsupported("item correction hero missing"))?;
            let mut item = None;
            let mut target = None;
            for &id in &row.deck {
                let Some(card) = session.world.card(id) else {
                    continue;
                };
                let definition = &session.world.definitions.cards[card.definition];
                if definition.color.map(|color| session.symbols.name(color)) == Some(color)
                    && session.symbols.name(definition.tier) == tier
                {
                    if card.lifecycle == CardLifecycle::Item {
                        item.get_or_insert(id);
                    } else {
                        target.get_or_insert(id);
                    }
                }
            }
            let (Some(item), Some(target)) = (item, target) else {
                return Err(EngineError::InvalidInput(
                    "No item and matching upgrade in this deck".into(),
                ));
            };
            if session
                .swap_item_instances_bound(objects, hero, item, target)?
                .is_none()
            {
                return Err(EngineError::InvalidInput(
                    "The matching upgrade cannot be swapped from its current zone".into(),
                ));
            }
            if session.planning.phase == Phase::Planning {
                for field in [Field::Pending, Field::Second] {
                    let pending = objects.phases.root(&objects.arena, field)?;
                    if matches!(objects.arena.field(pending, hero), Some(ObjectValue::Atom(Datum::Instance(id))) if id == target.0)
                    {
                        objects
                            .arena
                            .set_field(pending, hero, ObjectValue::Atom(Datum::Instance(item.0)))
                            .map_err(unsupported)?;
                    }
                }
            }
            objects.readonly.take();
            Ok(())
        })
    }
}
