//! Source discard precedence uses model equality, while mutations keep identity.
use super::phases::Field;
use super::*;
use crate::{
    cards::CardId,
    objects::{Mutation, Target},
    world::CardLifecycle,
};

enum Removal {
    Hand(ObjectId, usize),
    Current(ObjectId),
    Played(ObjectId, usize),
    None,
}
pub(super) struct DiscardPlan {
    target: CardId,
    owner: Symbol,
    raw_hero: Symbol,
    printed: Symbol,
    source: Symbol,
    removal: Removal,
    discard: ObjectId,
    log: ObjectId,
    existing_log: Option<ObjectId>,
    removed_effects: Vec<usize>,
    history: Vec<(Symbol, Vec<Symbol>)>,
}
impl NativeSession {
    fn discard_operand_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        literal: Field,
        key: Field,
    ) -> Result<Option<Symbol>> {
        let value = self.truthy_symbol_bound(
            objects,
            objects.arena.field(step, objects.phases.k(literal)),
        )?;
        if value.is_some() {
            return Ok(value);
        }
        let key =
            self.truthy_symbol_bound(objects, objects.arena.field(step, objects.phases.k(key)))?;
        key.map_or(Ok(None), |key| {
            self.truthy_symbol_bound(objects, objects.arena.context_value(key))
        })
    }
    fn discard_list_bound(
        &self,
        objects: &BoundObjects,
        model: ObjectId,
        field: Field,
    ) -> Result<ObjectId> {
        match objects.arena.field(model, objects.phases.k(field)) {
            Some(ObjectValue::Object(list)) if objects.arena.is_mutable_list(list) => Ok(list),
            _ => Err(unsupported("discard requires mutable card lists")),
        }
    }
    pub(super) fn preflight_discard_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<DiscardPlan>> {
        let Some(raw_hero) =
            self.discard_operand_bound(objects, step, Field::HeroId, Field::HeroKey)?
        else {
            return Ok(None);
        };
        self.sync_card_collections_bound(objects)?;
        let Some(hero) = self.world.hero(raw_hero) else {
            return Ok(None);
        };
        let owner = hero.id;
        let Some(printed) =
            self.discard_operand_bound(objects, step, Field::CardId, Field::CardKey)?
        else {
            return Ok(None);
        };
        let Some(ObjectValue::Atom(Datum::Text(source))) =
            objects.arena.field(step, objects.phases.k(Field::Source))
        else {
            return Err(unsupported("discard source must be a container"));
        };
        let is_printed = |id: CardId| {
            self.world.definitions.cards[self.world.card(id).unwrap().definition].printed_id
                == printed
        };
        let hand = hero.hand.iter().copied().find(|id| is_printed(*id));
        let played = hero
            .played
            .iter()
            .flatten()
            .copied()
            .find(|id| is_printed(*id))
            .or_else(|| hero.current.filter(|id| is_printed(*id)));
        let (target, from_hand) = if source == objects.phases.k(Field::PlayedContainer) {
            played
                .map(|card| (card, false))
                .or_else(|| hand.map(|card| (card, true)))
        } else {
            // HAND preferred, and DECK/DISCARD fall back to HAND then PLAYED.
            hand.map(|card| (card, true))
                .or_else(|| played.map(|card| (card, false)))
        }
        .map_or((None, false), |(card, hand)| (Some(card), hand));
        let Some(target) = target else {
            return Ok(None);
        };
        let card = self.world.card(target).unwrap();
        if self.world.definitions.cards[card.definition].spell {
            return Err(unsupported("spell cannot enter normal card lifecycle"));
        }
        let source = objects.phases.k(if from_hand {
            Field::HandState
        } else if card.lifecycle == CardLifecycle::Resolved {
            Field::PlayedContainer
        } else {
            Field::CurrentSource
        });
        let model = self
            .hero_model_bound(objects, owner)
            .ok_or_else(|| unsupported("discard hero model missing"))?;
        let removal = if from_hand {
            let mut index = None;
            for (i, id) in hero.hand.iter().enumerate() {
                if self
                    .world
                    .card_model_equal(*id, target)
                    .map_err(unsupported)?
                {
                    index = Some(i);
                    break;
                }
            }
            Removal::Hand(
                self.discard_list_bound(objects, model, Field::Hand)?,
                index.ok_or_else(|| unsupported("discard hand target missing"))?,
            )
        } else if hero
            .current
            .map(|id| self.world.card_model_equal(id, target))
            .transpose()
            .map_err(unsupported)?
            .unwrap_or(false)
        {
            Removal::Current(model)
        } else {
            let mut index = None;
            for (i, id) in hero.played.iter().enumerate() {
                if let Some(id) = id
                    && self
                        .world
                        .card_model_equal(*id, target)
                        .map_err(unsupported)?
                {
                    index = Some(i);
                    break;
                }
            }
            match index {
                Some(index) => Removal::Played(
                    self.discard_list_bound(objects, model, Field::Played)?,
                    index,
                ),
                None => Removal::None,
            }
        };
        let discard = self.discard_list_bound(objects, model, Field::Discard)?;
        let log = objects.phases.root(&objects.arena, Field::DiscardLog)?;
        if objects.arena.kind(log) != ObjectKind::Dict {
            return Err(unsupported("discard log must be a dictionary"));
        }
        let existing_log = match objects.arena.field(log, owner) {
            None => None,
            Some(ObjectValue::Object(list)) if objects.arena.is_mutable_list(list) => Some(list),
            _ => return Err(unsupported("discard log entry must be a list")),
        };
        // Multiple hero log entries may share the same physical list.
        let mut history = Vec::new();
        for (hero, value) in objects.arena.fields(log).unwrap() {
            let ObjectValue::Object(list) = value else {
                return Err(unsupported("discard history entry must be a list"));
            };
            let mut cards = objects
                .arena
                .sequence(*list)
                .ok_or_else(|| unsupported("discard history list missing"))?
                .iter()
                .map(|value| match value {
                    ObjectValue::Atom(Datum::Text(card)) => Ok(*card),
                    _ => Err(unsupported("discard history identity must be text")),
                })
                .collect::<Result<Vec<_>>>()?;
            if Some(*list) == existing_log {
                cards.push(printed);
            }
            history.push((*hero, cards));
        }
        if existing_log.is_none() {
            history.push((owner, vec![printed]));
        }
        let revealed = objects.phases.root(&objects.arena, Field::Revealed)?;
        if objects.arena.sequence(revealed).is_none() {
            return Err(unsupported("revealed-card tuple missing"));
        }
        let nodes = objects.effect_nodes()?;
        if nodes.len() != self.world.effects.instances.len() {
            return Err(unsupported("discard effect collection binding drift"));
        }
        let removed_effects = self
            .world
            .effects
            .instances
            .iter()
            .enumerate()
            .filter_map(|(i, e)| (e.card == Some(printed)).then_some(i))
            .collect();
        self.phase_generation_check(0, 1)?;
        Ok(Some(DiscardPlan {
            target,
            owner,
            raw_hero,
            printed,
            source,
            removal,
            discard,
            log,
            existing_log,
            removed_effects,
            history,
        }))
    }
    pub(super) fn discard_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<DiscardPlan>,
    ) -> Result<()> {
        let Some(plan) = plan else {
            return Ok(());
        };
        match plan.removal {
            Removal::Hand(list, index) => objects
                .arena
                .apply(&Mutation::Pop {
                    target: Target::Object(list),
                    index: index as i64,
                })
                .map_err(unsupported)?,
            Removal::Played(list, index) => objects
                .arena
                .set_index(list, index, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?,
            Removal::Current(model) => objects
                .arena
                .set_field(
                    model,
                    objects.phases.k(Field::Current),
                    ObjectValue::Atom(Datum::Null),
                )
                .map_err(unsupported)?,
            Removal::None => {}
        }
        self.world
            .change_lifecycle(plan.target, CardLifecycle::Discard);
        let row = self.world.card_mut(plan.target).unwrap();
        row.facedown = false;
        let physical = row.source_instance;
        objects
            .arena
            .set_physical_field(
                physical,
                objects.phases.k(Field::CardState),
                Datum::Text(objects.phases.k(Field::DiscardState)),
            )
            .map_err(unsupported)?;
        objects
            .arena
            .set_physical_field(
                physical,
                objects.phases.k(Field::Facedown),
                Datum::Bool(false),
            )
            .map_err(unsupported)?;
        objects
            .arena
            .remove_physical_field(physical, objects.phases.k(Field::InitiativeOverride))
            .map_err(unsupported)?;
        objects
            .arena
            .append(plan.discard, ObjectValue::Atom(Datum::Instance(physical)))
            .map_err(unsupported)?;
        self.sync_card_collections_bound(objects)?;
        self.reveal_knowledge_bound(objects, plan.owner, plan.printed)?;
        let log = if let Some(list) = plan.existing_log {
            list
        } else {
            let list = objects.arena.list(Vec::new());
            objects
                .arena
                .set_field(plan.log, plan.owner, list.clone())
                .map_err(unsupported)?;
            let ObjectValue::Object(list) = list else {
                unreachable!()
            };
            list
        };
        objects
            .arena
            .append(log, ObjectValue::Atom(Datum::Text(plan.printed)))
            .map_err(unsupported)?;
        self.world.history.discarded = plan.history;
        let before = self
            .world
            .cards
            .iter()
            .map(|card| card.active)
            .collect::<Vec<_>>();
        self.world.expire_effects_by_card(plan.printed);
        // Replacement is required even for zero matches; old aliases detach.
        objects.remove_effect_rows(&plan.removed_effects)?;
        self.sync_effect_card_flags(objects, &before)?;
        for (field, value) in [
            (Field::DiscardedId, plan.printed),
            (Field::DiscardedOwner, plan.owner),
            (Field::DiscardSource, plan.source),
        ] {
            objects
                .arena
                .set_context(
                    objects.phases.k(field),
                    ObjectValue::Atom(Datum::Text(value)),
                )
                .map_err(unsupported)?;
        }
        let passive = self.phase_step(objects, "passive")?;
        for (field, value) in [
            (Field::HeroId, plan.raw_hero),
            (Field::Trigger, objects.phases.k(Field::AfterDiscard)),
        ] {
            objects
                .arena
                .set_field(
                    passive,
                    objects.phases.k(field),
                    ObjectValue::Atom(Datum::Text(value)),
                )
                .map_err(unsupported)?;
        }
        self.push_live_steps_bound(objects, &[passive])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_log_mutation_refreshes_every_hero_history_cache() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let index: Value = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(base.join("object-execution.json")).unwrap(),
        ))
        .unwrap();
        let entry = index["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|case| case["name"] == "object-execution-discard-log-shared")
            .unwrap();
        let case: Value = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(base.join(entry["path"].as_str().unwrap())).unwrap(),
        ))
        .unwrap();
        let mut session =
            NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                .unwrap();
        session.advance_prefix(1).unwrap();
        let arien = session.symbols.lookup("hero_arien").unwrap();
        let hanu = session.symbols.lookup("hero_hanu").unwrap();
        let history = &session.world.history.discarded;
        let a = &history.iter().find(|(hero, _)| *hero == arien).unwrap().1;
        let b = &history.iter().find(|(hero, _)| *hero == hanu).unwrap().1;
        assert_eq!(a.len(), 2);
        assert_eq!(a, b);
        let restored = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
        let named = |session: &NativeSession| {
            session
                .world
                .history
                .discarded
                .iter()
                .map(|(hero, cards)| {
                    (
                        session.symbols.name(*hero).to_owned(),
                        cards
                            .iter()
                            .map(|card| session.symbols.name(*card).to_owned())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(named(&session), named(&restored));
    }
}
