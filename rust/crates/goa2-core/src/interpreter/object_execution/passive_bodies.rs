//! Cold source recipes and live typed binding for passive continuations.
use super::phases::Field;
use super::*;
use crate::{cards::CardId, objects::ObjectSlot};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Kind {
    Living,
    Profession,
    Jinx,
    Legend,
    Fury,
    Archwizard,
    Flurry,
    Hell,
    Gun,
    Wild,
    Runes,
    Bullet,
    Winter,
    Cloak,
    Voltage,
    Dragon,
    March,
    CloakAttack,
    CloakMove,
    MovePlain,
    MoveStraight,
    MoveFull,
    MoveStraightFull,
    TokenOverflow,
    BatchPrimitives,
    RockUltimate,
}
fn method_kind(method: &str) -> Result<Kind> {
    Ok(match method {
        "LivingTsunamiEffect.get_passive_steps" => Kind::Living,
        "AComplicatedProfessionEffect.get_passive_steps" => Kind::Profession,
        "JinxEffect.get_passive_steps" => Kind::Jinx,
        "LegendOfTheSkiesEffect.get_passive_steps" => Kind::Legend,
        "BattleFuryEffect.get_passive_steps" => Kind::Fury,
        "TheArchwizardEffect.get_passive_steps" => Kind::Archwizard,
        "FlurryOfBlowsEffect.get_passive_steps" => Kind::Flurry,
        "WhatTheHellAreYouEffect.get_passive_steps" => Kind::Hell,
        "BigSoddingGunEffect.get_passive_steps" => Kind::Gun,
        "WildHuntEffect.get_passive_steps" => Kind::Wild,
        "RuneMasteryEffect.get_passive_steps" => Kind::Runes,
        "BulletTimeEffect.get_passive_steps" => Kind::Bullet,
        "ReignOfWinterEffect.get_passive_steps" => Kind::Winter,
        "CloakAndDaggersEffect.get_passive_steps" => Kind::Cloak,
        "HighVoltageEffect.get_passive_steps" => Kind::Voltage,
        "DragonKnightEffect.get_passive_steps" => Kind::Dragon,
        "MarchOfNatureEffect.get_passive_steps" => Kind::March,
        _ => return Err(unsupported("unknown passive body method")),
    })
}
#[derive(Clone, Debug)]
pub(super) struct Recipe {
    pub(super) step: StepRecipe,
    pub(super) topology_anchor: Option<(Symbol, ObjectId)>,
    pub(super) objects: Vec<(Vec<ObjectSlot>, ObjectKind)>,
    pub(super) primary_operands: Vec<super::primary_operands::Binding>,
    pub(super) raw_card_bindings: Vec<Vec<ObjectSlot>>,
    imports: Vec<(Vec<ObjectSlot>, usize, Vec<ObjectSlot>)>,
    range: Option<Vec<ObjectSlot>>,
    aliases: Vec<(Vec<ObjectSlot>, Vec<ObjectSlot>)>,
    programs: Vec<(Vec<ObjectSlot>, usize)>,
}
impl Recipe {
    pub(super) fn compile_primary(
        row: &Value,
        steps: &mut Vec<CompiledStep>,
        symbols: &mut Symbols,
    ) -> Result<Self> {
        let mut row = row.clone();
        let operands = super::primary_operands::extract(&mut row["fields"], symbols)?;
        let mut recipe = Self::compile(&row, steps, symbols)?;
        recipe.primary_operands = operands;
        Ok(recipe)
    }

    pub(super) fn compile(
        row: &Value,
        steps: &mut Vec<CompiledStep>,
        symbols: &mut Symbols,
    ) -> Result<Self> {
        let wire = &row["fields"];
        let instruction = BoundObjects::compile_wire(wire, steps, symbols)?;
        let mut objects = Vec::new();
        let mut aliases = Vec::new();
        let mut programs = Vec::new();
        let mut range = None;
        for value in row["objects"]
            .as_array()
            .ok_or_else(|| unsupported("source recipe model bindings missing"))?
        {
            let binding: crate::codec::ObjectBinding = serde_json::from_value(value.clone())
                .map_err(|error| unsupported(error.to_string()))?;
            if binding.paths.is_empty()
                || (binding.deepcopy_shared && binding.kind != ObjectKind::FrozenModel)
            {
                return Err(unsupported("invalid source recipe binding"));
            }
            let typed = |path: &[ObjectPathPart], symbols: &mut Symbols| {
                path.iter()
                    .map(|part| match part {
                        ObjectPathPart::Key(key) => ObjectSlot::Field(symbols.intern(key)),
                        ObjectPathPart::Index(index) => ObjectSlot::Index(*index),
                    })
                    .collect::<Vec<_>>()
            };
            let path = &binding.paths[0];
            let first = typed(path, symbols);
            let child = crate::codec::object_at_path(wire, path).map_err(unsupported)?;
            if child
                .get("type")
                .is_some_and(|value| value == "range_filter")
            {
                range = Some(first.clone());
            }
            if binding.kind == ObjectKind::Step && !path.is_empty() {
                programs.push((
                    first.clone(),
                    BoundObjects::compile_wire(child, steps, symbols)?,
                ));
            }
            for path in &binding.paths[1..] {
                aliases.push((typed(path, symbols), first.clone()));
            }
            objects.push((first, binding.kind));
        }
        let mut imports = Vec::new();
        if let Some(rows) = row.get("imports") {
            for import in rows
                .as_array()
                .ok_or_else(|| unsupported("recipe imports missing"))?
            {
                let path = |value: &Value, symbols: &mut Symbols| -> Result<Vec<ObjectSlot>> {
                    let parts: Vec<ObjectPathPart> = serde_json::from_value(value.clone())
                        .map_err(|e| unsupported(e.to_string()))?;
                    Ok(parts
                        .into_iter()
                        .map(|part| match part {
                            ObjectPathPart::Key(key) => ObjectSlot::Field(symbols.intern(&key)),
                            ObjectPathPart::Index(index) => ObjectSlot::Index(index),
                        })
                        .collect())
                };
                let back = import["back"]
                    .as_u64()
                    .and_then(|v| usize::try_from(v).ok())
                    .filter(|v| *v > 0)
                    .ok_or_else(|| unsupported("recipe import offset invalid"))?;
                imports.push((
                    path(&import["path"], symbols)?,
                    back,
                    path(&import["original"], symbols)?,
                ));
            }
        }
        Ok(Self {
            step: StepRecipe {
                instruction,
                fields: Datum::from_json(wire, symbols).map_err(unsupported)?,
            },
            objects,
            topology_anchor: None,
            primary_operands: Vec::new(),
            raw_card_bindings: Vec::new(),
            imports,
            range,
            aliases,
            programs,
        })
    }
}
#[derive(Clone, Debug)]
pub(super) struct BodyRules {
    methods: Vec<(Symbol, Kind)>,
    recipes: HashMap<Kind, Vec<Recipe>>,
    pub(super) owner: Symbol,
}
impl BodyRules {
    pub(super) fn compile(steps: &mut Vec<CompiledStep>, symbols: &mut Symbols) -> Result<Self> {
        let source =
            &crate::templates::instructions().map_err(unsupported)?["passive_continuations"];
        let mut methods = Vec::new();
        let mut kinds = HashMap::new();
        for (effect, method) in source["methods"]
            .as_object()
            .ok_or_else(|| unsupported("passive body metadata missing"))?
        {
            let kind = method_kind(
                method
                    .as_str()
                    .ok_or_else(|| unsupported("passive body method invalid"))?,
            )?;
            methods.push((symbols.intern(effect), kind));
            kinds.insert(effect.as_str(), kind);
        }
        let mut recipes = HashMap::new();
        for (name, rows) in source["recipes"]
            .as_object()
            .ok_or_else(|| unsupported("passive body recipes missing"))?
        {
            let kind = match name.as_str() {
                "token/rock_ultimate" => Kind::RockUltimate,
                "token/batch_primitives" => Kind::BatchPrimitives,
                "token/overflow" => Kind::TokenOverflow,
                "movement/0" => Kind::MovePlain,
                "movement/1" => Kind::MoveStraight,
                "movement/2" => Kind::MoveFull,
                "movement/3" => Kind::MoveStraightFull,
                "cloak_and_daggers/ATTACK" => Kind::CloakAttack,
                "cloak_and_daggers/MOVEMENT" => Kind::CloakMove,
                other => *kinds
                    .get(other)
                    .ok_or_else(|| unsupported("unknown passive recipe"))?,
            };
            let mut body = Vec::new();
            for row in rows
                .as_array()
                .ok_or_else(|| unsupported("invalid passive recipe list"))?
            {
                body.push(Recipe::compile(row, steps, symbols)?);
            }
            recipes.insert(kind, body);
        }
        Ok(Self {
            methods,
            recipes,
            owner: symbols.intern("template-passive-owner"),
        })
    }
    pub(super) fn template_owner(&self) -> Symbol {
        self.owner
    }
    fn kind(&self, effect: Symbol) -> Option<Kind> {
        self.methods
            .iter()
            .find_map(|(id, k)| (*id == effect).then_some(*k))
    }
    pub(super) fn recipe(&self, kind: Kind) -> Result<&[Recipe]> {
        self.recipes
            .get(&kind)
            .map(Vec::as_slice)
            .ok_or_else(|| unsupported("passive recipe not compiled"))
    }
}
#[derive(Clone, Debug)]
enum ValuePatch {
    Scalar(Datum),
    List(Vec<Datum>),
}
#[derive(Clone, Debug)]
struct Patch {
    step: usize,
    range: bool,
    field: Symbol,
    value: ValuePatch,
}
pub(super) struct BodyPlan {
    kind: Kind,
    owner: Symbol,
    printed: Symbol,
    provider: Option<Symbol>,
    patches: Vec<Patch>,
    dynamic: Option<super::primary_bodies::PrimaryPlan>,
    last_flurry: Option<ObjectValue>,
    exclude_cloak: bool,
}
impl NativeSession {
    fn body_context_text(&self, objects: &BoundObjects, field: Field) -> Result<Option<Symbol>> {
        self.truthy_symbol_bound(
            objects,
            objects.arena.context_value(objects.phases.k(field)),
        )
    }
    fn body_integer(
        &self,
        objects: &BoundObjects,
        field: Field,
        default: Option<i64>,
    ) -> Result<i64> {
        match objects.arena.context_value(objects.phases.k(field)) {
            None => default.ok_or_else(|| unsupported("passive numeric operand missing")),
            Some(ObjectValue::Atom(Datum::Int(value))) => Ok(value),
            Some(ObjectValue::Atom(Datum::Bool(value))) => Ok(i64::from(value)),
            Some(ObjectValue::Atom(Datum::Float(value)))
                if value.is_finite()
                    && value.fract() == 0.0
                    && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0)
                        .contains(&value) =>
            {
                Ok(value as i64)
            }
            _ => Err(unsupported("passive numeric coercion not compiled")),
        }
    }
    pub(super) fn passive_body_plan(
        &mut self,
        objects: &BoundObjects,
        owner: Symbol,
        card: CardId,
        effect: Symbol,
        offer: crate::passives::Offer,
    ) -> Result<Option<BodyPlan>> {
        let program = objects.scheduler.as_ref().unwrap().clone();
        let Some(mut kind) = program.passive_bodies.kind(effect) else {
            return Ok(None);
        };
        let p = &objects.phases;
        let hero = self
            .world
            .hero(owner)
            .ok_or_else(|| unsupported("passive owner missing"))?;
        let mut dynamic = None;
        let mut last_flurry = None;
        let mut exclude_cloak = false;
        let mut patches = Vec::new();
        let mut scalar = |step, range, field, value| {
            patches.push(Patch {
                step,
                range,
                field: p.k(field),
                value: ValuePatch::Scalar(value),
            })
        };
        let expected = match kind {
            Kind::Living => Some(Field::BeforeAttack),
            Kind::Profession => Some(Field::AfterMarker),
            Kind::Jinx | Kind::Fury => Some(Field::AfterDiscard),
            Kind::Hell => Some(Field::AfterLaugh),
            Kind::Gun => Some(Field::AfterPush),
            Kind::Wild => Some(Field::BeforeAction),
            Kind::Winter | Kind::Flurry => Some(Field::AfterAttack),
            Kind::Cloak => Some(Field::AfterBasic),
            Kind::Voltage => Some(Field::AfterBasicSkill),
            Kind::Dragon => Some(Field::AfterMovement),
            Kind::March => Some(Field::AfterResolve),
            _ => None,
        };
        if expected.is_some_and(|field| offer.trigger != p.k(field)) {
            return Ok(None);
        }
        match kind {
            Kind::Profession | Kind::Gun => {
                let key = if kind == Kind::Gun {
                    Field::PushVictim
                } else {
                    Field::MarkerTarget
                };
                let Some(victim) = self.body_context_text(objects, key)? else {
                    return Ok(None);
                };
                let Some(victim) = self.world.hero(victim) else {
                    return Ok(None);
                };
                if kind == Kind::Gun && victim.team == hero.team {
                    return Ok(None);
                }
            }
            Kind::Legend => {
                let Some(previous) = hero.card_in_turn_slot(
                    self.world
                        .combat
                        .turn
                        .checked_sub(1)
                        .ok_or_else(|| unsupported("passive turn overflow"))?,
                ) else {
                    return Ok(None);
                };
                let printed = self.world.definitions.cards
                    [self.world.card(previous).unwrap().definition]
                    .printed_id;
                scalar(0, false, Field::Value, Datum::Text(printed));
            }
            Kind::Fury | Kind::Bullet => {
                let field = if kind == Kind::Fury {
                    Field::DiscardedId
                } else {
                    Field::BasicActionCard
                };
                if !objects
                    .arena
                    .context_value(p.k(field))
                    .is_some_and(|value| objects.arena.truthy(&value))
                {
                    return Ok(None);
                }
            }
            Kind::Runes => {
                let model = self
                    .hero_model_bound(objects, owner)
                    .ok_or_else(|| unsupported("rune hero missing"))?;
                let Some(ObjectValue::Object(runes)) =
                    objects.arena.field(model, p.k(Field::RuneSlots))
                else {
                    return Err(unsupported("rune slots mapping missing"));
                };
                let fields = objects
                    .arena
                    .fields(runes)
                    .ok_or_else(|| unsupported("rune slots must be a mapping"))?;
                let active = self
                    .symbols
                    .lookup(&self.world.combat.turn.to_string())
                    .and_then(|key| objects.arena.field(runes, key));
                let options = [Field::RuneAxe, Field::RuneBird, Field::RuneAnvil, Field::RuneHorn].into_iter().map(|f| p.k(f)).filter(|rune| fields.iter().any(|(_,v)| matches!(v, ObjectValue::Atom(Datum::Text(value)) if value == rune)) && !matches!(&active, Some(ObjectValue::Atom(Datum::Text(value))) if value == rune)).map(Datum::Text).collect::<Vec<_>>();
                if options.is_empty() {
                    return Ok(None);
                }
                scalar(
                    0,
                    false,
                    Field::Output,
                    Datum::Text(p.k(if offer.trigger == p.k(Field::BeforeDefense) {
                        Field::RuneDefenseOutput
                    } else {
                        Field::RuneActionOutput
                    })),
                );
                patches.push(Patch {
                    step: 0,
                    range: false,
                    field: p.k(Field::Options),
                    value: ValuePatch::List(options),
                });
            }
            Kind::Hell | Kind::Winter | Kind::Voltage | Kind::March => {
                let radius = if kind == Kind::March {
                    self.world.definitions.cards[self.world.card(card).unwrap().definition]
                        .radius
                        .unwrap_or(0)
                } else {
                    self.world
                        .card_stats(owner, card)
                        .map_err(unsupported)?
                        .radius
                        .unwrap_or(0)
                };
                scalar(0, true, Field::MaxRange, Datum::Int(radius));
            }
            Kind::Dragon => {
                let mut cards = Vec::new();
                for id in hero.played.iter().flatten() {
                    let row = self.world.card(*id).unwrap();
                    let definition = &self.world.definitions.cards[row.definition];
                    if !row.facedown
                        && definition.primary == Some(crate::world::ActionKind::Skill)
                        && self.passive_primary_allowed(owner, *id)?
                    {
                        cards.push(Datum::Text(definition.printed_id));
                    }
                }
                if cards.is_empty() {
                    return Ok(None);
                }
                patches.push(Patch {
                    step: 0,
                    range: false,
                    field: p.k(Field::AllowedCards),
                    value: ValuePatch::List(cards),
                });
            }
            Kind::Flurry => {
                if objects
                    .arena
                    .context_value(p.k(Field::FlurryRepeat))
                    .is_some_and(|v| objects.arena.truthy(&v))
                {
                    return Ok(None);
                }
                let Some(effect) = self.body_context_text(objects, Field::AttackEffect)? else {
                    return Ok(None);
                };
                let Some(printed) = self.body_context_text(objects, Field::AttackCard)? else {
                    return Ok(None);
                };
                let source = hero
                    .current
                    .into_iter()
                    .chain(hero.played.iter().flatten().copied())
                    .chain(hero.discard.iter().copied())
                    .find(|id| {
                        self.world.definitions.cards[self.world.card(*id).unwrap().definition]
                            .printed_id
                            == printed
                    });
                let Some(source) = source else {
                    return Ok(None);
                };
                if !self.reaction_rules.registered_effect(effect) {
                    return Ok(None);
                }
                dynamic =
                    Some(self.prepare_passive_primary_bound(objects, owner, source, effect, true)?);
                last_flurry = Some(
                    objects
                        .arena
                        .context_value(p.k(Field::DefenderId))
                        .filter(|v| objects.arena.truthy(v))
                        .or_else(|| objects.arena.context_value(p.k(Field::Victim)))
                        .unwrap_or(ObjectValue::Atom(Datum::Null)),
                );
            }
            Kind::Cloak => {
                if !self
                    .world
                    .repeat_legality(owner)
                    .map_err(unsupported)?
                    .allowed
                {
                    return Ok(None);
                }
                let Some(action) = self.body_context_text(objects, Field::BasicActionType)? else {
                    return Ok(None);
                };
                if objects
                    .arena
                    .context_value(p.k(Field::BasicActionValue))
                    .is_none_or(|v| matches!(v, ObjectValue::Atom(Datum::Null)))
                {
                    return Ok(None);
                }
                if action != p.k(Field::AttackKind)
                    && action != p.k(Field::MovementKind)
                    && action != p.k(Field::SkillKind)
                {
                    return Ok(None);
                }
                if action != p.k(Field::MovementKind)
                    && let Some(effect) =
                        self.body_context_text(objects, Field::BasicActionEffect)?
                    && self.reaction_rules.registered_effect(effect)
                    && hero.current.is_some()
                {
                    let source = hero.current.unwrap();
                    dynamic = Some(
                        self.prepare_passive_primary_bound(objects, owner, source, effect, false)?,
                    );
                    exclude_cloak = action == p.k(Field::AttackKind);
                    kind = Kind::CloakAttack;
                }
                if dynamic.is_some() {
                    // The source primary supplies its own stats and restrictions.
                } else {
                    if action == p.k(Field::SkillKind) {
                        return Ok(None);
                    }
                    let value = self.body_integer(objects, Field::BasicActionValue, None)?;
                    kind = if action == p.k(Field::AttackKind) {
                        Kind::CloakAttack
                    } else {
                        Kind::CloakMove
                    };
                    scalar(
                        1,
                        false,
                        if kind == Kind::CloakAttack {
                            Field::Damage
                        } else {
                            Field::Range
                        },
                        Datum::Int(value),
                    );
                    if kind == Kind::CloakAttack {
                        scalar(
                            1,
                            false,
                            Field::Range,
                            Datum::Int(self.body_integer(
                                objects,
                                Field::BasicActionRange,
                                Some(1),
                            )?),
                        );
                    }
                }
            }
            _ => {}
        }
        let recipes = program.passive_bodies.recipe(kind)?;
        let recipes = if dynamic.is_some() && kind != Kind::Flurry {
            &recipes[..1]
        } else {
            recipes
        };
        for patch in &patches {
            let recipe = recipes
                .get(patch.step)
                .ok_or_else(|| unsupported("passive patch step missing"))?;
            if patch.range && recipe.range.is_none() {
                return Err(unsupported("passive radius slot missing"));
            }
        }
        let count = recipes
            .iter()
            .map(|r| {
                r.objects
                    .iter()
                    .filter(|(_, kind)| *kind == ObjectKind::Step)
                    .count() as u64
            })
            .sum::<u64>();
        let dynamic_count = dynamic.as_ref().map_or(0, |plan| {
            plan.recipes
                .iter()
                .flat_map(|r| &r.objects)
                .filter(|(_, k)| *k == ObjectKind::Step)
                .count() as u64
        });
        self.phase_generation_check(0, count + dynamic_count + 1)?;
        Ok(Some(BodyPlan {
            kind,
            owner,
            printed: offer.card,
            provider: offer.provider,
            patches,
            dynamic,
            last_flurry,
            exclude_cloak,
        }))
    }
    pub(super) fn queue_passive_body_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: BodyPlan,
    ) -> Result<()> {
        let program = objects.scheduler.as_ref().unwrap().clone();
        let recipes = program.passive_bodies.recipe(plan.kind)?;
        let recipes = if plan.dynamic.is_some() && plan.kind != Kind::Flurry {
            &recipes[..1]
        } else {
            recipes
        };
        let mut queued = Vec::new();
        for recipe in recipes {
            let step =
                self.allocate_source_recipe_bound(objects, recipe, &Datum::Text(plan.owner))?;
            queued.push(step);
        }
        for patch in plan.patches {
            let step = queued[patch.step];
            let target = if patch.range {
                objects
                    .arena
                    .classify_recipe_object(
                        step,
                        recipes[patch.step].range.as_ref().unwrap(),
                        ObjectKind::Model,
                    )
                    .map_err(unsupported)?
            } else {
                step
            };
            match patch.value {
                ValuePatch::Scalar(value) => objects
                    .arena
                    .set_field(target, patch.field, ObjectValue::Atom(value))
                    .map_err(unsupported)?,
                ValuePatch::List(values) => {
                    let Some(ObjectValue::Object(list)) = objects.arena.field(target, patch.field)
                    else {
                        return Err(unsupported("passive list slot missing"));
                    };
                    objects.arena.clear_list(list).map_err(unsupported)?;
                    for value in values {
                        objects
                            .arena
                            .append(list, ObjectValue::Atom(value))
                            .map_err(unsupported)?;
                    }
                }
            }
        }
        if let Some(dynamic) = plan.dynamic {
            if let Some(target) = &plan.last_flurry {
                objects
                    .arena
                    .set_field(queued[1], objects.phases.k(Field::Value), target.clone())
                    .map_err(unsupported)?;
            }
            let suffix = if plan.kind == Kind::Flurry {
                queued.pop()
            } else {
                None
            };
            let children = self.allocate_source_recipes_bound(
                objects,
                dynamic.recipes.iter().map(|recipe| (recipe, plan.owner)),
            )?;
            for child in children {
                let kind = match objects
                    .arena
                    .field(child, objects.phases.k(Field::FilterType))
                {
                    Some(ObjectValue::Atom(Datum::Text(kind))) => self.symbols.name(kind),
                    _ => return Err(unsupported("dynamic passive step type missing")),
                };
                let exclusion = if plan.kind == Kind::Flurry {
                    if kind == "attack_sequence"
                        && plan
                            .last_flurry
                            .as_ref()
                            .is_some_and(|v| objects.arena.truthy(v))
                    {
                        objects
                            .arena
                            .set_field(
                                child,
                                objects.phases.k(Field::Mandatory),
                                ObjectValue::Atom(Datum::Bool(true)),
                            )
                            .map_err(unsupported)?;
                        Some((
                            Field::TargetFilters,
                            self.symbols.lookup("last_flurry_target").unwrap(),
                        ))
                    } else {
                        None
                    }
                } else if plan.exclude_cloak
                    && kind == "attack_sequence"
                    && (!objects
                        .arena
                        .field(child, objects.phases.k(Field::AttackTargetKey))
                        .is_some_and(|v| objects.arena.truthy(&v))
                        || objects
                            .arena
                            .field(child, objects.phases.k(Field::AttackOutputKey))
                            .is_some_and(|v| objects.arena.truthy(&v)))
                {
                    Some((
                        Field::TargetFilters,
                        objects.phases.k(Field::LastCombatTarget),
                    ))
                } else if plan.exclude_cloak
                    && kind == "select_step"
                    && matches!(objects.arena.field(child, objects.phases.k(Field::TargetType)),
                    Some(ObjectValue::Atom(Datum::Text(value))) if self.symbols.name(value) == "UNIT")
                {
                    Some((Field::Filters, objects.phases.k(Field::LastCombatTarget)))
                } else {
                    None
                };
                if let Some((field, key)) = exclusion {
                    let Some(ObjectValue::Object(filters)) =
                        objects.arena.field(child, objects.phases.k(field))
                    else {
                        return Err(unsupported("dynamic passive filter list missing"));
                    };
                    let ObjectValue::Object(filter) =
                        objects.arena.allocate(&program.action_bodies.exclusion)
                    else {
                        unreachable!()
                    };
                    objects
                        .arena
                        .classify_recipe_object(filter, &[], ObjectKind::Model)
                        .map_err(unsupported)?;
                    objects
                        .arena
                        .set_field(
                            filter,
                            self.symbols.lookup("exclude_self").unwrap(),
                            ObjectValue::Atom(Datum::Bool(true)),
                        )
                        .map_err(unsupported)?;
                    let keys = objects
                        .arena
                        .list(vec![ObjectValue::Atom(Datum::Text(key))]);
                    objects
                        .arena
                        .set_field(filter, objects.phases.k(Field::ExcludeKeys), keys)
                        .map_err(unsupported)?;
                    objects
                        .arena
                        .append(filters, ObjectValue::Object(filter))
                        .map_err(unsupported)?;
                }
                queued.push(child);
            }
            if let Some(suffix) = suffix {
                queued.push(suffix);
            }
        }
        let mark = self.phase_step(objects, "mark_passive")?;
        for (field, value) in [
            (Field::CardId, Datum::Text(plan.printed)),
            (
                Field::PassiveProvider,
                plan.provider.map_or(Datum::Null, Datum::Text),
            ),
        ] {
            objects
                .arena
                .set_field(mark, objects.phases.k(field), ObjectValue::Atom(value))
                .map_err(unsupported)?;
        }
        queued.push(mark);
        self.push_live_steps_bound(objects, &queued)
    }
    pub(super) fn allocate_source_recipes_bound<'a>(
        &mut self,
        objects: &mut BoundObjects,
        recipes: impl IntoIterator<Item = (&'a Recipe, Symbol)>,
    ) -> Result<Vec<ObjectId>> {
        let mut queued = Vec::new();
        for (recipe, owner) in recipes {
            let step =
                self.allocate_source_recipe_value_bound(objects, recipe, &Datum::Text(owner))?;
            for (path, back, original) in &recipe.imports {
                let index = queued
                    .len()
                    .checked_sub(*back)
                    .ok_or_else(|| unsupported("recipe import group incomplete"))?;
                let target = objects
                    .arena
                    .recipe_object(queued[index], original)
                    .map_err(unsupported)?;
                objects
                    .arena
                    .alias_recipe_object(step, path, target)
                    .map_err(unsupported)?;
            }
            queued.push(step);
        }
        Ok(queued)
    }
    pub(super) fn allocate_source_recipe_bound(
        &mut self,
        objects: &mut BoundObjects,
        recipe: &Recipe,
        owner: &Datum,
    ) -> Result<ObjectId> {
        if !recipe.imports.is_empty() {
            return Err(unsupported("shared recipe needs grouped allocation"));
        }
        self.allocate_source_recipe_value_bound(objects, recipe, owner)
    }
    fn allocate_source_recipe_value_bound(
        &mut self,
        objects: &mut BoundObjects,
        recipe: &Recipe,
        owner: &Datum,
    ) -> Result<ObjectId> {
        let program = objects.scheduler.as_ref().unwrap().clone();
        let step = objects
            .arena
            .step_substituting_value(&recipe.step.fields, program.passive_bodies.owner, owner)
            .map_err(unsupported)?;
        for (path, kind) in &recipe.objects {
            let object = objects
                .arena
                .classify_recipe_object(step, path, *kind)
                .map_err(unsupported)?;
            if *kind == ObjectKind::Step {
                self.next_step += 1;
                objects
                    .arena
                    .set_field(
                        object,
                        program.step_id,
                        ObjectValue::GeneratedStep(self.next_step),
                    )
                    .map_err(unsupported)?;
            }
        }
        for (path, original) in &recipe.aliases {
            let target = objects
                .arena
                .recipe_object(step, original)
                .map_err(unsupported)?;
            objects
                .arena
                .alias_recipe_object(step, path, target)
                .map_err(unsupported)?;
        }
        if let Some((field, anchor)) = recipe.topology_anchor {
            objects
                .arena
                .set_field(step, field, ObjectValue::Object(anchor))
                .map_err(unsupported)?;
        }
        self.bind_body_program_bound(objects, step, recipe.step.instruction)?;
        for (path, instruction) in &recipe.programs {
            let object = objects
                .arena
                .recipe_object(step, path)
                .map_err(unsupported)?;
            self.bind_body_program_bound(objects, object, *instruction)?;
        }
        Ok(step)
    }
    fn bind_body_program_bound(
        &self,
        objects: &mut BoundObjects,
        object: ObjectId,
        instruction: usize,
    ) -> Result<()> {
        objects.bind_copied_program(object, instruction);
        if let Instruction::ForEach { templates, .. } | Instruction::LiveRepeat { templates } =
            &self.steps[instruction].instruction
        {
            let children = objects.template_objects(object)?;
            if children.len() != templates.len() {
                return Err(unsupported("passive nested template drift"));
            }
            for (object, instruction) in children.into_iter().zip(templates) {
                self.bind_body_program_bound(objects, object, *instruction)?;
            }
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
            .find(|c| c["name"] == name)
            .unwrap();
        let case: Value = serde_json::from_reader(std::io::BufReader::new(
            crate::fixture_cache::open(base.join(entry["path"].as_str().unwrap())).unwrap(),
        ))
        .unwrap();
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    fn empty_zone_lane_checks_use_live_operands_without_generation_or_shared_root_mutation() {
        let mut root =
            session("object-execution-dynamic-passive-flurry_of_blows-crane_stance-ordinary");
        root.advance_prefix(1).unwrap();
        let before = root.export_envelope().unwrap();
        let mut worker = root.clone();
        worker.next_step = u64::MAX;
        worker.advance_prefix(1).unwrap();
        assert_eq!(root.export_envelope().unwrap().state, before.state);
        for nonempty in [false, true] {
            let mut worker = root.clone();
            worker
                .with_live_objects(|session, objects| {
                    if nonempty {
                        let zones = objects.arena.allocate(&Datum::Object(vec![(
                            objects.phases.k(Field::Red),
                            Datum::Null,
                        )]));
                        objects
                            .arena
                            .set_root(objects.phases.k(Field::RepeatZones), zones)
                            .map_err(unsupported)?;
                    } else {
                        let step = objects
                            .steps_in_list(objects.stack)?
                            .last()
                            .copied()
                            .unwrap();
                        let lane = session.symbols.lookup("lane_id").unwrap();
                        objects
                            .arena
                            .set_field(
                                step,
                                lane,
                                ObjectValue::Atom(Datum::Text(objects.phases.k(Field::Red))),
                            )
                            .map_err(unsupported)?;
                    }
                    Ok(())
                })
                .unwrap();
            worker.advance_prefix(1).unwrap();
            assert_eq!(root.export_envelope().unwrap().state, before.state);
        }
    }
    #[test]
    fn dynamic_rebuild_generation_guards_leave_offer_and_arena_unchanged_across_restore() {
        for (name, flurry) in [
            (
                "object-execution-passive-body-flurry_of_blows-after_attack-accepted",
                true,
            ),
            (
                "object-execution-passive-body-cloak_and_daggers-after_basic_action-attack",
                false,
            ),
        ] {
            let mut session = session(name);
            session
                .with_live_objects(|session, objects| {
                    let owner = session.planning.actor.unwrap();
                    let card = session.world.hero(owner).unwrap().hand[0];
                    let definition = &session.world.definitions.cards
                        [session.world.card(card).unwrap().definition];
                    let printed = definition.printed_id;
                    let effect = definition.effect;
                    let model = session.hero_model_bound(objects, owner).unwrap();
                    objects
                        .arena
                        .set_field(
                            model,
                            objects.phases.k(Field::Current),
                            ObjectValue::Atom(Datum::Instance(card.0)),
                        )
                        .map_err(unsupported)?;
                    if flurry {
                        objects
                            .arena
                            .set_context(
                                objects.phases.k(Field::AttackCard),
                                ObjectValue::Atom(Datum::Text(printed)),
                            )
                            .map_err(unsupported)?;
                        objects
                            .arena
                            .set_context(
                                objects.phases.k(Field::AttackEffect),
                                ObjectValue::Atom(Datum::Text(effect)),
                            )
                            .map_err(unsupported)?;
                    } else {
                        objects
                            .arena
                            .set_context(
                                objects.phases.k(Field::BasicActionEffect),
                                ObjectValue::Atom(Datum::Text(effect)),
                            )
                            .map_err(unsupported)?;
                    }
                    session.sync_card_collections_bound(objects)
                })
                .unwrap();
            let before = session.export_envelope().unwrap();
            let mut restored = NativeSession::compile(before.clone()).unwrap();
            session.next_step = u64::MAX;
            restored.next_step = u64::MAX;
            for worker in [&mut session, &mut restored] {
                assert!(
                    worker
                        .advance_prefix(1)
                        .unwrap_err()
                        .to_string()
                        .contains("step generation counter exhausted")
                );
                assert_eq!(worker.export_envelope().unwrap().state, before.state);
                assert_eq!(
                    worker.export_envelope().unwrap().object_bindings,
                    before.object_bindings
                );
            }
        }
    }
}
