//! Registered defense/on-block construction with source-derived cold recipes.
use super::passive_bodies::Recipe;
use super::phases::Field;
use super::{
    Arc, BoundObjects, CompiledStep, Datum, HashMap, HashSet, Instruction, NativeSession,
    ObjectKind, ObjectValue, Result, Symbol, Symbols, Value, unsupported,
};
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug)]
enum Hook {
    Default,
    Fixed,
    Bounty,
    Spawn,
    Smoke,
    Crowd,
    Oath,
    Ranged,
    Melee,
}
#[derive(Clone, Debug)]
struct Method {
    hook: Hook,
    on_block: bool,
    primary: Symbol,
}
#[derive(Clone, Debug)]
struct Oath {
    blocks: Vec<(Symbol, Symbol)>,
    choose: bool,
}
#[derive(Clone, Debug)]
pub(super) struct DefenseRules {
    methods: HashMap<Symbol, Method>,
    recipes: HashMap<String, Vec<Recipe>>,
    oaths: HashMap<Symbol, Oath>,
    card: Symbol,
    range: i64,
    radius: i64,
    bonus: i64,
    runes: [Symbol; 4],
}
fn catalog() -> Result<&'static Value> {
    static DATA: OnceLock<std::result::Result<Value, String>> = OnceLock::new();
    DATA.get_or_init(|| {
        let data: Value = serde_json::from_str(include_str!(
            "../../../../../content/catalogs/defense-builders.json"
        ))
        .map_err(|error| error.to_string())?;
        let manifest = crate::codec::oracle_manifest()?;
        if data["schema_version"] != 1 || data["source_sha256"] != manifest["source_sha256"] {
            return Err("defense catalog source identity mismatch".into());
        }
        Ok(data)
    })
    .as_ref()
    .map_err(|error| unsupported(error.clone()))
}
fn hook(name: &str) -> Result<Hook> {
    Ok(match name {
        "CardEffect.build_defense_steps" => Hook::Default,
        "AspiringDuelistEffect.build_defense_steps"
        | "ExpertDuelistEffect.build_defense_steps"
        | "MasterDuelistEffect.build_defense_steps"
        | "UnstableTimelineEffect.build_defense_steps" => Hook::Fixed,
        "PerfectGetawayEffect.build_defense_steps"
        | "CloseCallEffect.build_defense_steps"
        | "NarrowEscapeEffect.build_defense_steps" => Hook::Bounty,
        "_SpawnPointDefenseEffect.build_defense_steps" => Hook::Spawn,
        "PoofEffect.build_defense_steps"
        | "VanishEffect.build_defense_steps"
        | "RuseEffect.build_defense_steps" => Hook::Smoke,
        "CrowdControlEffect.build_defense_steps" => Hook::Crowd,
        "OathEffect.build_defense_steps" => Hook::Oath,
        "DodgeEffect.build_defense_steps"
        | "SidestepEffect.build_defense_steps"
        | "EvadeEffect.build_defense_steps"
        | "StopProjectilesEffect.build_defense_steps"
        | "ReflectProjectilesEffect.build_defense_steps"
        | "DeflectProjectilesEffect.build_defense_steps" => Hook::Ranged,
        "ParryEffect.build_defense_steps" | "RiposteEffect.build_defense_steps" => Hook::Melee,
        _ => return Err(unsupported("unknown defense builder method")),
    })
}
impl DefenseRules {
    pub fn compile(steps: &mut Vec<CompiledStep>, symbols: &mut Symbols) -> Result<Self> {
        let data = catalog()?;
        let mut methods = HashMap::new();
        for (effect, raw) in data["methods"]
            .as_object()
            .ok_or_else(|| unsupported("defense methods missing"))?
        {
            let on = raw["on_block"]
                .as_str()
                .ok_or_else(|| unsupported("on-block method missing"))?;
            let on_block = match on {
                "CardEffect.build_on_block_steps" => false,
                "OathEffect.build_on_block_steps"
                | "SidestepEffect.build_on_block_steps"
                | "ParryEffect.build_on_block_steps"
                | "RiposteEffect.build_on_block_steps"
                | "EvadeEffect.build_on_block_steps"
                | "ReflectProjectilesEffect.build_on_block_steps"
                | "DeflectProjectilesEffect.build_on_block_steps" => true,
                _ => return Err(unsupported("unknown on-block builder method")),
            };
            methods.insert(
                symbols.intern(effect),
                Method {
                    hook: hook(
                        raw["defense"]
                            .as_str()
                            .ok_or_else(|| unsupported("defense method missing"))?,
                    )?,
                    on_block,
                    primary: symbols.intern(
                        raw["primary"]
                            .as_str()
                            .ok_or_else(|| unsupported("primary method missing"))?,
                    ),
                },
            );
        }
        let mut recipes = HashMap::new();
        for (name, rows) in data["recipes"]
            .as_object()
            .ok_or_else(|| unsupported("defense recipes missing"))?
        {
            recipes.insert(
                name.clone(),
                rows.as_array()
                    .ok_or_else(|| unsupported("defense recipe list missing"))?
                    .iter()
                    .map(|row| Recipe::compile(row, steps, symbols))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        let mut oaths = HashMap::new();
        for (name, raw) in data["runes"]
            .as_object()
            .ok_or_else(|| unsupported("defense rune metadata missing"))?
        {
            let mut blocks = Vec::new();
            for (rune, block) in raw["blocks"]
                .as_object()
                .ok_or_else(|| unsupported("defense rune block mapping missing"))?
            {
                blocks.push((
                    symbols.intern(rune),
                    symbols.intern(
                        block
                            .as_str()
                            .ok_or_else(|| unsupported("defense rune block type missing"))?,
                    ),
                ));
            }
            oaths.insert(
                symbols.intern(name),
                Oath {
                    blocks,
                    choose: raw["choose_one"]
                        .as_bool()
                        .ok_or_else(|| unsupported("defense rune choice flag missing"))?,
                },
            );
        }
        for name in [
            "_pre_defense_actor",
            "_pre_onblock_actor",
            "_discard",
            "CardEffect.build_steps",
            "AwakenEffect.build_steps",
            "bounty",
            "snorri_ult_rune_action",
            "snorri_ult_rune_defense",
            "options",
            "matching_options",
        ] {
            symbols.intern(name);
        }
        Ok(Self {
            methods,
            recipes,
            oaths,
            card: symbols.intern(
                data["bindings"]["card"]
                    .as_str()
                    .ok_or_else(|| unsupported("defense card binding missing"))?,
            ),
            range: data["bindings"]["range"]
                .as_i64()
                .ok_or_else(|| unsupported("defense range binding missing"))?,
            radius: data["bindings"]["radius"]
                .as_i64()
                .ok_or_else(|| unsupported("defense radius binding missing"))?,
            bonus: data["bindings"]["bonus"]
                .as_i64()
                .ok_or_else(|| unsupported("defense bonus binding missing"))?,
            runes: ["axe", "bird", "anvil", "horn"].map(|name| symbols.intern(name)),
        })
    }
}
fn k(symbols: &Symbols, name: &str) -> Symbol {
    symbols.lookup(name).expect("compiled defense field")
}
pub(super) struct DefensePlan {
    owner: Symbol,
    card: Symbol,
    recipes: Vec<Recipe>,
    range: i64,
    radius: i64,
    bonus: i64,
    choice: Option<(Vec<Symbol>, Vec<Symbol>)>,
    on_block: bool,
}
impl NativeSession {
    fn defense_truthy(&self, objects: &BoundObjects, field: Field) -> bool {
        objects
            .arena
            .context_value(objects.phases.k(field))
            .is_some_and(|value| objects.arena.truthy(&value))
    }
    fn defense_spawn_bonus(&self, owner: Symbol, radius: i64) -> Result<bool> {
        let Some(origin) = self.world.resolved_position(owner) else {
            return Ok(false);
        };
        let all = self.world.tide_of_darkness(Some(owner));
        let mut count = 0;
        for &point in self.world.tile_hexes() {
            if !all
                && (!self.world.spawn_at(point, false) || !self.world.tile_in_battle_zone(point))
            {
                continue;
            }
            if self.world.obstacle(point)
                || i64::from(origin.distance(point)) > radius
                || !self
                    .world
                    .effect_connected(origin, point, &[])
                    .map_err(unsupported)?
            {
                continue;
            }
            if !self
                .world
                .obstacle_for_actor(point, Some(owner))
                .map_err(unsupported)?
            {
                count += 1;
                if count >= 2 {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    fn defense_runes(
        &self,
        objects: &BoundObjects,
        printed: Symbol,
        rules: &DefenseRules,
    ) -> Result<Vec<Symbol>> {
        let mut active = HashSet::new();
        // Source owner lookup checks current/extra, then hand/deck/discard/played,
        // then ultimate, in global hero order; it is independent of the attacker.
        let owner = self.world.heroes.iter().find(|hero| {
            hero.current
                .into_iter()
                .chain(hero.extra)
                .chain(hero.hand.iter().copied())
                .chain(hero.deck.iter().copied())
                .chain(hero.discard.iter().copied())
                .chain(hero.played.iter().flatten().copied())
                .chain(hero.ultimate)
                .any(|id| {
                    self.world.card(id).is_some_and(|card| {
                        self.world.definitions.cards[card.definition].printed_id == printed
                    })
                })
        });
        if let Some(owner) = owner
            && let Some(model) = self.hero_model_bound(objects, owner.id)
        {
            let Some(ObjectValue::Object(slots)) = objects
                .arena
                .field(model, objects.phases.k(Field::RuneSlots))
            else {
                return Err(unsupported("defense rune slots missing"));
            };
            if let Some(turn) = self.symbols.lookup(&self.world.combat.turn.to_string())
                && let Some(value) = objects.arena.field(slots, turn)
            {
                let ObjectValue::Atom(Datum::Text(rune)) = value else {
                    return Err(unsupported("defense turn rune must be text"));
                };
                if !rules.runes.contains(&rune) {
                    return Err(unsupported("invalid defense turn rune"));
                }
                active.insert(rune);
            }
        }
        for name in ["snorri_ult_rune_action", "snorri_ult_rune_defense"] {
            if let Some(value) = objects
                .arena
                .context_value(k(&self.symbols, name))
                .filter(|value| objects.arena.truthy(value))
            {
                let ObjectValue::Atom(Datum::Text(rune)) = value else {
                    return Err(unsupported("defense ultimate rune must be text"));
                };
                if !rules.runes.contains(&rune) {
                    return Err(unsupported("invalid defense ultimate rune"));
                }
                active.insert(rune);
            }
        }
        Ok(rules
            .runes
            .into_iter()
            .filter(|rune| active.contains(rune))
            .collect())
    }
    pub(super) fn preflight_defense_bound(
        &mut self,
        objects: &BoundObjects,
        on_block: bool,
    ) -> Result<Option<DefensePlan>> {
        if on_block && !self.defense_truthy(objects, Field::BlockSucceeded)
            || !self.defense_truthy(objects, Field::PrimaryDefense)
        {
            return Ok(None);
        }
        let Some(card_value) = objects
            .arena
            .context_value(objects.phases.k(Field::DefenseCardId))
            .filter(|value| objects.arena.truthy(value))
        else {
            return Ok(None);
        };
        let Some(value) = objects
            .arena
            .context_value(objects.phases.k(Field::DefenderId))
            .filter(|value| objects.arena.truthy(value))
        else {
            return Ok(None);
        };
        let raw = objects
            .arena
            .typed_value(&value, Arc::make_mut(&mut self.symbols))
            .map_err(unsupported)?;
        let text = raw
            .python_scalar_text(&self.symbols)
            .ok_or_else(|| unsupported("defender Python conversion not bound"))?;
        let defender = Arc::make_mut(&mut self.symbols).intern(&text);
        let Some(owner) = self.world.hero(defender).map(|hero| hero.id) else {
            return Ok(None);
        };
        if !on_block
            && self.world.position(defender).is_none()
            && !self.world.has_board_presence(defender)
        {
            return Ok(None);
        }
        let ObjectValue::Atom(Datum::Text(printed)) = card_value else {
            return Ok(None);
        };
        self.sync_card_collections_bound(objects)?;
        let hero = self
            .world
            .hero(owner)
            .ok_or_else(|| unsupported("defense owner cache missing"))?;
        let Some(card) = hero.hand.iter().chain(&hero.discard).copied().find(|id| {
            self.world.card(*id).is_some_and(|card| {
                self.world.definitions.cards[card.definition].printed_id == printed
            })
        }) else {
            return Ok(None);
        };
        let instance = self
            .world
            .card(card)
            .ok_or_else(|| unsupported("defense card cache missing"))?;
        let definition = &self.world.definitions.cards[instance.definition];
        if instance.facedown || !definition.has_effect {
            return Ok(None);
        }
        let effect = definition.effect;
        let base_radius = definition.radius;
        let rules = objects.scheduler.as_ref().unwrap().defense_bodies.clone();
        let Some(method) = rules.methods.get(&effect) else {
            return Ok(None);
        };
        // Both hooks compute attacked-piece stats before inspecting their body.
        let stats = self.world.card_stats(defender, card).map_err(unsupported)?;
        let mut range = stats.range;
        let mut radius = stats.radius.unwrap_or(0);
        let mut bonus = 0;
        let mut choice = None;
        let (hook, variant) = if on_block {
            if !method.on_block {
                return Ok(None);
            }
            if matches!(
                self.symbols.name(effect),
                "reflect_projectiles" | "deflect_projectiles"
            ) && range == 0
            {
                range = 3;
            }
            ("on_block", "default")
        } else {
            let variant = match method.hook {
                Hook::Default => {
                    // A None defense hook invokes primary construction with
                    // owner-based stats, even when the primary body is empty.
                    let fallback = self.world.card_stats(owner, card).map_err(unsupported)?;
                    range = fallback.range;
                    radius = fallback.radius.unwrap_or(0);
                    if method.primary == k(&self.symbols, "CardEffect.build_steps") {
                        return Ok(None);
                    }
                    if method.primary != k(&self.symbols, "AwakenEffect.build_steps") {
                        return Err(unsupported("primary fallback body is not yet bound"));
                    }
                    if fallback.radius.is_none() {
                        return Err(unsupported("awaken card requires a radius"));
                    }
                    "default"
                }
                Hook::Fixed => "default",
                Hook::Smoke => {
                    if self.world.resolved_position(owner).is_none() {
                        "no-origin"
                    } else {
                        "default"
                    }
                }
                Hook::Bounty => {
                    let markers = objects.phases.root(&objects.arena, Field::Markers)?;
                    let placed = match objects.arena.field(markers, k(&self.symbols, "bounty")) {
                        None => false,
                        Some(ObjectValue::Object(marker)) => !matches!(
                            objects
                                .arena
                                .field(marker, objects.phases.k(Field::TargetId)),
                            None | Some(ObjectValue::Atom(Datum::Null))
                        ),
                        _ => return Err(unsupported("defense bounty marker model missing")),
                    };
                    if placed { "yes" } else { "no" }
                }
                Hook::Spawn => {
                    if self.defense_spawn_bonus(owner, radius)? {
                        "yes"
                    } else {
                        "no"
                    }
                }
                Hook::Crowd => {
                    let Some(origin) = self.world.resolved_position(defender) else {
                        return Ok(None);
                    };
                    radius = stats
                        .radius
                        .filter(|value| *value != 0)
                        .or(base_radius.filter(|value| *value != 0))
                        .unwrap_or(3);
                    for piece in self.world.piece_ids(owner) {
                        if piece == defender {
                            continue;
                        }
                        if let Some(point) = self.world.resolved_position(piece)
                            && i64::from(origin.distance(point)) <= radius
                            && self
                                .world
                                .effect_connected(origin, point, &[])
                                .map_err(unsupported)?
                        {
                            bonus += 2;
                        }
                    }
                    if bonus == 0 {
                        return Ok(None);
                    }
                    "default"
                }
                Hook::Oath => {
                    let oath = rules
                        .oaths
                        .get(&effect)
                        .ok_or_else(|| unsupported("defense oath rule missing"))?;
                    let active = self.defense_runes(objects, printed, &rules)?;
                    let eligible = active
                        .into_iter()
                        .filter(|rune| oath.blocks.iter().any(|(value, _)| value == rune))
                        .collect::<Vec<_>>();
                    let basic = self.defense_truthy(objects, Field::AttackBasic);
                    let ranged = self.defense_truthy(objects, Field::AttackRanged);
                    let matching = eligible
                        .iter()
                        .copied()
                        .filter(|rune| {
                            oath.blocks.iter().any(|(value, block)| {
                                value == rune
                                    && match self.symbols.name(*block) {
                                        "basic" => basic,
                                        "melee" => !ranged,
                                        "ranged" => ranged,
                                        "non_basic" => !basic,
                                        _ => false,
                                    }
                            })
                        })
                        .collect::<Vec<_>>();
                    if oath.choose && eligible.len() > 1 {
                        choice = Some((eligible, matching));
                        "choose"
                    } else if !matching.is_empty() {
                        "match"
                    } else {
                        "none"
                    }
                }
                Hook::Ranged | Hook::Melee => {
                    if self.defense_truthy(objects, Field::AttackRanged) {
                        "ranged"
                    } else {
                        "melee"
                    }
                }
            };
            (
                if matches!(method.hook, Hook::Default) {
                    "primary"
                } else {
                    "defense"
                },
                variant,
            )
        };
        let name = format!("{}/{hook}/{variant}", self.symbols.name(effect));
        let recipes = rules
            .recipes
            .get(&name)
            .ok_or_else(|| unsupported(format!("defense recipe missing: {name}")))?
            .clone();
        if recipes.is_empty() {
            return Ok(None);
        }
        let count = recipes
            .iter()
            .flat_map(|recipe| &recipe.objects)
            .filter(|(_, kind)| *kind == ObjectKind::Step)
            .count();
        self.phase_generation_check(
            0,
            u64::try_from(count)
                .ok()
                .and_then(|value| value.checked_add(2))
                .ok_or_else(|| unsupported("defense generation count overflow"))?,
        )?;
        Ok(Some(DefensePlan {
            owner,
            card: printed,
            recipes,
            range,
            radius,
            bonus,
            choice,
            on_block,
        }))
    }
    pub(super) fn defense_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<DefensePlan>,
    ) -> Result<()> {
        let Some(plan) = plan else {
            return Ok(());
        };
        let rules = objects.scheduler.as_ref().unwrap().defense_bodies.clone();
        fn bind(value: &mut Datum, rules: &DefenseRules, plan: &DefensePlan) {
            match value {
                Datum::Text(value) if *value == rules.card => *value = plan.card,
                Datum::Int(value) => {
                    if *value == rules.range {
                        *value = plan.range;
                    } else if *value == rules.radius {
                        *value = plan.radius;
                    } else if *value == rules.bonus {
                        *value = plan.bonus;
                    }
                }
                Datum::List(values) => {
                    for value in values {
                        bind(value, rules, plan);
                    }
                }
                Datum::Object(fields) => {
                    for (_, value) in fields {
                        bind(value, rules, plan);
                    }
                }
                _ => {}
            }
        }
        let saved = k(
            &self.symbols,
            if plan.on_block {
                "_pre_onblock_actor"
            } else {
                "_pre_defense_actor"
            },
        );
        let enter = self.phase_step(objects, "set_actor")?;
        let mut queued = vec![enter];
        for recipe in &plan.recipes {
            let mut recipe = recipe.clone();
            bind(&mut recipe.step.fields, &rules, &plan);
            let step =
                self.allocate_source_recipe_bound(objects, &recipe, &Datum::Text(plan.owner))?;
            if let Some((options, matching)) = &plan.choice
                && matches!(
                    self.steps[recipe.step.instruction].instruction,
                    Instruction::LiveRuneChoice
                )
            {
                for (field, values) in [("options", options), ("matching_options", matching)] {
                    let list = objects.arena.list(
                        values
                            .iter()
                            .map(|value| ObjectValue::Atom(Datum::Text(*value)))
                            .collect(),
                    );
                    objects
                        .arena
                        .set_field(step, k(&self.symbols, field), list)
                        .map_err(unsupported)?;
                }
            }
            queued.push(step);
        }
        let exit = self.phase_step(objects, "set_actor")?;
        for (step, actor, save) in [
            (enter, objects.phases.k(Field::DefenderId), saved),
            (exit, saved, k(&self.symbols, "_discard")),
        ] {
            for (field, value) in [(Field::ActorKey, actor), (Field::SaveKey, save)] {
                objects
                    .arena
                    .set_field(
                        step,
                        objects.phases.k(field),
                        ObjectValue::Atom(Datum::Text(value)),
                    )
                    .map_err(unsupported)?;
            }
        }
        queued.push(exit);
        objects.arena.symbols = self.symbols.clone();
        self.push_live_steps_bound(objects, &queued)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{EngineError, fixture_case};
    use super::*;
    fn session(name: &str) -> NativeSession {
        let case = fixture_case(name);
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    fn defense_generation_guards_leave_actor_stack_and_models_unchanged() {
        for name in ["expert_duelist", "unstable_timeline", "awaken"] {
            let mut worker = session(&format!("object-execution-defense-body-{name}-text-first"));
            worker.next_step = u64::MAX - 2;
            let before = worker.export_envelope().unwrap();
            assert!(
                matches!(worker.advance_prefix(1),Err(EngineError::Unsupported(message)) if message.contains("generation counter exhausted"))
            );
            let after = worker.export_envelope().unwrap();
            assert_eq!(after.state, before.state);
            assert_eq!(after.object_bindings, before.object_bindings);
        }
    }
    #[test]
    fn generated_arithmetic_reads_live_amounts_in_private_workers() {
        let case = fixture_case("object-execution-defense-body-crowd_control-text-first");
        let expected=case["expected"].as_array().unwrap().last().unwrap()["semantic_state"]["execution_context"]["defense_bonus"].as_i64().unwrap();
        let mut root = session("object-execution-defense-body-crowd_control-text-first");
        root.advance_prefix(1).unwrap();
        let before = root.export_envelope().unwrap();
        let mut ordinary = root.clone();
        let mut changed = root.clone();
        let field = k(&changed.symbols, "type");
        let objects = changed.objects.as_ref().unwrap();
        let step=changed.stack.iter().filter_map(|frame|frame.object).find(|step|matches!(objects.arena.field(*step,field),Some(ObjectValue::Atom(Datum::Text(value))) if changed.symbols.name(value)=="add_context_value")).unwrap();
        let objects = changed.objects.as_mut().unwrap();
        objects
            .arena
            .set_field(
                step,
                objects.phases.k(Field::Amount),
                ObjectValue::Atom(Datum::Int(9)),
            )
            .unwrap();
        for (worker, expected) in [(&mut ordinary, expected), (&mut changed, 9)] {
            worker.advance_prefix(3).unwrap();
            let objects = worker.objects.as_ref().unwrap();
            assert!(
                matches!(objects.arena.context_value(worker.symbols.lookup("defense_bonus").unwrap()),Some(ObjectValue::Atom(Datum::Int(value))) if value==expected)
            );
            assert_eq!(worker.planning.actor, worker.symbols.lookup("hero_brogan"));
        }
        let after = root.export_envelope().unwrap();
        assert_eq!(after.state, before.state);
        assert_eq!(after.object_bindings, before.object_bindings);
    }
    #[test]
    fn glitch_recipes_keep_shared_filters_and_register_all_stored_programs() {
        let mut worker = session("object-execution-defense-body-unstable_timeline-text-first");
        worker.advance_prefix(1).unwrap();
        let objects = worker.objects.as_ref().unwrap();
        let field = k(&worker.symbols, "type");
        let find = |name| {
            worker.stack.iter().filter_map(|frame|frame.object).find(|step|matches!(objects.arena.field(*step,field),Some(ObjectValue::Atom(Datum::Text(value))) if worker.symbols.name(value)==name)).unwrap()
        };
        let batch = find("place_token_batch");
        let ObjectValue::Object(slots) = objects
            .arena
            .field(batch, k(&worker.symbols, "slot_filters"))
            .unwrap()
        else {
            panic!("slots missing")
        };
        let mut shared = None;
        for slot in objects.arena.sequence(slots).unwrap() {
            let ObjectValue::Object(slot) = slot else {
                panic!("slot missing")
            };
            let ObjectValue::Object(filter) = objects.arena.sequence(*slot).unwrap()[0] else {
                panic!("filter missing")
            };
            if let Some(original) = shared {
                assert_eq!(filter, original);
            } else {
                shared = Some(filter);
            }
        }
        let cleanup = find("create_effect");
        let ObjectValue::Object(finishing) =
            objects.arena.field(cleanup, objects.finishing).unwrap()
        else {
            panic!("finishing missing")
        };
        let roots = objects.arena.sequence(finishing).unwrap();
        for root in roots {
            let ObjectValue::Object(root) = root else {
                panic!("root missing")
            };
            assert!(objects.program(*root).is_some());
        }
        let ObjectValue::Object(loop_step) = roots[1] else {
            panic!("loop missing")
        };
        let ObjectValue::Object(children) =
            objects.arena.field(loop_step, objects.templates).unwrap()
        else {
            panic!("children missing")
        };
        for child in objects.arena.sequence(children).unwrap() {
            let ObjectValue::Object(child) = child else {
                panic!("child missing")
            };
            assert!(objects.program(*child).is_some());
        }
    }
}
