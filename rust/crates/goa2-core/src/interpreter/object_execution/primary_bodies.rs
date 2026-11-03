//! Registered primary text uses alias-aware cold builders and live card bindings.
use super::passive_bodies::Recipe;
use super::phases::Field;
use super::*;
use crate::world::ActionKind;
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub(super) struct PrimaryRules {
    methods: HashMap<Symbol, Symbol>,
    pub(super) spell_bodies: Arc<super::spell_bodies::SpellBodies>,
    spells: HashSet<Symbol>,
    recipes: HashMap<Symbol, Vec<Recipe>>,
    variants: HashMap<Symbol, StatefulPrimary>,
    ranged: HashMap<Symbol, Vec<usize>>,
    fallback: HashMap<&'static str, Vec<Recipe>>,
    empty: Symbol,
    card: Symbol,
    defense_log: Symbol,
    skill_log: Symbol,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum PrimaryMode {
    Ordinary,
    Copied,
    AfterAttack,
}
#[derive(Clone, Debug)]
enum StatefulPrimary {
    MisaUltimate([Vec<Recipe>; 2]),
    MinRetired {
        variants: [Vec<Recipe>; 2],
        operand: Symbol,
    },
}
fn catalog() -> Result<&'static Value> {
    static DATA: OnceLock<std::result::Result<Value, String>> = OnceLock::new();
    DATA.get_or_init(|| {
        let data: Value = serde_json::from_str(include_str!(
            "../../../../../tests/fixtures/primary-builders.json"
        ))
        .map_err(|e| e.to_string())?;
        if data["schema_version"] != 3
            || data["source_sha256"] != crate::codec::oracle_manifest()?["source_sha256"]
        {
            return Err("primary builder source identity mismatch".into());
        }
        Ok(data)
    })
    .as_ref()
    .map_err(|error| unsupported(error.clone()))
}
impl PrimaryRules {
    pub(super) fn registered(&self, effect: Symbol, spell: bool) -> bool {
        if spell {
            self.spells.contains(&effect)
        } else {
            self.methods.contains_key(&effect)
        }
    }

    pub(super) fn compile(steps: &mut Vec<CompiledStep>, symbols: &mut Symbols) -> Result<Self> {
        let data = catalog()?;
        let mut methods = HashMap::new();
        for (effect, method) in data["methods"]
            .as_object()
            .ok_or_else(|| unsupported("primary methods missing"))?
        {
            methods.insert(
                symbols.intern(effect),
                symbols.intern(
                    method
                        .as_str()
                        .ok_or_else(|| unsupported("primary method missing"))?,
                ),
            );
        }
        let spells = data["spell_effects"]
            .as_array()
            .ok_or_else(|| unsupported("spell registry identity missing"))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(|s| symbols.intern(s))
                    .ok_or_else(|| unsupported("spell registry name missing"))
            })
            .collect::<Result<HashSet<_>>>()?;
        let mut recipes = HashMap::new();
        for (effect, rows) in data["recipes"]
            .as_object()
            .ok_or_else(|| unsupported("primary recipes missing"))?
        {
            recipes.insert(
                symbols.intern(effect),
                rows.as_array()
                    .ok_or_else(|| unsupported("primary recipe rows missing"))?
                    .iter()
                    .map(|row| Recipe::compile(row, steps, symbols))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        let variants = data["stateful_recipes"]
            .as_object()
            .ok_or_else(|| unsupported("stateful primary recipes missing"))?
            .iter()
            .map(|(effect, body)| {
                let mut rows = |name: &str| {
                    body["variants"][name]
                        .as_array()
                        .ok_or_else(|| unsupported("stateful primary variant missing"))?
                        .iter()
                        .map(|row| Recipe::compile(row, steps, symbols))
                        .collect::<Result<Vec<_>>>()
                };
                let kind = match body["predicate"].as_str() {
                    Some("misa_has_ultimate") => StatefulPrimary::MisaUltimate([
                        rows("without_ultimate")?,
                        rows("with_ultimate")?,
                    ]),
                    Some("min_has_retired_tier_two") => {
                        let variants = [
                            rows("without_retired_tier_two")?,
                            rows("with_retired_tier_two")?,
                        ];
                        let operand = symbols.intern(
                            body["retired_card_operand"]
                                .as_str()
                                .ok_or_else(|| unsupported("retired-card operand missing"))?,
                        );
                        StatefulPrimary::MinRetired { variants, operand }
                    }
                    _ => return Err(unsupported("unknown stateful primary predicate")),
                };
                Ok((symbols.intern(effect), kind))
            })
            .collect::<Result<HashMap<_, _>>>()?;
        let mut ranged = HashMap::new();
        for (effect, rows) in data["ranged_steps"]
            .as_object()
            .ok_or_else(|| unsupported("primary ranged bindings missing"))?
        {
            ranged.insert(
                symbols.intern(effect),
                rows.as_array()
                    .ok_or_else(|| unsupported("primary ranged rows missing"))?
                    .iter()
                    .map(|row| {
                        row.as_u64()
                            .and_then(|n| usize::try_from(n).ok())
                            .ok_or_else(|| unsupported("primary ranged index invalid"))
                    })
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        if data["radius_fallbacks"]
            != crate::primary::catalog().map_err(unsupported)?["radius_fallbacks"]
        {
            return Err(unsupported("primary radius fallback catalogs disagree"));
        }
        let mut fallback = HashMap::new();
        for name in ["movement", "attack", "defense", "skill"] {
            fallback.insert(
                name,
                data["fallback"][name]
                    .as_array()
                    .ok_or_else(|| unsupported("primary fallback missing"))?
                    .iter()
                    .map(|row| Recipe::compile(row, steps, symbols))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        Ok(Self {
            methods,
            spell_bodies: Arc::new(super::spell_bodies::SpellBodies::compile(steps, symbols)?),
            spells,
            recipes,
            variants,
            ranged,
            fallback,
            empty: symbols.intern("CardEffect.build_steps"),
            card: symbols.intern("template-primary-card"),
            defense_log: symbols.intern("template-primary-defense-log"),
            skill_log: symbols.intern("template-primary-skill-log"),
        })
    }
}
#[derive(Default)]
pub(super) struct PrimaryPlan {
    pub(super) owner: Option<Symbol>,
    pub(super) card: Option<Symbol>,
    pub(super) recipes: Vec<Recipe>,
}
impl NativeSession {
    fn misa_has_ultimate_bound(&self, objects: &BoundObjects, owner: Symbol) -> Result<bool> {
        let model = self
            .hero_model_bound(objects, owner)
            .ok_or_else(|| unsupported("Misa performer model missing"))?;
        let ultimate = objects
            .arena
            .field(model, objects.phases.k(Field::Ultimate))
            .ok_or_else(|| unsupported("Misa ultimate field missing"))?;
        if matches!(ultimate, ObjectValue::Atom(Datum::Null)) {
            return Ok(false);
        }
        let level = match objects.arena.field(model, objects.phases.k(Field::Level)) {
            Some(ObjectValue::Atom(Datum::Int(value))) => value,
            Some(ObjectValue::Atom(Datum::Bool(value))) => i64::from(value),
            _ => return Err(unsupported("Misa performer level must be numeric")),
        };
        Ok(level >= 8)
    }
    pub(super) fn preflight_primary_text_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<PrimaryPlan> {
        let hero = self.passive_text_bound(objects, step, Field::HeroId)?;
        let Some(model) = self.hero_model_bound(objects, hero) else {
            return Ok(PrimaryPlan::default());
        };
        let Some(card) = self.model_card_bound(objects, model, Field::Current)? else {
            return Ok(PrimaryPlan::default());
        };
        self.sync_card_collections_bound(objects)?;
        self.prepare_primary_body_bound(objects, hero, card, None)
    }
    pub(super) fn prepare_primary_body_bound(
        &mut self,
        objects: &BoundObjects,
        hero: Symbol,
        card: crate::cards::CardId,
        computed: Option<CardStats>,
    ) -> Result<PrimaryPlan> {
        self.prepare_primary_body_kind_bound(objects, hero, card, computed, PrimaryMode::Ordinary)
    }
    pub(super) fn prepare_copied_body_bound(
        &mut self,
        objects: &BoundObjects,
        hero: Symbol,
        card: crate::cards::CardId,
    ) -> Result<PrimaryPlan> {
        let stats = self.world.card_stats(hero, card).map_err(unsupported)?;
        self.prepare_primary_body_kind_bound(objects, hero, card, Some(stats), PrimaryMode::Copied)
    }
    pub(super) fn prepare_deferred_primary_bound(
        &mut self,
        objects: &BoundObjects,
        hero: Symbol,
        card: crate::cards::CardId,
    ) -> Result<PrimaryPlan> {
        let stats = self.world.card_stats(hero, card).map_err(unsupported)?;
        self.prepare_primary_body_kind_bound(
            objects,
            hero,
            card,
            Some(stats),
            PrimaryMode::AfterAttack,
        )
    }
    fn prepare_primary_body_kind_bound(
        &mut self,
        objects: &BoundObjects,
        hero: Symbol,
        card: crate::cards::CardId,
        computed: Option<CardStats>,
        mode: PrimaryMode,
    ) -> Result<PrimaryPlan> {
        let copied = mode == PrimaryMode::Copied;
        let mut plan = PrimaryPlan::default();
        let instance = self
            .world
            .card(card)
            .ok_or_else(|| unsupported("primary current card missing"))?;
        let definition = self.world.definitions.cards[instance.definition].clone();
        let owner = self.world.hero(hero).unwrap().id;
        plan.owner = Some(owner);
        plan.card = Some(definition.printed_id);
        if instance.facedown || (definition.effect_absent && !copied) {
            return Ok(plan);
        }
        let rules = objects.scheduler.as_ref().unwrap().primary_bodies.clone();
        let spell = mode != PrimaryMode::AfterAttack
            && definition.spell
            && rules.spells.contains(&definition.effect);
        let registered = if spell {
            None
        } else {
            rules.methods.get(&definition.effect)
        };
        let mut stats = CardStats {
            primary_value: 0,
            range: 0,
            radius: None,
        };
        let mut logs = None;
        let mut retired_binding = None;
        plan.recipes = if spell {
            stats = if let Some(stats) = computed {
                stats
            } else {
                self.world.card_stats(owner, card).map_err(unsupported)?
            };
            self.prepare_spell_recipes_bound(objects, owner, card, &mut stats)?
        } else if let Some(method) = registered {
            stats = if let Some(stats) = computed {
                stats
            } else {
                self.world.card_stats(owner, card).map_err(unsupported)?
            };
            if *method == rules.empty {
                Vec::new()
            } else if let Some(kind) = rules.variants.get(&definition.effect) {
                match kind {
                    StatefulPrimary::MisaUltimate(variants) => {
                        variants[usize::from(self.misa_has_ultimate_bound(objects, owner)?)].clone()
                    }
                    StatefulPrimary::MinRetired { variants, operand } => {
                        let ids = self
                            .world
                            .hero(owner)
                            .unwrap()
                            .deck
                            .iter()
                            .filter_map(|id| {
                                let card = self.world.card(*id).unwrap();
                                let definition = &self.world.definitions.cards[card.definition];
                                (card.lifecycle == crate::world::CardLifecycle::Retired
                                    && self.symbols.name(definition.tier) == "II")
                                    .then_some(definition.printed_id)
                            })
                            .collect::<Vec<_>>();
                        let variant = usize::from(!ids.is_empty());
                        retired_binding = Some((*operand, ids));
                        variants[variant].clone()
                    }
                }
            } else {
                rules
                    .recipes
                    .get(&definition.effect)
                    .ok_or_else(|| {
                        unsupported(format!(
                            "primary body is not yet bound: {}",
                            self.symbols.name(definition.effect)
                        ))
                    })?
                    .clone()
            }
        } else if copied {
            stats = computed.ok_or_else(|| unsupported("copied primary stats missing"))?;
            if stats.range == 0 {
                stats.range = 1;
            }
            match definition.primary {
                Some(ActionKind::Movement) => rules.fallback["movement"].clone(),
                Some(ActionKind::Attack) => rules.fallback["attack"].clone(),
                _ => Vec::new(),
            }
        } else {
            let inputs = self.spatial_inputs_bound(
                objects,
                [
                    objects.phases.k(Field::PerformingCard),
                    objects.phases.k(Field::PerformingOwner),
                ],
            );
            match definition.primary {
                Some(ActionKind::Movement) => {
                    stats.primary_value = self
                        .world
                        .computed_stat_in_context(
                            hero,
                            Stat::Movement,
                            definition.primary_value.unwrap_or(0),
                            &inputs,
                        )
                        .map_err(unsupported)?;
                    rules.fallback["movement"].clone()
                }
                Some(ActionKind::Attack) => {
                    stats.primary_value = self
                        .world
                        .computed_stat_in_context(
                            hero,
                            Stat::Attack,
                            definition.primary_value.unwrap_or(0),
                            &inputs,
                        )
                        .map_err(unsupported)?;
                    stats.range = self
                        .world
                        .computed_stat_in_context(
                            hero,
                            Stat::Range,
                            definition.range.filter(|v| *v != 0).unwrap_or(1),
                            &inputs,
                        )
                        .map_err(unsupported)?;
                    rules.fallback["attack"].clone()
                }
                Some(ActionKind::Defense) => {
                    logs = Some((
                        rules.defense_log,
                        format!("{} Defends (Primary).", self.symbols.name(hero)),
                    ));
                    rules.fallback["defense"].clone()
                }
                Some(ActionKind::Skill) => {
                    logs = Some((
                        rules.skill_log,
                        format!(
                            "Skill '{}' did nothing.",
                            self.symbols.name(definition.name)
                        ),
                    ));
                    rules.fallback["skill"].clone()
                }
                _ => Vec::new(),
            }
        };
        let log = logs.map(|(key, text)| (key, Arc::make_mut(&mut self.symbols).intern(&text)));
        fn bind(
            value: &mut Datum,
            rules: &PrimaryRules,
            card: Symbol,
            stats: &CardStats,
            log: Option<(Symbol, Symbol)>,
            operands: &crate::primary::PrimaryOperands,
            retired: Option<(Symbol, &[Symbol])>,
        ) {
            match value {
                Datum::Text(v) if *v == rules.card => *v = card,
                Datum::Text(v) if log.is_some_and(|(key, _)| key == *v) => *v = log.unwrap().1,
                Datum::Int(v) if *v == crate::primary::PRIMARY => *v = stats.primary_value,
                Datum::Int(v) if *v == crate::primary::RANGE => *v = stats.range,
                Datum::Int(v) if *v == crate::primary::RADIUS => {
                    *value = stats.radius.map_or(Datum::Null, Datum::Int)
                }
                Datum::Int(v) => {
                    if let Some(default) = operands.radius_default(*v, stats.radius) {
                        *v = default;
                    }
                }
                Datum::List(values) => {
                    if let Some((marker, ids)) = retired
                        && values.len() == 1
                        && matches!(values[0], Datum::Text(value) if value == marker)
                    {
                        *values = ids.iter().copied().map(Datum::Text).collect();
                    } else {
                        for value in values {
                            bind(value, rules, card, stats, log, operands, retired);
                        }
                    }
                }
                Datum::Object(fields) => {
                    for (_, value) in fields {
                        bind(value, rules, card, stats, log, operands, retired);
                    }
                }
                _ => {}
            }
        }
        for (index, recipe) in plan.recipes.iter_mut().enumerate() {
            bind(
                &mut recipe.step.fields,
                &rules,
                definition.printed_id,
                &stats,
                log,
                &self.primary_operands,
                retired_binding
                    .as_ref()
                    .map(|(marker, ids)| (*marker, ids.as_slice())),
            );
            if registered.is_some()
                && rules
                    .ranged
                    .get(&definition.effect)
                    .is_some_and(|rows| rows.contains(&index))
            {
                let Datum::Object(fields) = &mut recipe.step.fields else {
                    return Err(unsupported("primary root recipe missing"));
                };
                let key = self
                    .symbols
                    .lookup("is_ranged")
                    .ok_or_else(|| unsupported("primary ranged field missing"))?;
                let Some((_, value)) = fields.iter_mut().find(|(k, _)| *k == key) else {
                    return Err(unsupported("primary ranged operand missing"));
                };
                *value = Datum::Bool(definition.ranged);
            }
        }
        let count = plan
            .recipes
            .iter()
            .flat_map(|recipe| &recipe.objects)
            .filter(|(_, kind)| *kind == ObjectKind::Step)
            .count();
        if mode != PrimaryMode::AfterAttack && count != 0 {
            self.phase_generation_check(
                0,
                u64::try_from(count)
                    .map_err(|_| unsupported("primary generation count overflow"))?,
            )?;
        }
        Ok(plan)
    }
    pub(super) fn primary_text_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: PrimaryPlan,
    ) -> Result<()> {
        objects.arena.symbols = self.symbols.clone();
        if let Some(card) = plan.card {
            objects.write(objects.action_keys[1], Datum::Text(card))?;
        }
        let Some(owner) = plan.owner else {
            return Ok(());
        };
        let mut queued = Vec::with_capacity(plan.recipes.len());
        for recipe in &plan.recipes {
            queued.push(self.allocate_source_recipe_bound(objects, recipe, &Datum::Text(owner))?);
        }
        self.push_live_steps_bound(objects, &queued)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(name: &str) -> NativeSession {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let index: Value = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(base.join("object-execution.json")).unwrap(),
        ))
        .unwrap();
        let entry = index["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap();
        let case: Value = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(base.join(entry["path"].as_str().unwrap())).unwrap(),
        ))
        .unwrap();
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    fn primary_generation_guard_preserves_current_context_stack_and_aliases() {
        let mut worker = session("object-execution-primary-text-liquid_leap-ordinary");
        worker.next_step = u64::MAX - 1;
        let before = worker.export_envelope().unwrap();
        let depth = worker.stack.len();
        assert!(
            worker
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("step generation counter exhausted")
        );
        assert_eq!(worker.stack.len(), depth);
        let after = worker.export_envelope().unwrap();
        assert_eq!(before.state, after.state);
        assert_eq!(before.object_bindings, after.object_bindings);
    }
    #[test]
    fn empty_primary_builder_needs_no_generated_ids() {
        let mut worker = session("object-execution-primary-text-empty-dragon_knight");
        worker.next_step = u64::MAX;
        worker.advance_prefix(1).unwrap();
        assert_eq!(worker.next_step, u64::MAX);
        assert_eq!(worker.stack.len(), 1);
    }
    #[test]
    fn primary_hero_operand_is_live_and_private_to_a_worker() {
        let root = session("object-execution-primary-text-liquid_leap-ordinary");
        let before = root.export_envelope().unwrap();
        let mut changed = root.clone();
        let hero = changed.symbols.lookup("hero_hanu").unwrap();
        let step = changed.stack.last().unwrap().object.unwrap();
        let objects = changed.objects.as_mut().unwrap();
        objects
            .arena
            .set_field(
                step,
                objects.phases.k(Field::HeroId),
                ObjectValue::Atom(Datum::Text(hero)),
            )
            .unwrap();
        changed.advance_prefix(1).unwrap();
        let after = changed.export_envelope().unwrap();
        assert_eq!(
            after.state["execution_context"]["current_card_id"],
            "old-card"
        );
        assert_eq!(changed.stack.len(), 1);
        assert_eq!(root.export_envelope().unwrap().state, before.state);
    }
    #[test]
    fn generated_primary_loops_bind_nested_programs_and_unique_step_ids() {
        let mut worker = session("object-execution-primary-text-bullet_hell-ordinary");
        worker.advance_prefix(1).unwrap();
        let objects = worker.objects.as_ref().unwrap();
        let step=worker.stack.iter().filter_map(|frame|frame.object).find(|id|matches!(objects.arena.field(*id,objects.phases.k(Field::FilterType)),Some(ObjectValue::Atom(Datum::Text(kind))) if worker.symbols.name(kind)=="for_each")).unwrap();
        let children = objects.template_objects(step).unwrap();
        assert!(!children.is_empty());
        let mut ids = HashSet::new();
        for child in children {
            assert!(objects.program(child).is_some());
            let Some(ObjectValue::GeneratedStep(id)) = objects
                .arena
                .field(child, objects.scheduler.as_ref().unwrap().step_id)
            else {
                panic!("nested step ID missing");
            };
            assert!(ids.insert(id));
        }
    }
}
