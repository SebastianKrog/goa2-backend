//! Retrieval stages typed list edits in source order without cloning the arena.
use super::phases::Field;
use super::{
    BoundObjects, Datum, HashMap, HashSet, NativeEvent, NativeSession, ObjectId, ObjectValue,
    Result, card_error, unsupported,
};
use crate::{
    cards::CardId,
    objects::{Mutation, Target},
    world::{CardLifecycle, NativeWorld},
};

struct Roles {
    index: usize,
    model: ObjectId,
    lists: [ObjectId; 5],
}
struct Returned {
    target: CardId,
    hand: ObjectId,
    played: ObjectId,
    played_index: Option<usize>,
    discard: ObjectId,
    discard_index: Option<usize>,
    clear_current: bool,
    effects: Vec<usize>,
}
pub(super) struct RetrievePlan {
    world: NativeWorld,
    model: ObjectId,
    rows: Vec<Returned>,
    events: Vec<NativeEvent>,
}
fn first_equal(
    world: &NativeWorld,
    values: &[Option<CardId>],
    target: CardId,
) -> Result<Option<usize>> {
    for (index, value) in values.iter().enumerate() {
        if let Some(value) = value
            && world
                .card_model_equal(*value, target)
                .map_err(unsupported)?
        {
            return Ok(Some(index));
        }
    }
    Ok(None)
}
impl NativeSession {
    fn retrieval_list_bound(
        &self,
        objects: &BoundObjects,
        model: ObjectId,
        field: Field,
    ) -> Result<ObjectId> {
        match objects.arena.field(model, objects.phases.k(field)) {
            Some(ObjectValue::Object(id)) if objects.arena.is_mutable_list(id) => Ok(id),
            _ => Err(unsupported("retrieval requires mutable card lists")),
        }
    }
    fn retrieval_roles_bound(&self, objects: &BoundObjects) -> Result<Vec<Roles>> {
        objects
            .phases
            .heroes
            .iter()
            .map(|(hero, model)| {
                let index = self
                    .world
                    .heroes
                    .iter()
                    .position(|row| row.id == *hero)
                    .ok_or_else(|| unsupported("retrieval hero cache missing"))?;
                let lists = [
                    Field::Deck,
                    Field::Hand,
                    Field::Discard,
                    Field::Spells,
                    Field::Played,
                ]
                .map(|field| self.retrieval_list_bound(objects, *model, field));
                let [deck, hand, discard, spells, played] = lists;
                Ok(Roles {
                    index,
                    model: *model,
                    lists: [deck?, hand?, discard?, spells?, played?],
                })
            })
            .collect()
    }
    fn refresh_retrieval_roles(
        world: &mut NativeWorld,
        objects: &BoundObjects,
        roles: &[Roles],
        changed: &HashMap<ObjectId, Vec<Option<CardId>>>,
        cleared: &HashSet<ObjectId>,
    ) -> Result<()> {
        for role in roles {
            let mut values = Vec::new();
            for (index, list) in role.lists.iter().enumerate() {
                let cards = if let Some(cards) = changed.get(list) {
                    cards.clone()
                } else {
                    objects
                        .arena
                        .sequence(*list)
                        .ok_or_else(|| unsupported("retrieval role list missing"))?
                        .iter()
                        .map(|v| match v {
                            ObjectValue::Atom(Datum::Instance(id)) => Ok(Some(CardId(*id))),
                            ObjectValue::Atom(Datum::Null) if index == 4 => Ok(None),
                            _ => Err(unsupported("retrieval physical role missing")),
                        })
                        .collect::<Result<Vec<_>>>()?
                };
                if index != 4 && cards.iter().any(Option::is_none) {
                    return Err(unsupported(
                        "retrieval alias creates non-card lifecycle entries",
                    ));
                }
                values.push(cards);
            }
            let row = &mut world.heroes[role.index];
            row.deck = values[0].iter().flatten().copied().collect();
            row.hand = values[1].iter().flatten().copied().collect();
            row.discard = values[2].iter().flatten().copied().collect();
            row.spells = values[3].iter().flatten().copied().collect();
            row.played = values.pop().unwrap();
            if cleared.contains(&role.model) {
                row.current = None;
            }
        }
        world.refresh_card_auras();
        Ok(())
    }
    pub(super) fn preflight_retrieve_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<RetrievePlan>> {
        let actor = if let Some(key) = self.spatial_text_bound(objects, step, Field::HeroKey)? {
            self.truthy_symbol_bound(objects, objects.arena.context_value(key))?
        } else {
            self.planning
                .actor
                .filter(|id| !self.symbols.name(*id).is_empty())
        };
        let Some(actor) = actor else {
            return Ok(None);
        };
        self.sync_card_collections_bound(objects)?;
        let Some(hero) = self.world.hero(actor) else {
            return Ok(None);
        };
        let model = self
            .hero_model_bound(objects, hero.id)
            .ok_or_else(|| unsupported("retrieval hero model missing"))?;
        let all = self.spatial_bool_bound(objects, step, Field::RetrieveAll)?;
        let targets = if all {
            hero.discard.clone()
        } else {
            let key = self.passive_text_bound(objects, step, Field::CardKey)?;
            let Some(printed) =
                self.truthy_symbol_bound(objects, objects.arena.context_value(key))?
            else {
                return Ok(None);
            };
            hero.played
                .iter()
                .flatten()
                .chain(&hero.discard)
                .chain(hero.current.iter())
                .copied()
                .find(|id| {
                    self.world.definitions.cards[self.world.card(*id).unwrap().definition]
                        .printed_id
                        == printed
                })
                .into_iter()
                .collect()
        };
        if targets.is_empty() {
            return Ok(None);
        }
        let roles = self.retrieval_roles_bound(objects)?;
        let hand = self.retrieval_list_bound(objects, model, Field::Hand)?;
        let played = self.retrieval_list_bound(objects, model, Field::Played)?;
        let discard = self.retrieval_list_bound(objects, model, Field::Discard)?;
        let mut changed = HashMap::new();
        for list in [hand, played, discard] {
            let cards = objects
                .arena
                .sequence(list)
                .unwrap()
                .iter()
                .map(|v| match v {
                    ObjectValue::Atom(Datum::Instance(id)) => Ok(Some(CardId(*id))),
                    ObjectValue::Atom(Datum::Null) => Ok(None),
                    _ => Err(unsupported("retrieval physical card missing")),
                })
                .collect::<Result<Vec<_>>>()?;
            changed.insert(list, cards);
        }
        if objects.effect_nodes()?.len() != self.world.effects.instances.len() {
            return Err(unsupported("retrieval effect collection drift"));
        }
        let mut world = self.world.staged_clone();
        let mut current = hero.current;
        let mut cleared = HashSet::new();
        let mut rows = Vec::new();
        let mut events = Vec::new();
        for target in targets {
            world.normal_card(target).map_err(card_error)?;
            if first_equal(&world, &changed[&hand], target)?.is_some() {
                return Err(unsupported(
                    "already-in-hand retrieval requires source error/partial-failure semantics",
                ));
            }
            let card = world.card(target).unwrap();
            let def = &world.definitions.cards[card.definition];
            let printed = def.printed_id;
            let name = def.name;
            world.change_lifecycle(target, CardLifecycle::Hand);
            world.card_mut(target).unwrap().facedown = false;
            changed.get_mut(&hand).unwrap().push(Some(target));
            let played_index = first_equal(&world, &changed[&played], target)?;
            if let Some(index) = played_index {
                changed.get_mut(&played).unwrap()[index] = None;
            }
            let discard_index = first_equal(&world, &changed[&discard], target)?;
            if let Some(index) = discard_index {
                changed.get_mut(&discard).unwrap().remove(index);
            }
            let clear_current = current
                .map(|id| world.card_model_equal(id, target))
                .transpose()
                .map_err(unsupported)?
                .unwrap_or(false);
            if clear_current {
                current = None;
                cleared.insert(model);
            }
            Self::refresh_retrieval_roles(&mut world, objects, &roles, &changed, &cleared)?;
            let effects = world
                .effects
                .instances
                .iter()
                .enumerate()
                .filter_map(|(i, e)| (e.card == Some(printed)).then_some(i))
                .collect();
            world.expire_effects_by_card(printed);
            rows.push(Returned {
                target,
                hand,
                played,
                played_index,
                discard,
                discard_index,
                clear_current,
                effects,
            });
            events.push(NativeEvent::CardRetrieved {
                hero: actor,
                card: printed,
                name,
                kept: None,
            });
        }
        Ok(Some(RetrievePlan {
            world,
            model,
            rows,
            events,
        }))
    }
    pub(super) fn retrieve_card_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<RetrievePlan>,
    ) -> Result<Vec<NativeEvent>> {
        let Some(plan) = plan else {
            return Ok(Vec::new());
        };
        let before = self
            .world
            .cards()
            .iter()
            .map(|c| c.active)
            .collect::<Vec<_>>();
        for row in &plan.rows {
            let physical = row.target.0;
            objects
                .arena
                .set_physical_field(
                    physical,
                    objects.phases.k(Field::CardState),
                    Datum::Text(objects.phases.k(Field::HandState)),
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
                .append(row.hand, ObjectValue::Atom(Datum::Instance(physical)))
                .map_err(unsupported)?;
            if let Some(index) = row.played_index {
                objects
                    .arena
                    .set_index(row.played, index, ObjectValue::Atom(Datum::Null))
                    .map_err(unsupported)?;
            }
            if let Some(index) = row.discard_index {
                objects
                    .arena
                    .apply(&Mutation::Pop {
                        target: Target::Object(row.discard),
                        index: index as i64,
                    })
                    .map_err(unsupported)?;
            }
            if row.clear_current {
                objects
                    .arena
                    .set_field(
                        plan.model,
                        objects.phases.k(Field::Current),
                        ObjectValue::Atom(Datum::Null),
                    )
                    .map_err(unsupported)?;
            }
            objects.remove_effect_rows(&row.effects)?;
        }
        self.world = plan.world;
        self.sync_effect_card_flags(objects, &before)?;
        self.sync_card_collections_bound(objects)?;
        Ok(plan.events)
    }
}
