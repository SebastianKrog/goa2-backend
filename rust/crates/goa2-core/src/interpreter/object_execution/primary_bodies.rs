//! Registered primary text uses alias-aware cold builders and live card bindings.
use super::passive_bodies::Recipe;
use super::phases::Field;
use super::*;
use crate::{objects::ObjectSlot, world::ActionKind};
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
    Reperformed,
    Copied,
    AfterAttack,
    Passive { effect: Symbol, raw: bool },
}
#[derive(Clone, Debug)]
enum StatefulPrimary {
    LevelEightUltimate {
        variants: [Vec<Recipe>; 2],
        required: Option<Symbol>,
    },
    SnorriRunes(Vec<Vec<Recipe>>),
    IgnatiaCoin([Vec<Recipe>; 6]),
    FaceupSkills(Vec<Recipe>),
    TopologyAnchor(Vec<Recipe>),
    ActingPiece(Vec<Recipe>),
    ResolvedOrReperformed([Vec<Recipe>; 2]),
    RightmostPlayedCard([Vec<Recipe>; 2]),
    RowennaLance([Vec<Recipe>; 2]),
    DodgerRespawn([Vec<Recipe>; 2]),
    DodgerRitual(Vec<Vec<Recipe>>),
    CardInDiscard([Vec<Recipe>; 2]),
    UrsafarEnraged {
        variants: [Vec<Recipe>; 2],
        primary_bonus: i64,
    },
    TaliWinterWeapon {
        base: Vec<Recipe>,
        bonus: i64,
    },
    DodgerDreadRazor([Vec<Recipe>; 2]),
    DodgerDeathTrap {
        variants: [Vec<Recipe>; 2],
        operand: Symbol,
    },
    BountyMovement {
        base: Vec<Recipe>,
        bonus: i64,
        marker: Symbol,
    },
    WhisperTrail(Vec<Vec<Recipe>>),
    TakahideDeckGold([Vec<Recipe>; 2]),
    EmmittTurnDiscards {
        base: Vec<Recipe>,
        group: Vec<Recipe>,
        insert_at: usize,
    },
    MinRetired {
        variants: [Vec<Recipe>; 2],
        operand: Symbol,
    },
}
pub(super) fn catalog() -> Result<&'static Value> {
    static DATA: OnceLock<std::result::Result<Value, String>> = OnceLock::new();
    DATA.get_or_init(|| {
        let data: Value = serde_json::from_str(include_str!(
            "../../../../../content/catalogs/primary-builders.json"
        ))
        .map_err(|e| e.to_string())?;
        if data["schema_version"] != 4
            || data["source_sha256"] != crate::codec::oracle_manifest()?["source_sha256"]
        {
            return Err("primary builder source identity mismatch".into());
        }
        Ok(data)
    })
    .as_ref()
    .map_err(|error| unsupported(error.clone()))
}
fn attach_raw_bindings(recipe: &mut Recipe, paths: &Value, symbols: &mut Symbols) -> Result<()> {
    for path in paths
        .as_array()
        .ok_or_else(|| unsupported("raw primary bindings missing"))?
    {
        let path = path
            .as_array()
            .ok_or_else(|| unsupported("raw primary path missing"))?;
        let mut slots = Vec::new();
        for part in path {
            slots.push(if let Some(key) = part.as_str() {
                ObjectSlot::Field(symbols.intern(key))
            } else {
                ObjectSlot::Index(
                    part.as_u64()
                        .and_then(|v| usize::try_from(v).ok())
                        .ok_or_else(|| unsupported("raw primary path invalid"))?,
                )
            });
        }
        recipe.raw_card_bindings.push(slots);
    }
    Ok(())
}
fn clear_raw_card_bindings(recipe: &mut Recipe) -> Result<()> {
    for path in &recipe.raw_card_bindings {
        let mut value = &mut recipe.step.fields;
        for slot in path {
            value = match (value, slot) {
                (Datum::Object(fields), ObjectSlot::Field(key)) => {
                    fields.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
                }
                (Datum::List(values), ObjectSlot::Index(index)) => values.get_mut(*index),
                _ => None,
            }
            .ok_or_else(|| unsupported("raw primary binding path drift"))?;
        }
        *value = Datum::Null;
    }
    Ok(())
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
                    .enumerate()
                    .map(|(index, row)| {
                        let mut recipe = Recipe::compile_primary(row, steps, symbols)?;
                        attach_raw_bindings(
                            &mut recipe,
                            &data["raw_card_bindings"][effect][index],
                            symbols,
                        )?;
                        Ok(recipe)
                    })
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
                        .enumerate()
                        .map(|(index, row)| {
                            let mut recipe = Recipe::compile_primary(row, steps, symbols)?;
                            attach_raw_bindings(
                                &mut recipe,
                                &body["raw_card_bindings"][name][index],
                                symbols,
                            )?;
                            Ok(recipe)
                        })
                        .collect::<Result<Vec<_>>>()
                };
                let kind = match body["predicate"].as_str() {
                    Some("misa_has_ultimate" | "has_level_eight_ultimate") => {
                        let variants = [rows("without_ultimate")?, rows("with_ultimate")?];
                        StatefulPrimary::LevelEightUltimate {
                            variants,
                            required: body["required_ultimate"]
                                .as_str()
                                .map(|id| symbols.intern(id)),
                        }
                    }
                    Some("bounty_movement") => StatefulPrimary::BountyMovement {
                        base: rows("base")?,
                        bonus: body["bonus"]
                            .as_i64()
                            .ok_or_else(|| unsupported("Bounty movement bonus missing"))?,
                        marker: symbols.intern("bounty"),
                    },
                    Some("dodger_death_trap") => {
                        StatefulPrimary::DodgerDeathTrap {
                            variants: [rows("all")?, rows("excluded")?],
                            operand: symbols.intern(body["operand"].as_str().ok_or_else(|| {
                                unsupported("Death Trap exclusion operand missing")
                            })?),
                        }
                    }
                    Some("dodger_dread_razor") => {
                        StatefulPrimary::DodgerDreadRazor([rows("adjacent_only")?, rows("choice")?])
                    }
                    Some("acting_piece") => StatefulPrimary::ActingPiece(rows("body")?),
                    Some("topology_anchor") => StatefulPrimary::TopologyAnchor(rows("body")?),
                    Some("faceup_skills") => StatefulPrimary::FaceupSkills(rows("body")?),
                    Some("ignatia_coin") => StatefulPrimary::IgnatiaCoin([
                        rows("blue")?,
                        rows("orange")?,
                        rows("equilibrium")?,
                        rows("blue_ultimate")?,
                        rows("orange_ultimate")?,
                        rows("equilibrium_ultimate")?,
                    ]),
                    Some("ursafar_enraged") => StatefulPrimary::UrsafarEnraged {
                        variants: [rows("calm")?, rows("enraged")?],
                        primary_bonus: body["primary_bonus"].as_i64().unwrap_or(0),
                    },
                    Some("card_in_discard") => {
                        StatefulPrimary::CardInDiscard([rows("played")?, rows("discarded")?])
                    }
                    Some("tali_winter_weapon") => StatefulPrimary::TaliWinterWeapon {
                        base: rows("base")?,
                        bonus: body["bonus"]
                            .as_i64()
                            .ok_or_else(|| unsupported("Winter weapon bonus missing"))?,
                    },
                    Some("dodger_ritual") => {
                        StatefulPrimary::DodgerRitual(vec![rows("short")?, rows("enough")?])
                    }
                    Some("dodger_darkest_ritual") => StatefulPrimary::DodgerRitual(vec![
                        rows("short")?,
                        rows("enough")?,
                        rows("short_unlocked")?,
                        rows("enough_unlocked")?,
                    ]),
                    Some("dodger_respawn") => {
                        StatefulPrimary::DodgerRespawn([rows("RED")?, rows("BLUE")?])
                    }
                    Some("rowenna_lance") => {
                        StatefulPrimary::RowennaLance([rows("melee")?, rows("lance")?])
                    }
                    Some("rightmost_played_card") => {
                        StatefulPrimary::RightmostPlayedCard([rows("empty")?, rows("present")?])
                    }
                    Some("resolved_or_reperformed") => {
                        StatefulPrimary::ResolvedOrReperformed([rows("first")?, rows("repeat")?])
                    }
                    Some("snorri_active_runes") => {
                        let count = body["variants"]
                            .as_object()
                            .map(|variants| variants.len())
                            .filter(|count| matches!(count, 8 | 16))
                            .ok_or_else(|| unsupported("invalid rune variant count"))?;
                        StatefulPrimary::SnorriRunes(
                            (0..count)
                                .map(|mask| rows(&mask.to_string()))
                                .collect::<Result<_>>()?,
                        )
                    }
                    Some("whisper_trail_spawn_count") => {
                        let count = body["variants"]
                            .as_object()
                            .map(|variants| variants.len())
                            .filter(|count| (2..=4).contains(count))
                            .ok_or_else(|| unsupported("invalid trail variant count"))?;
                        StatefulPrimary::WhisperTrail(
                            (0..count)
                                .map(|distance| rows(&distance.to_string()))
                                .collect::<Result<_>>()?,
                        )
                    }
                    Some("emmitt_turn_discards") => {
                        let base = rows("empty")?;
                        let mut one = rows("one")?;
                        let insert_at = body["insert_at"]
                            .as_u64()
                            .and_then(|index| usize::try_from(index).ok())
                            .filter(|index| *index <= base.len() && one.len() == base.len() + 5)
                            .ok_or_else(|| unsupported("Emmitt retrieval group invalid"))?;
                        let group = one.drain(insert_at..insert_at + 5).collect();
                        StatefulPrimary::EmmittTurnDiscards {
                            base,
                            group,
                            insert_at,
                        }
                    }
                    Some("takahide_deck_gold") => StatefulPrimary::TakahideDeckGold([
                        rows("without_replacement")?,
                        rows("with_replacement")?,
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
        let mut fallback = HashMap::new();
        for name in ["movement", "attack", "defense", "skill"] {
            fallback.insert(
                name,
                data["fallback"][name]
                    .as_array()
                    .ok_or_else(|| unsupported("primary fallback missing"))?
                    .iter()
                    .map(|row| Recipe::compile_primary(row, steps, symbols))
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
    fn emmitt_retrieval_groups_bound(
        &mut self,
        objects: &BoundObjects,
        owner: Symbol,
        radius: i64,
        group: &[Recipe],
    ) -> Result<Vec<Recipe>> {
        let log_key = self.symbols.lookup("turn_discard_log").unwrap();
        let Some(ObjectValue::Object(log)) = objects.arena.root_field(log_key) else {
            return Err(unsupported("turn discard log missing"));
        };
        let team = self.world.hero(owner).unwrap().team;
        let origin = self.world.resolved_position(owner);
        let mut eligible = Vec::new();
        for hero in self.world.heroes.iter().filter(|hero| hero.team == team) {
            let Some(value) = objects.arena.field(log, hero.id) else {
                continue;
            };
            if !objects.arena.truthy(&value) {
                continue;
            }
            let mut include = hero.id == owner;
            if !include
                && radius > 0
                && let Some(origin) = origin
            {
                for piece in self.world.piece_ids(hero.id) {
                    if let Some(target) = self.world.position(piece)
                        && i64::from(origin.distance(target)) <= radius
                        && self
                            .world
                            .effect_connected(origin, target, &[])
                            .map_err(unsupported)?
                    {
                        include = true;
                        break;
                    }
                }
            }
            if include {
                let discards = objects
                    .arena
                    .typed_value(&value, Arc::make_mut(&mut self.symbols))
                    .map_err(unsupported)?;
                eligible.push((hero.id, discards));
            }
        }
        eligible.sort_by_key(|(hero, _)| *hero != owner);
        let symbols = Arc::make_mut(&mut self.symbols);
        let owner_marker = symbols
            .lookup("template-passive-owner")
            .ok_or_else(|| unsupported("Emmitt hero marker missing"))?;
        let discard_marker = symbols
            .lookup("template-time-capsule-discard")
            .ok_or_else(|| unsupported("Emmitt discard marker missing"))?;
        fn bind_group(
            value: &mut Datum,
            owner_marker: Symbol,
            hero: Symbol,
            discard_marker: Symbol,
            discards: &Datum,
            keys: &[(Symbol, Symbol)],
        ) {
            match value {
                Datum::Text(id) if *id == owner_marker => *id = hero,
                Datum::Text(id) => {
                    if let Some((_, key)) = keys.iter().find(|(marker, _)| marker == id) {
                        *id = *key;
                    }
                }
                Datum::List(values) if values.as_slice() == [Datum::Text(discard_marker)] => {
                    *value = discards.clone();
                }
                Datum::List(values) => {
                    for value in values {
                        bind_group(value, owner_marker, hero, discard_marker, discards, keys);
                    }
                }
                Datum::Object(fields) => {
                    for (_, value) in fields {
                        bind_group(value, owner_marker, hero, discard_marker, discards, keys);
                    }
                }
                _ => {}
            }
        }
        let mut recipes = Vec::new();
        for (index, (hero, discards)) in eligible.into_iter().enumerate() {
            let keys = ["hero", "choice", "yes", "cards", "card"].map(|name| {
                let marker = symbols.intern(&format!("tc_{name}_0"));
                let key = symbols.intern(&format!("tc_{name}_{index}"));
                (marker, key)
            });
            for template in group {
                let mut recipe = template.clone();
                bind_group(
                    &mut recipe.step.fields,
                    owner_marker,
                    hero,
                    discard_marker,
                    &discards,
                    &keys,
                );
                recipes.push(recipe);
            }
        }
        Ok(recipes)
    }

    fn has_level_eight_ultimate_bound(
        &self,
        objects: &BoundObjects,
        owner: Symbol,
    ) -> Result<bool> {
        let model = self
            .hero_model_bound(objects, owner)
            .ok_or_else(|| unsupported("ultimate performer model missing"))?;
        let ultimate = objects
            .arena
            .field(model, objects.phases.k(Field::Ultimate))
            .ok_or_else(|| unsupported("ultimate field missing"))?;
        if matches!(ultimate, ObjectValue::Atom(Datum::Null)) {
            return Ok(false);
        }
        let level = match objects.arena.field(model, objects.phases.k(Field::Level)) {
            Some(ObjectValue::Atom(Datum::Int(value))) => value,
            Some(ObjectValue::Atom(Datum::Bool(value))) => i64::from(value),
            _ => return Err(unsupported("ultimate performer level must be numeric")),
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
    pub(super) fn prepare_reperformed_primary_body_bound(
        &mut self,
        objects: &BoundObjects,
        hero: Symbol,
        card: crate::cards::CardId,
        computed: CardStats,
    ) -> Result<PrimaryPlan> {
        self.prepare_primary_body_kind_bound(
            objects,
            hero,
            card,
            Some(computed),
            PrimaryMode::Reperformed,
        )
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
    pub(super) fn prepare_passive_primary_bound(
        &mut self,
        objects: &BoundObjects,
        hero: Symbol,
        card: crate::cards::CardId,
        effect: Symbol,
        raw: bool,
    ) -> Result<PrimaryPlan> {
        let stats = self.world.card_stats(hero, card).map_err(unsupported)?;
        self.prepare_primary_body_kind_bound(
            objects,
            hero,
            card,
            Some(stats),
            PrimaryMode::Passive { effect, raw },
        )
    }
    fn empty_spawn_count(
        &self,
        hero: Symbol,
        radius: i64,
        cap: usize,
        minion_only: bool,
    ) -> Result<usize> {
        let Some(origin) = self.world.position(hero) else {
            return Ok(0);
        };
        let override_zones = self.world.tide_of_darkness(Some(hero));
        let mut count = 0;
        for &hex in self.world.tile_hexes() {
            if (!override_zones
                && (!self.world.spawn_at(hex, minion_only) || !self.world.tile_in_battle_zone(hex)))
                || self.world.obstacle(hex)
                || i64::from(origin.distance(hex)) > radius
                || !self
                    .world
                    .effect_connected(origin, hex, &[])
                    .map_err(unsupported)?
                || self
                    .world
                    .obstacle_for_actor(hex, Some(hero))
                    .map_err(unsupported)?
            {
                continue;
            }
            count += 1;
            if count == cap {
                break;
            }
        }
        Ok(count)
    }

    fn adjacent_empty_spawn_bound(&self, owner: Symbol, origin: Hex) -> Result<bool> {
        let tide = self.world.tide_of_darkness(Some(owner));
        for point in origin.neighbors() {
            if self.world.on_map(point)
                && (tide
                    || (self.world.spawn_at(point, false) && self.world.tile_in_battle_zone(point)))
                && self
                    .world
                    .effect_connected(origin, point, &[])
                    .map_err(unsupported)?
                && !self
                    .world
                    .obstacle_for_actor(point, Some(owner))
                    .map_err(unsupported)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn active_runes_bound(&self, objects: &BoundObjects, printed: Symbol) -> usize {
        let p = &objects.phases;
        let allowed = [
            Field::RuneBird,
            Field::RuneAnvil,
            Field::RuneHorn,
            Field::RuneAxe,
        ]
        .map(|field| p.k(field));
        let mut mask = 0;
        let mut include = |value: ObjectValue| {
            if let ObjectValue::Atom(Datum::Text(rune)) = value
                && let Some(index) = allowed.iter().position(|allowed| *allowed == rune)
            {
                mask |= 1 << index;
            }
        };
        if let Some(hero) = self.world.heroes.iter().find(|hero| {
            hero.current
                .iter()
                .chain(&hero.extra)
                .chain(&hero.hand)
                .chain(&hero.deck)
                .chain(&hero.discard)
                .chain(hero.played.iter().flatten())
                .chain(&hero.ultimate)
                .any(|id| {
                    self.world.card(*id).is_some_and(|row| {
                        self.world.definitions.cards[row.definition].printed_id == printed
                    })
                })
        }) && let Some(model) = self.hero_model_bound(objects, hero.id)
            && let Some(ObjectValue::Object(slots)) =
                objects.arena.field(model, p.k(Field::RuneSlots))
            && let Some(slot) = self.symbols.lookup(&self.world.combat.turn.to_string())
            && let Some(value) = objects.arena.field(slots, slot)
        {
            include(value);
        }
        for field in [Field::RuneActionOutput, Field::RuneDefenseOutput] {
            if let Some(value) = objects.arena.context_value(p.k(field)) {
                include(value);
            }
        }
        mask
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
        let mut definition = self.world.definitions.cards[instance.definition].clone();
        let lifecycle = instance.lifecycle;
        let physical_effect = definition.effect;
        let passive = matches!(mode, PrimaryMode::Passive { .. });
        if let PrimaryMode::Passive { effect, .. } = mode {
            definition.effect = effect;
        }
        let owner = self.world.hero(hero).unwrap().id;
        plan.owner = Some(owner);
        plan.card = Some(definition.printed_id);
        if !passive && (instance.facedown || (definition.effect_absent && !copied)) {
            return Ok(plan);
        }
        let rules = objects.scheduler.as_ref().unwrap().primary_bodies.clone();
        let spell = !passive
            && mode != PrimaryMode::AfterAttack
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
        let mut list_binding = None;
        let mut binding_card = definition.printed_id;
        let mut texts = Vec::new();
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
                    StatefulPrimary::LevelEightUltimate { variants, required } => {
                        let enabled = self.has_level_eight_ultimate_bound(objects, owner)?
                            && required.is_none_or(|required| {
                                self.world
                                    .hero(owner)
                                    .and_then(|hero| hero.ultimate)
                                    .and_then(|id| self.world.card(id))
                                    .is_some_and(|card| {
                                        let definition =
                                            &self.world.definitions.cards[card.definition];
                                        definition.printed_id == required
                                            && (self.symbols.name(required) != "twin_strike"
                                                || (!card.facedown
                                                    && card.lifecycle
                                                        == crate::world::CardLifecycle::Passive
                                                    && definition.effect == required))
                                    })
                            });
                        variants[usize::from(enabled)].clone()
                    }
                    StatefulPrimary::BountyMovement {
                        base,
                        bonus,
                        marker,
                    } => {
                        let markers = objects.phases.root(&objects.arena, Field::Markers)?;
                        if let Some(ObjectValue::Object(model)) =
                            objects.arena.field(markers, *marker)
                            && matches!(
                                objects
                                    .arena
                                    .field(model, objects.phases.k(Field::TargetId)),
                                Some(ObjectValue::Atom(Datum::Text(_)))
                            )
                        {
                            stats.primary_value += bonus;
                        }
                        base.clone()
                    }
                    StatefulPrimary::DodgerDeathTrap { variants, operand } => {
                        let origin = self.world.position(owner);
                        let team = self.world.hero(owner).unwrap().team;
                        let mut excluded = Vec::new();
                        let mut any_valid = false;
                        if let Some(origin) = origin.filter(|_| !self.world.battle_zones.is_empty())
                        {
                            for (id, _) in &objects.phases.heroes {
                                if self.world.hero(*id).unwrap().team == team {
                                    continue;
                                }
                                let entity = self.world.entity(*id).unwrap();
                                let mut presences = vec![*id];
                                if entity.multi_piece {
                                    presences = self
                                        .world
                                        .definitions
                                        .entities
                                        .iter()
                                        .filter(|piece| {
                                            piece.kind == crate::world::EntityKind::Piece
                                                && piece.owner == Some(*id)
                                        })
                                        .map(|piece| piece.id)
                                        .collect();
                                    presences.sort_by_key(|id| self.symbols.name(*id));
                                }
                                for id in presences {
                                    let Some(point) = self.world.position(id) else {
                                        continue;
                                    };
                                    if i64::from(origin.distance(point))
                                        <= stats.radius.unwrap_or(0)
                                        && self
                                            .world
                                            .effect_connected(origin, point, &[])
                                            .map_err(unsupported)?
                                        && self.adjacent_empty_spawn_bound(owner, point)?
                                    {
                                        any_valid = true;
                                    } else {
                                        excluded.push(id);
                                    }
                                }
                            }
                        }
                        if any_valid {
                            let index = usize::from(!excluded.is_empty());
                            list_binding = Some((*operand, excluded));
                            variants[index].clone()
                        } else {
                            Vec::new()
                        }
                    }
                    StatefulPrimary::DodgerDreadRazor(variants) => {
                        let choice = if let Some(origin) = self.world.position(owner) {
                            !self.world.battle_zones.is_empty()
                                && self.adjacent_empty_spawn_bound(owner, origin)?
                        } else {
                            false
                        };
                        variants[usize::from(choice)].clone()
                    }
                    StatefulPrimary::ActingPiece(recipes) => {
                        let marker = self
                            .symbols
                            .lookup("template-razzle-piece")
                            .ok_or_else(|| unsupported("acting-piece operand missing"))?;
                        texts.push((marker, self.world.resolved_actor(owner)));
                        recipes.clone()
                    }
                    StatefulPrimary::TopologyAnchor(recipes) => {
                        if let Some(anchor) = self.world.resolved_position(owner) {
                            let (q, r, s) = anchor.coordinates();
                            let coordinates = [("q", q), ("r", r), ("s", s)];
                            let locations =
                                objects.phases.root(&objects.arena, Field::Locations)?;
                            let position = objects.arena.field(locations, owner).or_else(|| {
                                objects
                                    .arena
                                    .field(locations, self.world.resolved_actor(owner))
                            });
                            let Some(ObjectValue::Object(position)) = position else {
                                return Err(unsupported(
                                    "topology anchor missing from object graph",
                                ));
                            };
                            let anchor_field = self
                                .symbols
                                .lookup("isolated_hex")
                                .ok_or_else(|| unsupported("topology anchor field missing"))?;
                            let mut recipes = recipes.clone();
                            for recipe in &mut recipes {
                                if !matches!(
                                    self.steps[recipe.step.instruction].instruction,
                                    Instruction::LiveCreateEffect
                                ) {
                                    continue;
                                }
                                let Datum::Object(fields) = &mut recipe.step.fields else {
                                    return Err(unsupported("topology effect fields missing"));
                                };
                                let axis = fields
                                    .iter()
                                    .find_map(|(key, value)| match value {
                                        Datum::Text(axis)
                                            if self.symbols.name(*key) == "split_axis" =>
                                        {
                                            Some(self.symbols.name(*axis))
                                        }
                                        _ => None,
                                    })
                                    .ok_or_else(|| unsupported("topology axis missing"))?;
                                let split = coordinates
                                    .iter()
                                    .find(|(key, _)| *key == axis)
                                    .ok_or_else(|| unsupported("unknown topology axis"))?
                                    .1;
                                for (key, value) in fields {
                                    if self.symbols.name(*key) == "split_value" {
                                        *value = Datum::Int(i64::from(split));
                                    }
                                }
                                recipe.topology_anchor = Some((anchor_field, position));
                            }
                            recipes
                        } else {
                            Vec::new()
                        }
                    }
                    StatefulPrimary::FaceupSkills(recipes) => {
                        let mut eligible = Vec::new();
                        for &candidate in self.world.hero(owner).unwrap().played.iter().flatten() {
                            let instance = self.world.card(candidate).unwrap();
                            let definition = &self.world.definitions.cards[instance.definition];
                            if !instance.facedown
                                && definition.primary == Some(ActionKind::Skill)
                                && self.passive_primary_allowed(owner, candidate)?
                            {
                                eligible.push(definition.printed_id);
                            }
                        }
                        list_binding = Some((rules.card, eligible));
                        recipes.clone()
                    }
                    StatefulPrimary::IgnatiaCoin(variants) => {
                        let equilibrium = self.world.effects.instances.iter().any(|effect| {
                            effect.source == owner
                                && self.symbols.name(effect.definition.kind) == "equilibrium"
                        });
                        let hero = self.world.hero(owner).unwrap();
                        let ultimate = hero.level >= 8
                            && hero
                                .ultimate
                                .and_then(|id| self.world.card(id))
                                .is_some_and(|card| {
                                    self.symbols.name(
                                        self.world.definitions.cards[card.definition].printed_id,
                                    ) == "chaos_incarnate"
                                });
                        let branch = if equilibrium {
                            2
                        } else {
                            usize::from(self.team == Team::Red)
                        };
                        variants[branch + 3 * usize::from(ultimate)].clone()
                    }
                    StatefulPrimary::UrsafarEnraged {
                        variants,
                        primary_bonus,
                    } => {
                        let hero = self.world.hero(owner).unwrap();
                        let ultimate = hero.level >= 8
                            && hero
                                .ultimate
                                .and_then(|id| self.world.card(id))
                                .is_some_and(|card| {
                                    self.symbols.name(
                                        self.world.definitions.cards[card.definition].printed_id,
                                    ) == "unbound_fury"
                                });
                        let enraged = ultimate
                            || self.world.effects.instances.iter().any(|effect| {
                                effect.active
                                    && effect.source == owner
                                    && self.symbols.name(effect.definition.kind) == "enraged"
                            });
                        if enraged {
                            stats.primary_value += primary_bonus;
                        }
                        let mut enabled = enraged;
                        if enraged && self.symbols.name(definition.effect) == "angry_roar" {
                            let played: Vec<_> = hero.played.iter().flatten().copied().collect();
                            let mut eligible = Vec::new();
                            for candidate in played {
                                let Some(instance) = self.world.card(candidate) else {
                                    continue;
                                };
                                let candidate_definition =
                                    &self.world.definitions.cards[instance.definition];
                                let printed = candidate_definition.printed_id;
                                let effect = candidate_definition.effect;
                                if printed == definition.printed_id
                                    || instance.facedown
                                    || !(instance.active
                                        || instance.enraged
                                            && instance.lifecycle
                                                == crate::world::CardLifecycle::Resolved)
                                    || !rules.registered(effect, false)
                                {
                                    continue;
                                }
                                // Angry Roar always creates rage. Avoid recursively inspecting
                                // its choices if another physical card has copied that effect.
                                if self.symbols.name(effect) == "angry_roar"
                                    || self
                                        .prepare_passive_primary_bound(
                                            objects, owner, candidate, effect, true,
                                        )?
                                        .recipes
                                        .iter()
                                        .any(|recipe| {
                                            matches!(
                                                self.steps[recipe.step.instruction].instruction,
                                                Instruction::LiveCreateEffect
                                            )
                                        })
                                {
                                    eligible.push(printed);
                                }
                            }
                            enabled = !eligible.is_empty();
                            if enabled {
                                list_binding = Some((rules.card, eligible));
                            }
                        }
                        variants[usize::from(enabled)].clone()
                    }
                    StatefulPrimary::CardInDiscard(variants) => {
                        let discarded = lifecycle == crate::world::CardLifecycle::Discard;
                        let printed_matches = self.symbols.name(definition.effect) != "blizzard"
                            || self.symbols.name(definition.printed_id) == "blizzard";
                        variants[usize::from(discarded && printed_matches)].clone()
                    }
                    StatefulPrimary::TaliWinterWeapon { base, bonus } => {
                        if lifecycle != crate::world::CardLifecycle::Discard {
                            stats.primary_value += bonus;
                        }
                        base.clone()
                    }
                    StatefulPrimary::DodgerRitual(variants) => {
                        let count =
                            self.empty_spawn_count(owner, stats.radius.unwrap_or(0), 2, false)?;
                        let unlocked =
                            variants.len() == 4 && self.world.hero(owner).unwrap().level >= 8;
                        variants[usize::from(count >= 2) + 2 * usize::from(unlocked)].clone()
                    }
                    StatefulPrimary::DodgerRespawn(variants) => {
                        let team = self.world.hero(owner).unwrap().team;
                        let has_limbo = self.world.definitions.entities.iter().any(|entity| {
                            entity.kind == crate::world::EntityKind::Minion
                                && entity.roster == Some(team)
                                && !self.world.has_board_presence(entity.id)
                        });
                        if has_limbo {
                            variants[usize::from(self.symbols.name(team) == "BLUE")].clone()
                        } else {
                            Vec::new()
                        }
                    }
                    StatefulPrimary::RowennaLance(variants) => {
                        let lance = self.world.hero(owner).unwrap().level >= 8;
                        if lance {
                            let inputs = self.spatial_inputs_bound(
                                objects,
                                [
                                    objects.phases.k(Field::PerformingCard),
                                    objects.phases.k(Field::PerformingOwner),
                                ],
                            );
                            stats.range = self
                                .world
                                .computed_stat_in_context(owner, Stat::Range, 2, &inputs)
                                .map_err(unsupported)?;
                        }
                        variants[usize::from(lance)].clone()
                    }
                    StatefulPrimary::RightmostPlayedCard(variants) => {
                        let previous = self
                            .world
                            .hero(owner)
                            .and_then(|hero| hero.played.iter().rev().flatten().next())
                            .and_then(|id| self.world.card(*id));
                        if let Some(previous) = previous {
                            binding_card =
                                self.world.definitions.cards[previous.definition].printed_id;
                        }
                        variants[usize::from(previous.is_some())].clone()
                    }
                    StatefulPrimary::ResolvedOrReperformed(variants) => {
                        let reperforming = self.symbols.lookup("reperforming_card_id")
                            .and_then(|key| objects.arena.context_value(key))
                            .is_some_and(|value| matches!(value, ObjectValue::Atom(Datum::Text(id)) if id == definition.printed_id));
                        let repeat = lifecycle == crate::world::CardLifecycle::Resolved
                            || mode == PrimaryMode::Reperformed
                            || reperforming;
                        variants[usize::from(repeat)].clone()
                    }
                    StatefulPrimary::SnorriRunes(variants) => variants[self
                        .active_runes_bound(objects, definition.printed_id)
                        & (variants.len() - 1)]
                        .clone(),
                    StatefulPrimary::WhisperTrail(variants) => {
                        let count = self.empty_spawn_count(
                            owner,
                            stats.radius.unwrap_or(0),
                            variants.len() - 1,
                            true,
                        )?;
                        variants[count].clone()
                    }
                    StatefulPrimary::EmmittTurnDiscards {
                        base,
                        group,
                        insert_at,
                    } => {
                        let mut recipes = base.clone();
                        let groups = self.emmitt_retrieval_groups_bound(
                            objects,
                            owner,
                            stats.radius.unwrap_or(0),
                            group,
                        )?;
                        recipes.splice(*insert_at..*insert_at, groups);
                        recipes
                    }
                    StatefulPrimary::TakahideDeckGold(variants) => {
                        use crate::world::CardLifecycle;
                        let hero = self.world.hero(owner).unwrap();
                        let golds = hero.deck.iter().filter_map(|id| {
                            let card = self.world.card(*id).unwrap();
                            let definition = &self.world.definitions.cards[card.definition];
                            (definition.color == self.symbols.lookup("GOLD"))
                                .then_some((definition.printed_id, card.lifecycle))
                        });
                        let mut replacement =
                            golds.clone().any(|(_, state)| state == CardLifecycle::Deck);
                        let effect_name = self.symbols.name(definition.effect);
                        if effect_name == "bushido" {
                            if let Some((outgoing, _)) = golds
                                .clone()
                                .find(|(_, state)| *state != CardLifecycle::Deck)
                            {
                                binding_card = outgoing;
                            } else {
                                replacement = false;
                            }
                        }
                        if effect_name == "sting_like_a_bee" {
                            let marker = format!(
                                "Target a unit at maximum range ({})",
                                crate::primary::RANGE
                            );
                            let prompt = self.symbols.lookup(&marker).ok_or_else(|| {
                                unsupported("Takahide maximum-range prompt missing")
                            })?;
                            logs = Some((
                                prompt,
                                format!("Target a unit at maximum range ({})", stats.range),
                            ));
                        }
                        variants[usize::from(replacement)].clone()
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
                        list_binding = Some((*operand, ids));
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
        // The shared source dispatcher reads the physical card's effect ID,
        // even when a passive explicitly invokes a different registry entry.
        if passive
            && registered
                .is_some_and(|method| self.symbols.name(*method) == "SpellAccessEffect.build_steps")
        {
            let physical = physical_effect;
            let key = self.symbols.lookup("allowed_spell_ids").unwrap();
            let allowed = if rules.methods.get(&physical) == registered {
                rules
                    .recipes
                    .get(&physical)
                    .and_then(|rows| rows.first())
                    .and_then(|row| {
                        if let Datum::Object(fields) = &row.step.fields {
                            fields
                                .iter()
                                .find(|(field, _)| *field == key)
                                .map(|(_, value)| value.clone())
                        } else {
                            None
                        }
                    })
                    .ok_or_else(|| unsupported("physical spell access recipe missing"))?
            } else {
                Datum::List(Vec::new())
            };
            let Datum::Object(fields) = &mut plan.recipes[0].step.fields else {
                unreachable!()
            };
            fields
                .iter_mut()
                .find(|(field, _)| *field == key)
                .unwrap()
                .1 = allowed;
        }
        if let Some((key, text)) = logs {
            texts.push((key, Arc::make_mut(&mut self.symbols).intern(&text)));
        }
        let suffixes: &[&str] = match self.symbols.name(definition.effect) {
            "cold_snap" | "snowstorm" => &["direction"],
            "blizzard" => &["direction", "repeat_direction"],
            "spirit_wolf" | "spirit_bear" => &[
                "targets",
                "choice",
                "range_first",
                "adjacent_first",
                "range_target",
                "adjacent_target",
                "range_second",
                "adjacent_second",
                "range_second_target",
                "adjacent_second_target",
            ],
            "time_walk" | "fast_forward" => &["victim", "dest"],
            "time_snare" | "time_trap" | "time_bomb" => &["victim"],
            "back_to_the_future" => &[
                "victim",
                "dest",
                "choice",
                "recall",
                "shove",
                "target",
                "target_hex",
            ],
            _ => &[],
        };
        for suffix in suffixes {
            let marker = format!("{}_{suffix}", self.symbols.name(rules.card));
            let key = self
                .symbols
                .lookup(&marker)
                .ok_or_else(|| unsupported("Emmitt displacement context key missing"))?;
            let value = format!("{}_{suffix}", self.symbols.name(binding_card));
            texts.push((key, Arc::make_mut(&mut self.symbols).intern(&value)));
        }
        fn bind(
            value: &mut Datum,
            rules: &PrimaryRules,
            card: Symbol,
            texts: &[(Symbol, Symbol)],
            retired: Option<(Symbol, &[Symbol])>,
        ) -> Result<()> {
            match value {
                Datum::Text(v) if *v == rules.card => *v = card,
                Datum::Text(v) => {
                    if let Some((_, replacement)) = texts.iter().find(|(key, _)| key == v) {
                        *v = *replacement;
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
                            bind(value, rules, card, texts, retired)?;
                        }
                    }
                }
                Datum::Object(fields) => {
                    for (_, value) in fields {
                        bind(value, rules, card, texts, retired)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
        if self.symbols.name(definition.effect) == "ransack" {
            let supply = self.hero_model_bound(objects, owner).and_then(|model| {
                let field = self.symbols.lookup("piece_supply")?;
                objects.arena.field(model, field)
            });
            let Some(ObjectValue::Atom(Datum::Int(supply))) = supply else {
                return Err(unsupported("Ransack piece supply missing"));
            };
            // The attack and count are followed by one three-step group per
            // potential other piece, up to the printed cap of three.
            plan.recipes
                .truncate(2 + 3 * (supply.clamp(1, 4) as usize - 1));
        }
        for (index, recipe) in plan.recipes.iter_mut().enumerate() {
            super::primary_operands::bind(
                &mut recipe.step.fields,
                &recipe.primary_operands,
                &stats,
            )?;
            if matches!(mode, PrimaryMode::Passive { raw: true, .. }) {
                clear_raw_card_bindings(recipe)?;
            }
            bind(
                &mut recipe.step.fields,
                &rules,
                binding_card,
                &texts,
                list_binding
                    .as_ref()
                    .map(|(marker, ids)| (*marker, ids.as_slice())),
            )?;
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
        if !passive && mode != PrimaryMode::AfterAttack && count != 0 {
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
        let queued = self.allocate_source_recipes_bound(
            objects,
            plan.recipes.iter().map(|recipe| (recipe, owner)),
        )?;
        self.push_live_steps_bound(objects, &queued)
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
            .find(|entry| entry["name"] == name)
            .unwrap();
        let case: Value = serde_json::from_reader(std::io::BufReader::new(
            crate::fixture_cache::open(base.join(entry["path"].as_str().unwrap())).unwrap(),
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

#[cfg(test)]
mod hanu_tests {
    use super::*;
    fn session(effect: &str) -> NativeSession {
        let case = fixture_case(&format!(
            "object-execution-hanu-primary-{effect}-text-first"
        ));
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn hanu_group_generation_guard_precedes_context_and_stack_mutation() {
        let mut worker = session("trusted_sidekick");
        worker.next_step = u64::MAX - 1;
        let before = worker.export_envelope().unwrap();
        assert!(worker.advance_prefix(1).is_err());
        let after = worker.export_envelope().unwrap();
        assert_eq!(before.state, after.state);
        assert_eq!(before.object_bindings, after.object_bindings);
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn hanu_empty_ultimate_needs_no_generated_step_capacity() {
        let mut worker = session("the_ultimate_trick");
        worker.next_step = u64::MAX;
        worker.advance_prefix(1).unwrap();
        assert_eq!(worker.next_step, u64::MAX);
        assert_eq!(worker.stack.len(), 1);
    }
    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn hanu_cross_root_filter_models_share_within_one_worker_and_remain_private() {
        for effect in ["trusted_sidekick", "pile_on"] {
            let root = session(effect);
            let before = root.export_envelope().unwrap();
            let mut worker = root.clone();
            worker.advance_prefix(1).unwrap();
            let other = worker.clone();
            let objects = worker.objects.as_ref().unwrap();
            let filters = worker
                .stack
                .iter()
                .filter_map(|frame| frame.object)
                .filter_map(|step| {
                    objects
                        .arena
                        .field(step, objects.phases.k(Field::TargetFilters))
                })
                .filter_map(|value| match value {
                    ObjectValue::Object(list) => objects.arena.sequence(list),
                    _ => None,
                })
                .filter_map(|list| list.first())
                .filter_map(|value| match value {
                    ObjectValue::Object(id) => Some(*id),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(filters.len(), 2);
            assert_eq!(filters[0], filters[1]);
            let key = worker.symbols.lookup("unit_type").unwrap();
            let minion = worker
                .symbols
                .lookup(if effect == "pile_on" {
                    "HERO"
                } else {
                    "MINION"
                })
                .unwrap();
            worker
                .objects
                .as_mut()
                .unwrap()
                .arena
                .set_field(filters[0], key, ObjectValue::Atom(Datum::Text(minion)))
                .unwrap();
            let other_objects = other.objects.as_ref().unwrap();
            assert!(
                matches!(worker.objects.as_ref().unwrap().arena.field(filters[1], key),
                Some(ObjectValue::Atom(Datum::Text(value))) if value == minion)
            );
            assert!(matches!(other_objects.arena.field(filters[1], key),
                Some(ObjectValue::Atom(Datum::Text(value))) if value != minion));
            let after = root.export_envelope().unwrap();
            assert_eq!(before.state, after.state);
            assert_eq!(before.object_bindings, after.object_bindings);
        }
    }
}
