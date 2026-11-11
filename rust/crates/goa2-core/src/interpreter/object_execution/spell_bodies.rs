//! Namespaced spell builders: immutable prototypes plus explicit live state operands.
use super::passive_bodies::Recipe;
use super::phases::Field;
use super::*;
use crate::{cards::CardId, stats::CardStats, world::CardLifecycle};
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub(super) struct SpellBodies {
    recipes: HashMap<Symbol, Vec<Recipe>>,
    counted: Symbol,
}
impl SpellBodies {
    pub(super) fn compile(steps: &mut Vec<CompiledStep>, symbols: &mut Symbols) -> Result<Self> {
        static DATA: OnceLock<std::result::Result<Value, String>> = OnceLock::new();
        let data = DATA
            .get_or_init(|| {
                let data: Value = serde_json::from_str(include_str!(
                    "../../../../../content/catalogs/spell-builders.json"
                ))
                .map_err(|e| e.to_string())?;
                if data["schema_version"] != 2
                    || data["source_sha256"] != crate::codec::oracle_manifest()?["source_sha256"]
                {
                    return Err("spell builder source identity mismatch".into());
                }
                Ok(data)
            })
            .as_ref()
            .map_err(|e| unsupported(e.clone()))?;
        let mut recipes = HashMap::new();
        for (effect, rows) in data["recipes"]
            .as_object()
            .ok_or_else(|| unsupported("spell recipes missing"))?
        {
            recipes.insert(
                symbols.intern(effect),
                rows.as_array()
                    .ok_or_else(|| unsupported("spell rows missing"))?
                    .iter()
                    .map(|row| Recipe::compile_primary(row, steps, symbols))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        Ok(Self {
            recipes,
            counted: symbols.intern("wish_counted"),
        })
    }
}
fn replace(recipe: &mut Recipe, key: Symbol, value: Datum) -> Result<()> {
    let Datum::Object(fields) = &mut recipe.step.fields else {
        return Err(unsupported("spell root fields missing"));
    };
    let (_, slot) = fields
        .iter_mut()
        .find(|(k, _)| *k == key)
        .ok_or_else(|| unsupported("spell prototype operand missing"))?;
    *slot = value;
    Ok(())
}
impl NativeSession {
    fn outside_spells_bound(&self, objects: &BoundObjects, printed: Symbol) -> Result<i64> {
        let Some(owner) = self.spellbook_owner_bound()? else {
            return Ok(0);
        };
        let count = self
            .world
            .hero(owner)
            .unwrap()
            .spells
            .iter()
            .filter(|id| {
                let card = self.world.card(**id).unwrap();
                self.spell_has_state_bound(
                    objects,
                    **id,
                    Field::OutsideSpellbookState,
                    CardLifecycle::OutsideSpellbook,
                ) && self.world.definitions.cards[card.definition].printed_id != printed
            })
            .count();
        i64::try_from(count).map_err(|_| unsupported("outside spell count outside i64"))
    }
    pub(super) fn prepare_spell_recipes_bound(
        &mut self,
        objects: &BoundObjects,
        hero: Symbol,
        card: CardId,
        stats: &mut CardStats,
    ) -> Result<Vec<Recipe>> {
        let definition =
            self.world.definitions.cards[self.world.card(card).unwrap().definition].clone();
        let effect = self.symbols.name(definition.effect).to_owned();
        let rules = objects
            .scheduler
            .as_ref()
            .unwrap()
            .primary_bodies
            .spell_bodies
            .clone();
        let counted = effect == "wish"
            && matches!(objects.arena.context_value(objects.phases.k(Field::WishCountedCaster)),Some(ObjectValue::Atom(Datum::Text(id))) if id==hero);
        let mut recipes = rules
            .recipes
            .get(&if counted {
                rules.counted
            } else {
                definition.effect
            })
            .ok_or_else(|| unsupported("spell primary body is not yet bound"))?
            .clone();
        match effect.as_str() {
            "create_undead" => {
                let model = self
                    .hero_model_bound(objects, hero)
                    .ok_or_else(|| unsupported("spell caster model missing"))?;
                let Some(team) =
                    self.passive_optional_text_bound(objects, model, Field::ResourceTeam)?
                else {
                    return Ok(Vec::new());
                };
                replace(
                    &mut recipes[0],
                    objects.phases.k(Field::ResourceTeam),
                    Datum::Text(team),
                )?;
            }
            "midas_touch" => {
                let amount = self.outside_spells_bound(objects, definition.printed_id)? / 2;
                if amount == 0 {
                    return Ok(Vec::new());
                }
                replace(
                    &mut recipes[1],
                    objects.phases.k(Field::Amount),
                    Datum::Int(amount),
                )?;
            }
            "dimension_door" => {
                let radius = stats
                    .radius
                    .unwrap_or(0)
                    .checked_add(self.outside_spells_bound(objects, definition.printed_id)?)
                    .ok_or_else(|| unsupported("spell placement distance outside i64"))?;
                stats.radius = Some(radius);
                let prompt = Arc::make_mut(&mut self.symbols)
                    .intern(&format!("Place yourself exactly {radius} spaces away"));
                replace(
                    &mut recipes[0],
                    objects.phases.k(Field::Prompt),
                    Datum::Text(prompt),
                )?;
            }
            "sunburst" => {
                let count = self.outside_spells_bound(objects, definition.printed_id)?;
                stats.primary_value = stats
                    .primary_value
                    .checked_add(count)
                    .ok_or_else(|| unsupported("Sunburst attack outside i64"))?;
                stats.range = stats
                    .range
                    .checked_add(count)
                    .ok_or_else(|| unsupported("Sunburst range outside i64"))?;
            }
            "wish" if counted => {
                let allowed = if let Some(owner) = self.spellbook_owner_bound()? {
                    self.prepared_spells_bound(objects, owner)
                        .into_iter()
                        .map(|id| {
                            self.world.definitions.cards[self.world.card(id).unwrap().definition]
                                .printed_id
                        })
                        .filter(|id| *id != objects.phases.k(Field::Wish))
                        .map(Datum::Text)
                        .collect()
                } else {
                    Vec::new()
                };
                replace(
                    &mut recipes[0],
                    objects.phases.k(Field::AllowedSpellIds),
                    Datum::List(allowed),
                )?;
            }
            _ => {}
        }
        Ok(recipes)
    }
}
