//! Action menus reuse typed rules; child models bind through source recipes.
use super::passive_bodies::Recipe;
use super::phases::Field;
use super::{
    ActionResolution, ActionStep, Arc, BoundObjects, CompiledStep, Datum, HashMap, Instant,
    NativeRequest, NativeSession, ObjectId, ObjectKind, Passive, RequestChoices, RequestToken,
    Result, RuleStepTimer, Symbol, Symbols, Value, unsupported,
};
use crate::data::Registers;
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub(super) struct ActionBindings {
    recipes: HashMap<&'static str, Vec<Recipe>>,
    pub(super) exclusion: Datum,
    pub(super) clear_filters: Vec<crate::targeting::UnitFilter>,
    card: Symbol,
    key: Symbol,
    data: Symbol,
    log: Symbol,
}
impl ActionBindings {
    pub(super) fn recipes(&self, name: &str) -> &[Recipe] {
        &self.recipes[name]
    }
    pub(super) fn travel_recipe(&self, selector: bool) -> &Recipe {
        &self.recipes[if selector {
            "travel_select"
        } else {
            "travel_unit"
        }][0]
    }
    pub(super) fn compile(steps: &mut Vec<CompiledStep>, symbols: &mut Symbols) -> Result<Self> {
        symbols.intern("Choose which of you performs this action.");
        symbols.intern("card");
        static DATA: OnceLock<std::result::Result<Value, String>> = OnceLock::new();
        let data = DATA
            .get_or_init(|| {
                let data: Value = serde_json::from_str(include_str!(
                    "../../../../../content/catalogs/action-builders.json"
                ))
                .map_err(|e| e.to_string())?;
                if data["schema_version"] != 1
                    || data["source_sha256"] != crate::codec::oracle_manifest()?["source_sha256"]
                {
                    return Err("action constructor source mismatch".into());
                }
                Ok(data)
            })
            .as_ref()
            .map_err(|e| unsupported(e.clone()))?;
        let mut recipes = HashMap::new();
        for name in [
            "restore_context",
            "restore_other_context",
            "piece",
            "passive",
            "pre_move",
            "pre_discard",
            "text",
            "movement",
            "travel",
            "travel_unit",
            "travel_select",
            "attack",
            "log",
            "context",
            "clear",
        ] {
            recipes.insert(
                name,
                data["recipes"][name]
                    .as_array()
                    .ok_or_else(|| unsupported("action recipe missing"))?
                    .iter()
                    .map(|row| Recipe::compile(row, steps, symbols))
                    .collect::<Result<Vec<_>>>()?,
            );
        }
        for trigger in [
            Passive::BeforeAction,
            Passive::BeforeAttack,
            Passive::BeforeMovement,
            Passive::BeforeSkill,
            Passive::AfterAttack,
            Passive::AfterMovement,
            Passive::AfterBasicSkill,
            Passive::AfterBasicAction,
            Passive::AfterPrimaryAction,
            Passive::AfterResolveCard,
        ] {
            symbols.intern(trigger.name());
        }
        let clear_filters = data["recipes"]["clear"][0]["fields"]["filters"]
            .as_array()
            .ok_or_else(|| unsupported("clear filters missing"))?
            .iter()
            .map(|wire| crate::targeting::compile_unit_filter(wire, symbols).map_err(unsupported))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            exclusion: Datum::from_json(&data["exclusion_filter"], symbols).map_err(unsupported)?,
            clear_filters,
            recipes,
            card: symbols.intern("template-action-card"),
            key: symbols.intern("template-action-key"),
            data: symbols.intern("template-action-value"),
            log: symbols.intern("template-action-log"),
        })
    }
}
pub(super) struct CardActionPlan {
    card: Option<Datum>,
    action: Option<Datum>,
    hero: Symbol,
    recipes: Vec<Recipe>,
    request: Option<NativeRequest>,
}
impl NativeSession {
    pub(super) fn preflight_card_action_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
        mut profile: Option<&mut RuleStepTimer>,
    ) -> Result<CardActionPlan> {
        let hero = self.passive_text_bound(objects, step, Field::HeroId)?;
        self.sync_card_collections_bound(objects)?;
        let mut context = Registers::default();
        let inputs = self.spatial_inputs_bound(
            objects,
            [
                objects.phases.k(Field::PerformingCard),
                objects.phases.k(Field::PerformingOwner),
            ],
        );
        for (key, value) in inputs.0 {
            context.set(key, value);
        }
        let selected = objects
            .selected(step, self.selection_key)
            .map(|value| {
                objects
                    .arena
                    .typed_value(&value, Arc::make_mut(&mut self.symbols))
                    .map_err(unsupported)
            })
            .transpose()?;
        let started = profile.as_ref().map(|_| Instant::now());
        let resolution = self
            .action_rules
            .resolve(&self.world, hero, selected.as_ref(), &mut context)
            .map_err(unsupported)?;
        if let Some((profile, started)) = profile.as_deref_mut().zip(started) {
            profile.action_rules_ns = Some(started.elapsed().as_nanos());
            profile.category = Some(match &resolution {
                ActionResolution::Prompt(_) => "resolve_card.menu",
                ActionResolution::Finished(children) if children.is_empty() => {
                    "resolve_card.no_action"
                }
                ActionResolution::Finished(_) => "resolve_card.response",
            });
        }
        let card = context.get(objects.action_keys[1]).cloned();
        let mut plan = CardActionPlan {
            card,
            action: None,
            hero,
            recipes: Vec::new(),
            request: None,
        };
        let children = match resolution {
            ActionResolution::Prompt(menu) => {
                if self.next_request == u64::MAX && objects.token(step)?.is_none() {
                    return Err(unsupported("request generation counter exhausted"));
                }
                plan.request = Some(NativeRequest {
                    controlled: None,
                    id: RequestToken::Generated(0),
                    owner: hero,
                    prompt: menu.name,
                    movement_range: None,
                    pre_action_range: None,
                    choices: RequestChoices::Action(menu),
                    can_skip: false,
                    discard: false,
                });
                return Ok(plan);
            }
            ActionResolution::Finished(children) => children,
        };
        plan.action = context.get(objects.action_keys[0]).cloned();
        let started = profile.as_ref().map(|_| Instant::now());
        plan.recipes = self.prepare_action_recipes_bound(objects, hero, children)?;
        let count = plan
            .recipes
            .iter()
            .flat_map(|r| &r.objects)
            .filter(|(_, kind)| *kind == ObjectKind::Step)
            .count();
        if count != 0 {
            self.phase_generation_check(
                0,
                u64::try_from(count)
                    .map_err(|_| unsupported("action generation count overflow"))?,
            )?;
        }
        if let Some((profile, started)) = profile.zip(started) {
            profile.recipe_prep_ns = Some(started.elapsed().as_nanos());
        }
        Ok(plan)
    }
    pub(super) fn prepare_action_recipes_bound(
        &mut self,
        objects: &BoundObjects,
        hero: Symbol,
        children: Vec<ActionStep>,
    ) -> Result<Vec<Recipe>> {
        let mut recipes = Vec::new();
        let rules = objects.scheduler.as_ref().unwrap().action_bodies.clone();
        for child in children {
            let (name, card, value, range, key, data, log) = match child {
                ActionStep::ChoosePiece(_) => ("piece", None, 0, 0, None, None, None),
                ActionStep::Passive(trigger) => (
                    "passive",
                    None,
                    0,
                    0,
                    None,
                    Some(Datum::Text(self.symbols.lookup(trigger.name()).unwrap())),
                    None,
                ),
                ActionStep::PreMovement(_) => ("pre_move", None, 0, 0, None, None, None),
                ActionStep::PreDiscard(_) => ("pre_discard", None, 0, 0, None, None, None),
                ActionStep::CardText { card, .. } => ("text", Some(card), 0, 0, None, None, None),
                ActionStep::Movement { base, .. } => ("movement", None, base, 0, None, None, None),
                ActionStep::FastTravel { card, .. } => {
                    ("travel", Some(card), 0, 0, None, None, None)
                }
                ActionStep::Attack { damage, range } => {
                    ("attack", None, damage, range, None, None, None)
                }
                ActionStep::Clear if !self.world.has_board_presence(hero) => {
                    let text = format!(
                        "{} attempted clear but is not on board.",
                        self.symbols.name(hero)
                    );
                    (
                        "log",
                        None,
                        0,
                        0,
                        None,
                        None,
                        Some(Arc::make_mut(&mut self.symbols).intern(&text)),
                    )
                }
                ActionStep::Clear => ("clear", None, 0, 0, None, None, None),
                ActionStep::Hold(id) => {
                    let text = format!("{} Holds.", self.symbols.name(id));
                    (
                        "log",
                        None,
                        0,
                        0,
                        None,
                        None,
                        Some(Arc::make_mut(&mut self.symbols).intern(&text)),
                    )
                }
                ActionStep::Defend(id) => {
                    let text = format!("{} Defends (Active).", self.symbols.name(id));
                    (
                        "log",
                        None,
                        0,
                        0,
                        None,
                        None,
                        Some(Arc::make_mut(&mut self.symbols).intern(&text)),
                    )
                }
                ActionStep::PerformLog(id, kind) => {
                    let text = format!("{} performs {}.", self.symbols.name(id), kind.name());
                    (
                        "log",
                        None,
                        0,
                        0,
                        None,
                        None,
                        Some(Arc::make_mut(&mut self.symbols).intern(&text)),
                    )
                }
                ActionStep::Context(key, value) => {
                    ("context", None, 0, 0, Some(key), Some(value), None)
                }
            };
            struct Operands {
                card: Option<Symbol>,
                value: i64,
                range: i64,
                key: Option<Symbol>,
                data: Option<Datum>,
                log: Option<Symbol>,
            }
            fn bind(target: &mut Datum, rules: &ActionBindings, operands: &Operands) {
                match target {
                    Datum::Text(v) if *v == rules.card => {
                        if let Some(card) = operands.card {
                            *v = card;
                        }
                    }
                    Datum::Text(v) if *v == rules.key => {
                        if let Some(key) = operands.key {
                            *v = key;
                        }
                    }
                    Datum::Text(v) if *v == rules.data => {
                        if let Some(data) = &operands.data {
                            *target = data.clone();
                        }
                    }
                    Datum::Text(v) if *v == rules.log => {
                        if let Some(log) = operands.log {
                            *v = log;
                        }
                    }
                    Datum::Int(v) if *v == 62003 => *v = operands.value,
                    Datum::Int(v) if *v == 62007 => *v = operands.range,
                    Datum::List(values) => {
                        for v in values {
                            bind(v, rules, operands);
                        }
                    }
                    Datum::Object(fields) => {
                        for (_, v) in fields {
                            bind(v, rules, operands);
                        }
                    }
                    _ => {}
                }
            }
            let operands = Operands {
                card,
                value,
                range,
                key,
                data,
                log,
            };
            for recipe in &rules.recipes[name] {
                let mut recipe = recipe.clone();
                bind(&mut recipe.step.fields, &rules, &operands);
                recipes.push(recipe);
            }
        }
        Ok(recipes)
    }
    pub(super) fn card_action_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: CardActionPlan,
    ) -> Result<Option<NativeRequest>> {
        objects.arena.symbols = self.symbols.clone();
        if let Some(card) = plan.card {
            objects.write(objects.action_keys[1], card)?;
        }
        if let Some(action) = plan.action {
            objects.write(objects.action_keys[0], action)?;
        }
        let mut children = Vec::with_capacity(plan.recipes.len());
        for recipe in &plan.recipes {
            let step =
                self.allocate_source_recipe_bound(objects, recipe, &Datum::Text(plan.hero))?;
            Self::validate_returned_step_bound(objects, step)?;
            children.push(step);
        }
        self.push_live_steps_bound(objects, &children)?;
        Ok(plan.request)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ObjectValue, fixture_case};
    use super::*;
    fn session(name: &str) -> NativeSession {
        let case = fixture_case(&format!("object-execution-card-action-edge-{name}"));
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    fn card_action_generation_guard_precedes_context_or_stack_writes() {
        let mut worker = session("negative-attack");
        worker.next_step = u64::MAX;
        let before = worker.export_envelope().unwrap();
        assert!(
            worker
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("step generation counter exhausted")
        );
        let after = worker.export_envelope().unwrap();
        assert_eq!(before.state, after.state);
        assert_eq!(before.object_bindings, after.object_bindings);
    }
    #[test]
    fn piece_prompt_generation_guard_preserves_the_live_step() {
        let mut worker = session("piece-input-first");
        worker.next_request = u64::MAX;
        let before = worker.export_envelope().unwrap();
        assert!(
            worker
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("request generation counter exhausted")
        );
        let after = worker.export_envelope().unwrap();
        assert_eq!(before.state, after.state);
        assert_eq!(before.object_bindings, after.object_bindings);
    }
    #[test]
    fn live_card_actor_operand_is_private_and_rebound_before_execution() {
        let root = session("negative-attack");
        let before = root.export_envelope().unwrap();
        let mut worker = root.clone();
        let step = worker.stack.last().unwrap().object.unwrap();
        let absent = Arc::make_mut(&mut worker.symbols).intern("absent");
        let objects = worker.objects.as_mut().unwrap();
        objects
            .arena
            .set_field(
                step,
                objects.phases.k(Field::HeroId),
                ObjectValue::Atom(Datum::Text(absent)),
            )
            .unwrap();
        worker.advance_prefix(1).unwrap();
        assert!(worker.stack.is_empty());
        assert_eq!(
            worker.export_envelope().unwrap().materialize().unwrap()["execution_context"]["current_card_id"],
            "old-card"
        );
        assert_eq!(root.export_envelope().unwrap().state, before.state);
    }
    #[test]
    fn card_without_current_face_needs_no_request_or_step_capacity() {
        let mut worker = session("no-card");
        worker.next_request = u64::MAX;
        worker.next_step = u64::MAX;
        assert!(worker.advance_prefix(1).unwrap().request.is_none());
        assert!(worker.stack.is_empty());
    }
    #[test]
    fn menu_request_guard_precedes_current_card_context_update() {
        let mut worker = session("piece-operand");
        worker.next_request = u64::MAX;
        let before = worker.export_envelope().unwrap();
        assert!(
            worker
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("request generation counter exhausted")
        );
        let after = worker.export_envelope().unwrap();
        assert_eq!(before.state, after.state);
        assert_eq!(before.object_bindings, after.object_bindings);
    }
    #[test]
    fn live_piece_actor_can_autobind_without_request_capacity() {
        let root = session("piece-input-first");
        let before = root.export_envelope().unwrap();
        let mut worker = root.clone();
        worker.next_request = u64::MAX;
        let piece = worker.symbols.lookup("hero_razzle_piece_1").unwrap();
        let step = worker.stack.last().unwrap().object.unwrap();
        let objects = worker.objects.as_mut().unwrap();
        objects
            .arena
            .set_field(
                step,
                objects.phases.k(Field::HeroId),
                ObjectValue::Atom(Datum::Text(piece)),
            )
            .unwrap();
        assert!(worker.advance_prefix(1).unwrap().request.is_none());
        assert_eq!(worker.world.acting_piece, Some(piece));
        assert!(worker.stack.is_empty());
        assert_eq!(root.export_envelope().unwrap().state, before.state);
    }
}
