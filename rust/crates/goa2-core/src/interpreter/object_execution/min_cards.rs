//! Deferred red-card text and permanent-item conversion retain live physical data.
use super::passive_bodies::Recipe;
use super::phases::Field;
use super::*;
use crate::{cards::CardId, stats::Stat, world::CardLifecycle};

pub(super) struct ItemPlan {
    actor: Symbol,
    card: CardId,
    physical: u32,
    printed: Symbol,
    items: ObjectId,
    stat: Stat,
    stat_key: Symbol,
    value: i64,
    bonuses: [i64; 6],
    order: Vec<Stat>,
    owners: Vec<usize>,
}
pub(super) struct AfterAttackPlan {
    owner: Symbol,
    recipes: Vec<Recipe>,
    keys: Vec<Symbol>,
    defender: Option<ObjectValue>,
}
impl NativeSession {
    pub(super) fn preflight_convert_item_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<ItemPlan>> {
        let key = self.passive_text_bound(objects, step, Field::CardKey)?;
        let Some(value) = objects
            .arena
            .context_value(key)
            .filter(|v| objects.arena.truthy(v))
        else {
            return Ok(None);
        };
        let printed = self.resource_identity_bound(objects, value)?;
        let actor = self
            .passive_optional_text_bound(objects, step, Field::HeroId)?
            .filter(|id| !self.symbols.name(*id).is_empty())
            .or(self.planning.actor);
        let Some(actor) = actor else {
            return Ok(None);
        };
        self.sync_card_collections_bound(objects)?;
        let Some(hero) = self.world.hero(actor) else {
            return Ok(None);
        };
        let owner = hero.id;
        let card = hero.deck.iter().copied().find(|id| {
            self.world.definitions.cards[self.world.card(*id).unwrap().definition].printed_id
                == printed
        });
        let Some(card) = card else {
            return Ok(None);
        };
        let instance = self.world.card(card).unwrap();
        let Some(stat) = self.world.definitions.cards[instance.definition].item else {
            return Ok(None);
        };
        let physical = instance.source_instance;
        let stat_key = self
            .symbols
            .lookup(stat.name())
            .ok_or_else(|| unsupported("item stat symbol missing"))?;
        let model = self
            .hero_model_bound(objects, owner)
            .ok_or_else(|| unsupported("item owner model missing"))?;
        let Some(ObjectValue::Object(items)) =
            objects.arena.field(model, objects.phases.k(Field::Items))
        else {
            return Err(unsupported("item bonus mapping missing"));
        };
        if objects.arena.kind(items) != ObjectKind::Dict {
            return Err(unsupported("item bonuses require mutable dictionary"));
        }
        let mut bonuses = [0; 6];
        let mut order = Vec::new();
        for (key, value) in objects
            .arena
            .fields(items)
            .ok_or_else(|| unsupported("item bonus fields missing"))?
        {
            let stat = Stat::parse(self.symbols.name(*key)).map_err(unsupported)?;
            bonuses[stat.index()] = match value {
                ObjectValue::Atom(Datum::Int(value)) => *value,
                ObjectValue::Atom(Datum::Bool(value)) => i64::from(*value),
                _ => return Err(unsupported("item bonus must be an integer")),
            };
            order.push(stat);
        }
        let value = bonuses[stat.index()]
            .checked_add(1)
            .ok_or_else(|| unsupported("item bonus outside i64"))?;
        bonuses[stat.index()] = value;
        if !order.contains(&stat) {
            order.push(stat);
        }
        let mut owners = Vec::new();
        for (hero, model) in &objects.phases.heroes {
            if matches!(objects.arena.field(*model,objects.phases.k(Field::Items)),Some(ObjectValue::Object(id)) if id == items)
            {
                owners.push(
                    self.world
                        .heroes
                        .iter()
                        .position(|row| row.id == *hero)
                        .ok_or_else(|| unsupported("shared item owner missing"))?,
                );
            }
        }
        Ok(Some(ItemPlan {
            actor,
            card,
            physical,
            printed,
            items,
            stat,
            stat_key,
            value,
            bonuses,
            order,
            owners,
        }))
    }
    pub(super) fn convert_item_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<ItemPlan>,
    ) -> Result<Option<NativeEvent>> {
        let Some(plan) = plan else {
            return Ok(None);
        };
        objects
            .arena
            .set_field(
                plan.items,
                plan.stat_key,
                ObjectValue::Atom(Datum::Int(plan.value)),
            )
            .map_err(unsupported)?;
        objects
            .arena
            .set_physical_field(
                plan.physical,
                objects.phases.k(Field::CardState),
                Datum::Text(objects.phases.k(Field::ItemState)),
            )
            .map_err(unsupported)?;
        // The source state setter clears temporary initiative, even on repeated conversion.
        objects
            .arena
            .remove_physical_field(plan.physical, objects.phases.k(Field::InitiativeOverride))
            .map_err(unsupported)?;
        self.world.change_lifecycle(plan.card, CardLifecycle::Item);
        for owner in plan.owners {
            self.world.heroes[owner].items = plan.bonuses;
            self.world.heroes[owner].items_order = plan.order.clone();
        }
        self.world.refresh_card_auras();
        Ok(Some(NativeEvent::ItemGained {
            actor: plan.actor,
            stat: plan.stat,
            card: plan.printed,
        }))
    }
    pub(super) fn preflight_after_attack_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<AfterAttackPlan>> {
        let hero = self.passive_text_bound(objects, step, Field::HeroId)?;
        self.sync_card_collections_bound(objects)?;
        let Some(hero) = self.world.hero(hero) else {
            return Ok(None);
        };
        let owner = hero.id;
        let card = hero
            .played
            .iter()
            .flatten()
            .chain(&hero.discard)
            .copied()
            .find(|id| {
                let instance = self.world.card(*id).unwrap();
                !instance.facedown
                    && self.world.definitions.cards[instance.definition]
                        .color
                        .is_some_and(|color| self.symbols.name(color) == "RED")
            });
        let Some(card) = card else {
            return Ok(None);
        };
        let definition = &self.world.definitions.cards[self.world.card(card).unwrap().definition];
        let rules = objects.scheduler.as_ref().unwrap().primary_bodies.clone();
        if !rules.registered(definition.effect, false) {
            return Ok(None);
        }
        let plan = self.prepare_deferred_primary_bound(objects, owner, card)?;
        let is_attack = |recipe: &Recipe| {
            matches!(
                self.steps[recipe.step.instruction].instruction,
                Instruction::LiveAttackSequence
            )
        };
        let mut keys = Vec::new();
        for recipe in plan.recipes.iter().filter(|r| is_attack(r)) {
            let Datum::Object(fields) = &recipe.step.fields else {
                return Err(unsupported("after-attack recipe root missing"));
            };
            for name in ["target_id_key", "target_output_key"] {
                let Some(key) = self.symbols.lookup(name) else {
                    continue;
                };
                match fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v) {
                    None | Some(Datum::Null) => {}
                    Some(Datum::Text(value)) if self.symbols.name(*value).is_empty() => {}
                    Some(Datum::Text(value)) => keys.push(*value),
                    _ => return Err(unsupported("after-attack target context key invalid")),
                }
            }
            if !keys.is_empty() {
                break;
            }
        }
        let defender = objects
            .arena
            .context_value(objects.phases.k(Field::DefenderId))
            .filter(|v| objects.arena.truthy(v));
        let recipes = if let Some(index) = plan.recipes.iter().position(is_attack) {
            plan.recipes.into_iter().skip(index + 1).collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let count = recipes
            .iter()
            .flat_map(|r| &r.objects)
            .filter(|(_, kind)| *kind == ObjectKind::Step)
            .count();
        if count != 0 {
            self.phase_generation_check(
                0,
                u64::try_from(count)
                    .map_err(|_| unsupported("after-attack generation count overflow"))?,
            )?;
        }
        Ok(Some(AfterAttackPlan {
            owner,
            recipes,
            keys,
            defender,
        }))
    }
    pub(super) fn after_attack_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<AfterAttackPlan>,
    ) -> Result<()> {
        let Some(plan) = plan else {
            return Ok(());
        };
        objects.arena.symbols = self.symbols.clone();
        if let Some(defender) = plan.defender {
            for key in plan.keys {
                objects
                    .arena
                    .set_context(key, defender.clone())
                    .map_err(unsupported)?;
            }
        }
        let mut queued = Vec::with_capacity(plan.recipes.len());
        for recipe in &plan.recipes {
            queued.push(self.allocate_source_recipe_bound(
                objects,
                recipe,
                &Datum::Text(plan.owner),
            )?);
        }
        self.push_live_steps_bound(objects, &queued)
    }
}
