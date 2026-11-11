//! Live shared-object execution and controlled effect/card-lifecycle bindings.
//! Other world-changing instructions stay guarded until their dense slots bind.
use super::{
    ActionResolution, ActionStep, AdvanceResult, Arc, CardStats, CombatEvent, CompiledStep,
    CoordinateOp, Datum, DefeatBinding, EngineError, EntityKind, ExecutionBudget, Frame,
    GameOutcome, Hex, Instant, Instruction, MarkerEvent, NativeEvent, NativeRequest, NativeSession,
    Passive, Phase, Registers, RequestChoices, RequestToken, Result, Stat, StateEnvelope, Symbol,
    Symbols, Team, Value, card_error, combat_error, combat_template, compile_step, json,
    native_hex, prompt_actor, unsupported,
};
mod acting_piece;
mod action_control;
mod actor;
#[cfg(test)]
mod arien_tests;
mod attack_sequence;
#[cfg(test)]
mod battle_tests;
mod battles;
mod card_actions;
mod card_guess;
mod card_selection;
mod card_swap;
mod cleanup;
mod co_movement;
mod combat_resolution;
mod coordinates;
mod defeat;
mod defense_bodies;
mod directional_move;
mod discard;
mod effect_creation;
mod forced_discard;
#[cfg(test)]
mod gydion_tests;
mod hero_respawn;
mod hex_selection;
mod initiative;
mod journey;
mod markers;
mod min_cards;
#[cfg(test)]
mod min_tests;
#[cfg(test)]
mod mind_grip_tests;
mod mines;
mod minion_respawn;
mod minion_return;
#[cfg(test)]
mod misa_tests;
mod movement;
mod movement_sequence;
mod multi_selection;
mod passive_bodies;
mod passives;
mod perform_card;
mod perform_primary;
#[cfg(test)]
mod phase_closure_tests;
mod phases;
mod placement;
mod planning_commands;

pub(crate) use planning_commands::CardSyncProfile;
pub(super) fn with_card_sync_profile<T>(run: impl FnOnce() -> T) -> (T, CardSyncProfile) {
    planning_commands::with_card_sync_profile(run)
}
mod pre_action;
mod primary_bodies;
mod primary_operands;
mod push;
mod queries;
mod reaction_window;
mod repeat;
mod resolution;
mod resources;
mod session_rollback;
pub(super) use session_rollback::RollbackCheckpoint;
mod retrieval;
mod revelation;
mod rock_ultimate;
mod rune_choice;
mod spell_bodies;
mod spells;
mod starting_positions;
mod step_isolation;
mod table_card_patch;
mod table_effect_patch;
mod table_patch;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tigerclaw_tests;
mod token_batch;
mod token_paths;
mod tokens;
mod travel;
mod turrets;
mod unit_selection;
mod upgrades;
#[cfg(test)]
mod utility_tests;
#[cfg(test)]
mod wasp_tests;
use crate::{
    codec::{ObjectKind, ObjectPathPart},
    objects::{ObjectArena, ObjectId, ObjectValue},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
};

#[derive(Debug, Default, serde::Serialize)]
pub(super) struct RuleStepProfile {
    calls: usize,
    elapsed_ns: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    action_rules_ns: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recipe_prep_ns: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    action_apply_ns: Option<u128>,
}

impl RuleStepProfile {
    fn add_timing(total: &mut Option<u128>, sample: Option<u128>) {
        if let Some(sample) = sample {
            *total.get_or_insert(0) += sample;
        }
    }
}

pub(super) type RuleStepProfiles = BTreeMap<String, RuleStepProfile>;

thread_local! {
    static RULE_STEP_PROFILE: RefCell<Option<RuleStepProfiles>> = const { RefCell::new(None) };
}

pub(super) fn with_rule_step_profile<T>(run: impl FnOnce() -> T) -> (T, RuleStepProfiles) {
    struct Restore(Option<RuleStepProfiles>);
    impl Drop for Restore {
        fn drop(&mut self) {
            RULE_STEP_PROFILE.replace(self.0.take());
        }
    }

    let _restore = Restore(RULE_STEP_PROFILE.replace(Some(BTreeMap::new())));
    let result = run();
    (result, RULE_STEP_PROFILE.replace(None).unwrap())
}

struct RuleStepTimer {
    program: CompiledStep,
    start: Instant,
    category: Option<&'static str>,
    action_rules_ns: Option<u128>,
    recipe_prep_ns: Option<u128>,
    action_apply_ns: Option<u128>,
}

impl Drop for RuleStepTimer {
    fn drop(&mut self) {
        let elapsed_ns = self.start.elapsed().as_nanos();
        let name = self
            .category
            .or_else(|| self.program.wire["type"].as_str())
            .unwrap_or("unknown");
        RULE_STEP_PROFILE.with_borrow_mut(|profile| {
            if let Some(profile) = profile {
                let row = match profile.get_mut(name) {
                    Some(row) => row,
                    None => profile.entry(name.to_owned()).or_default(),
                };
                row.calls += 1;
                row.elapsed_ns += elapsed_ns;
                RuleStepProfile::add_timing(&mut row.action_rules_ns, self.action_rules_ns);
                RuleStepProfile::add_timing(&mut row.recipe_prep_ns, self.recipe_prep_ns);
                RuleStepProfile::add_timing(&mut row.action_apply_ns, self.action_apply_ns);
            }
        });
    }
}

pub(super) fn primary_operand_catalog() -> Result<&'static Value> {
    primary_bodies::catalog()
}

/// Natural turn expiry retains the original finishing list identities, without
/// removing effects. Rule-v9 turn scheduling removes these IDs after the text.
#[derive(Clone, Debug)]
pub struct LiveEffectExpiry {
    pub finishing: Vec<(Symbol, ObjectId)>,
    pub ids: Vec<Symbol>,
}
#[derive(Clone, Debug)]
struct StepRecipe {
    instruction: usize,
    fields: Datum,
}
#[derive(Clone, Debug)]
struct FinishingProgram {
    actor: StepRecipe,
    marker: StepRecipe,
    step_id: Symbol,
    actor_id: Symbol,
    phase_recipes: Vec<(&'static str, StepRecipe)>,
    passive_bodies: passive_bodies::BodyRules,
    defense_bodies: Arc<defense_bodies::DefenseRules>,
    primary_bodies: Arc<primary_bodies::PrimaryRules>,
    action_bodies: Arc<card_actions::ActionBindings>,
    journey_finishing: Vec<passive_bodies::Recipe>,
}
#[derive(Clone, Debug)]
pub(super) struct BoundObjects {
    pub arena: ObjectArena,
    stack: ObjectId,
    readonly: std::sync::OnceLock<Arc<crate::objects::ProtectedObjects>>,
    pending: Symbol,
    request: Symbol,
    value: Symbol,
    index: Symbol,
    templates: Symbol,
    programs: Arc<HashMap<ObjectId, usize>>,
    copied_programs: HashMap<ObjectId, usize>,
    copied_readonly: HashSet<ObjectId>,
    copied_binding_undo: CopiedBindingJournal,
    action_keys: [Symbol; 8],
    action_stack: Symbol,
    present: Symbol,
    type_stack: Symbol,
    actor: Symbol,
    effects: Symbol,
    effect_id: Symbol,
    active: Symbol,
    finishing: Symbol,
    number_options: Symbol,
    scheduler: Option<Arc<FinishingProgram>>,
    phases: phases::PhaseBindings,
    tokens: tokens::TokenBindings,
    effect_creation: Arc<effect_creation::EffectBindings>,
}

#[derive(Clone, Copy, Debug)]
enum CopiedBinding {
    Program(ObjectId, Option<usize>),
    Protection(ObjectId),
}

#[derive(Debug, Default)]
struct CopiedBindingJournal(Vec<CopiedBinding>);

impl Clone for CopiedBindingJournal {
    fn clone(&self) -> Self {
        Self::default()
    }
}

pub(super) struct SharedPrimaryPrograms {
    pub symbols: Symbols,
    pub steps: Vec<CompiledStep>,
    rules: Arc<primary_bodies::PrimaryRules>,
}

pub(super) fn shared_primary_programs() -> Result<&'static SharedPrimaryPrograms> {
    static PROGRAMS: std::sync::OnceLock<Result<SharedPrimaryPrograms>> =
        std::sync::OnceLock::new();
    PROGRAMS
        .get_or_init(|| {
            let mut symbols = Symbols::default();
            let mut steps = Vec::new();
            let rules = Arc::new(primary_bodies::PrimaryRules::compile(
                &mut steps,
                &mut symbols,
            )?);
            symbols.freeze_shared();
            Ok(SharedPrimaryPrograms {
                symbols,
                steps,
                rules,
            })
        })
        .as_ref()
        .map_err(Clone::clone)
}

impl BoundObjects {
    pub(super) fn hash_execution(&self, state: &mut impl std::hash::Hasher) {
        use std::hash::Hash;

        self.arena.hash_execution(state);
        self.stack.hash(state);
        for bindings in [self.programs.as_ref(), &self.copied_programs] {
            let mut entries: Vec<_> = bindings.iter().collect();
            entries.sort_unstable_by_key(|(id, _)| **id);
            entries.hash(state);
        }
        let mut readonly: Vec<_> = self.copied_readonly.iter().copied().collect();
        readonly.sort_unstable();
        readonly.hash(state);
    }

    pub(super) fn compile_validated(
        source: &StateEnvelope,
        symbols: Symbols,
    ) -> Result<Option<Self>> {
        if source.object_bindings.is_empty() {
            return Ok(None);
        }
        for step in source.state["execution_stack"]
            .as_array()
            .ok_or_else(|| unsupported("stack missing"))?
        {
            if !Self::supported(step) {
                return Err(unsupported(
                    "shared-object instruction family not yet bound",
                ));
            }
        }
        let mut arena = ObjectArena::compile_validated(source, symbols).map_err(unsupported)?;
        for key in [
            "attempts",
            "attempt",
            "victim_id",
            "card_id",
            "guessed_color",
            "actual_color",
            "correct",
            "victim_key",
            "card_key",
            "guess_key",
            "correct_output_key",
            "wrong_output_key",
            "rollback_reanchor_pending",
            "output_key",
            "selection_announced",
            "BLUE",
            "GOLD",
            "GREEN",
            "RED",
            "SILVER",
            "Guess the card's color",
        ] {
            Arc::make_mut(&mut arena.symbols).intern(key);
        }
        let pending = Arc::make_mut(&mut arena.symbols).intern("pending_input");
        let request = Arc::make_mut(&mut arena.symbols).intern("pending_request_id");
        let value = Arc::make_mut(&mut arena.symbols).intern("value");
        let index = Arc::make_mut(&mut arena.symbols).intern("current_index");
        let templates = Arc::make_mut(&mut arena.symbols).intern("steps_template");
        let action_keys = [
            "current_action_type",
            "current_card_id",
            "performing_card_id",
            "performing_card_owner_id",
            "reperforming_card_id",
            "token_type_override",
            "skip_markers",
            "substitution_actor_id",
        ]
        .map(|key| Arc::make_mut(&mut arena.symbols).intern(key));
        let action_stack = Arc::make_mut(&mut arena.symbols).intern("action_context_stack");
        let present = Arc::make_mut(&mut arena.symbols).intern("present");
        let type_stack = Arc::make_mut(&mut arena.symbols).intern("action_type_stack");
        let actor = Arc::make_mut(&mut arena.symbols).intern("current_actor_id");
        let effects = Arc::make_mut(&mut arena.symbols).intern("active_effects");
        let effect_id = Arc::make_mut(&mut arena.symbols).intern("id");
        let active = Arc::make_mut(&mut arena.symbols).intern("is_active_base");
        let finishing = Arc::make_mut(&mut arena.symbols).intern("finishing_steps");
        let number_options = Arc::make_mut(&mut arena.symbols).intern("number_options");
        let ObjectValue::Object(stack) = arena
            .value_at_path(&[ObjectPathPart::Key("execution_stack".into())])
            .map_err(unsupported)?
        else {
            return Err(unsupported("stack object missing"));
        };
        if !arena.is_mutable_list(stack) {
            return Err(unsupported("stack is not a list"));
        }
        let readonly = Arc::new(arena.readonly_objects());
        if readonly.contains(&arena.context_id()) {
            return Err(unsupported(
                "execution context aliases an unbound world/step operand",
            ));
        }
        let phases = phases::PhaseBindings::compile(source, &mut arena)?;
        let tokens = tokens::TokenBindings::compile(source, &mut arena)?;
        let effect_creation = Arc::new(effect_creation::EffectBindings::compile(&mut arena)?);
        Ok(Some(Self {
            arena,
            stack,
            readonly: std::sync::OnceLock::from(readonly),
            pending,
            request,
            value,
            index,
            templates,
            programs: Arc::new(HashMap::new()),
            copied_programs: HashMap::new(),
            copied_readonly: HashSet::new(),
            copied_binding_undo: CopiedBindingJournal::default(),
            action_keys,
            action_stack,
            present,
            type_stack,
            actor,
            effects,
            effect_id,
            active,
            finishing,
            number_options,
            scheduler: None,
            phases,
            tokens,
            effect_creation,
        }))
    }
    fn supported(step: &Value) -> bool {
        match step["type"].as_str() {
            Some(
                "set_context_flag"
                | "add_context_value"
                | "record_target"
                | "log_message"
                | "restore_action_context"
                | "restore_action_type"
                | "set_actor"
                | "finished_expiring_effect"
                | "advance_turn"
                | "find_next_actor"
                | "end_phase"
                | "minion_battle"
                | "lane_push"
                | "choose_minion_removal"
                | "resolve_displacement"
                | "check_lane_push"
                | "end_phase_cleanup"
                | "resolve_upgrades"
                | "round_reset"
                | "return_minion_to_zone"
                | "mine_path_choice"
                | "trigger_mine"
                | "force_discard"
                | "discard_card"
                | "retrieve_card"
                | "check_passive_abilities"
                | "offer_passive"
                | "mark_passive_used"
                | "move_sequence"
                | "force_defense_card_movement"
                | "push_unit"
                | "record_hex"
                | "compute_hex"
                | "compute_distance"
                | "resolve_turn_start_hex"
                | "check_adjacency"
                | "may_repeat_once"
                | "razzle_mirrored_push"
                | "remove_hero_piece"
                | "spawn_hero_piece"
                | "set_acting_piece"
                | "choose_acting_piece"
                | "fast_travel_sequence"
                | "fast_travel_unit"
                | "perform_primary_action"
                | "perform_card_action"
                | "cast_spell"
                | "wish_sequence"
                | "prepare_spellbook"
                | "remove_prepared_spells"
                | "multi_select"
                | "collect_units"
                | "count"
                | "check_context_condition"
                | "check_solo_win"
                | "check_zone_change"
                | "count_adjacent_enemies"
                | "count_cards"
                | "check_hero_defeated_this_round"
                | "check_unit_filters"
                | "check_distance"
                | "check_unit_type"
                | "check_unit_on_board"
                | "combine_boolean_context"
                | "convert_card_to_item"
                | "apply_after_attack_card_text"
                | "respawn_minion_at_hex"
                | "force_discard_or_defeat"
                | "choose_rune"
                | "laugh"
                | "reveal_hand_card"
                | "guess_card_color"
                | "reveal_and_resolve_guess"
                | "place_runes"
                | "choose_card_color"
                | "directional_move_units"
                | "force_discard_by_color"
                | "swap_resolved_cards"
                | "swap_card"
                | "swap_item_card"
                | "swap_with_deck_card"
                | "defeat_unit"
                | "resolve_combat"
                | "snapshot_adjacent_heroes"
                | "check_target_defeated"
                | "remove_unit"
                | "trigger_game_over"
                | "check_minion_protection"
                | "place_token"
                | "remove_token"
                | "place_token_batch"
                | "offer_rock_ultimate"
                | "place_turret"
                | "remove_turret"
                | "place_tokens_in_line"
                | "place_token_trail"
                | "attack_sequence"
                | "reaction_window"
                | "resolve_card_text"
                | "resolve_defense_text"
                | "gain_coins"
                | "gain_item"
                | "lose_coins"
                | "steal_coins"
                | "restore_spent_life_counter"
                | "spend_additional_life_counter"
                | "resolve_on_block_effect"
                | "resolve_pre_action_movement"
                | "resolve_pre_action_discard"
                | "move_unit"
                | "move_token"
                | "move_toward_target"
                | "co_directional_drag"
                | "set_card_initiative"
                | "schedule_action_control"
                | "schedule_journey_return"
                | "place_unit"
                | "swap_units"
                | "create_effect"
                | "place_marker"
                | "remove_marker"
                | "retrieve_unresolved_card"
                | "resolve_tie_breaker"
                | "resolve_card"
                | "confirm_resolution"
                | "finalize_hero_turn"
                | "respawn_hero"
                | "flip_tie_breaker_coin",
            ) => true,
            Some("select_step") => matches!(
                step["target_type"].as_str(),
                Some("NUMBER" | "CARD" | "HEX" | "UNIT" | "UNIT_OR_TOKEN")
            ),
            Some("for_each") => step["steps_template"]
                .as_array()
                .is_some_and(|steps| steps.iter().all(Self::supported)),
            _ => false,
        }
    }
    fn parked_type(step: &Value) -> bool {
        (step["type"] == "select_step" && step["target_type"] != "NUMBER")
            || matches!(
                step["type"].as_str(),
                Some(
                    "resolve_card"
                        | "move_unit"
                        | "move_token"
                        | "move_toward_target"
                        | "co_directional_drag"
                        | "set_card_initiative"
                        | "schedule_action_control"
                        | "schedule_journey_return"
                        | "place_unit"
                        | "swap_units"
                        | "create_effect"
                        | "place_marker"
                        | "remove_marker"
                        | "mine_path_choice"
                        | "trigger_mine"
                        | "force_discard"
                        | "discard_card"
                        | "retrieve_card"
                        | "check_passive_abilities"
                        | "offer_passive"
                        | "mark_passive_used"
                        | "move_sequence"
                        | "force_defense_card_movement"
                        | "push_unit"
                        | "record_hex"
                        | "compute_hex"
                        | "compute_distance"
                        | "resolve_turn_start_hex"
                        | "check_adjacency"
                        | "may_repeat_once"
                        | "razzle_mirrored_push"
                        | "remove_hero_piece"
                        | "spawn_hero_piece"
                        | "set_acting_piece"
                        | "choose_acting_piece"
                        | "fast_travel_sequence"
                        | "fast_travel_unit"
                        | "perform_primary_action"
                        | "perform_card_action"
                        | "cast_spell"
                        | "wish_sequence"
                        | "prepare_spellbook"
                        | "remove_prepared_spells"
                        | "multi_select"
                        | "collect_units"
                        | "count"
                        | "check_context_condition"
                        | "check_solo_win"
                        | "check_zone_change"
                        | "count_adjacent_enemies"
                        | "count_cards"
                        | "check_hero_defeated_this_round"
                        | "check_unit_filters"
                        | "check_distance"
                        | "check_unit_type"
                        | "check_unit_on_board"
                        | "combine_boolean_context"
                        | "convert_card_to_item"
                        | "apply_after_attack_card_text"
                        | "respawn_minion_at_hex"
                        | "force_discard_or_defeat"
                        | "choose_rune"
                        | "laugh"
                        | "reveal_hand_card"
                        | "guess_card_color"
                        | "reveal_and_resolve_guess"
                        | "place_runes"
                        | "choose_card_color"
                        | "directional_move_units"
                        | "force_discard_by_color"
                        | "swap_resolved_cards"
                        | "swap_card"
                        | "swap_item_card"
                        | "swap_with_deck_card"
                        | "defeat_unit"
                        | "resolve_combat"
                        | "snapshot_adjacent_heroes"
                        | "check_target_defeated"
                        | "remove_unit"
                        | "trigger_game_over"
                        | "check_minion_protection"
                        | "place_token"
                        | "remove_token"
                        | "place_token_batch"
                        | "offer_rock_ultimate"
                        | "place_turret"
                        | "remove_turret"
                        | "place_tokens_in_line"
                        | "place_token_trail"
                        | "attack_sequence"
                        | "reaction_window"
                        | "resolve_card_text"
                        | "resolve_defense_text"
                        | "gain_coins"
                        | "gain_item"
                        | "lose_coins"
                        | "steal_coins"
                        | "restore_spent_life_counter"
                        | "spend_additional_life_counter"
                        | "resolve_on_block_effect"
                        | "resolve_pre_action_movement"
                        | "resolve_pre_action_discard"
                )
            )
    }
    pub fn compile_wire(
        wire: &Value,
        steps: &mut Vec<CompiledStep>,
        symbols: &mut Symbols,
    ) -> Result<usize> {
        fn sanitize(wire: &mut Value) {
            if BoundObjects::parked_type(wire) || wire["type"] == "return_minion_to_zone" {
                wire["type"] = json!("finished_expiring_effect");
            }
            if let Some(templates) = wire.get_mut("steps_template").and_then(Value::as_array_mut) {
                for child in templates {
                    sanitize(child);
                }
            }
        }
        fn restore(
            index: usize,
            wire: &Value,
            steps: &mut Vec<CompiledStep>,
            symbols: &mut Symbols,
        ) -> Result<()> {
            steps[index].wire = wire.clone();
            if wire["type"] == "return_minion_to_zone" {
                steps[index].instruction = Instruction::LiveReturnMinions;
            } else if wire["type"] == "select_step" && wire["target_type"] == "HEX" {
                steps[index].instruction = Instruction::LiveHexSelect;
            } else if wire["type"] == "select_step"
                && matches!(wire["target_type"].as_str(), Some("UNIT" | "UNIT_OR_TOKEN"))
            {
                steps[index].instruction = Instruction::LiveUnitSelect;
            } else if wire["type"] == "resolve_pre_action_movement" {
                steps[index].instruction = Instruction::LivePreMovement;
            } else if wire["type"] == "resolve_pre_action_discard" {
                steps[index].instruction = Instruction::LivePreDiscard;
            } else if wire["type"] == "reaction_window" {
                steps[index].instruction = Instruction::LiveReaction;
            } else if wire["type"] == "attack_sequence" {
                steps[index].instruction = Instruction::LiveAttackSequence;
            } else if wire["type"] == "resolve_combat" {
                steps[index].instruction = Instruction::LiveCombatResolve;
            } else if wire["type"] == "snapshot_adjacent_heroes" {
                steps[index].instruction = Instruction::LiveAdjacentSnapshot;
            } else if wire["type"] == "check_target_defeated" {
                steps[index].instruction = Instruction::LiveTargetDefeated;
            } else if wire["type"] == "defeat_unit" {
                steps[index].instruction = Instruction::LiveDefeat;
            } else if wire["type"] == "remove_unit" {
                steps[index].instruction = Instruction::LiveRemoveUnit;
            } else if wire["type"] == "trigger_game_over" {
                steps[index].instruction = Instruction::LiveGameOver;
            } else if wire["type"] == "check_minion_protection" {
                steps[index].instruction = Instruction::LiveProtection;
            } else if wire["type"] == "multi_select" {
                steps[index].instruction = Instruction::LiveMultiSelect;
            } else if wire["type"] == "check_context_condition" {
                steps[index].instruction = Instruction::LiveContextCondition;
            } else if matches!(wire["type"].as_str(), Some("count" | "collect_units")) {
                steps[index].instruction = Instruction::LiveQuery {
                    collect: wire["type"] == "collect_units",
                };
            } else if wire["type"] == "select_step" && wire["target_type"] == "CARD" {
                steps[index].instruction = Instruction::LiveCardSelect;
            } else if wire["type"] == "check_passive_abilities" {
                steps[index].instruction = Instruction::LivePassive;
            } else if wire["type"] == "offer_passive" {
                steps[index].instruction = Instruction::LiveOfferPassive;
            } else if wire["type"] == "place_runes" {
                steps[index].instruction = Instruction::LiveRuneInscription;
            } else if wire["type"] == "directional_move_units" {
                steps[index].instruction = Instruction::LiveDirectionalMove;
            } else if wire["type"] == "choose_card_color" {
                steps[index].instruction = Instruction::LiveChooseCardColor;
            } else if wire["type"] == "laugh" {
                steps[index].instruction = Instruction::LiveLaugh;
            } else if wire["type"] == "reveal_hand_card" {
                for key in [
                    "revealer_id",
                    "target_unit_id",
                    "owner_id",
                    "card_id",
                    "tier_value",
                    "rollback_reanchor_pending",
                    "card_reveal",
                ] {
                    symbols.intern(key);
                }
                steps[index].instruction = Instruction::LiveRevealHandCard;
            } else if matches!(
                wire["type"].as_str(),
                Some("guess_card_color" | "reveal_and_resolve_guess")
            ) {
                steps[index].instruction = Instruction::LiveCardGuess {
                    reveal: wire["type"] == "reveal_and_resolve_guess",
                };
            } else if wire["type"] == "choose_rune" {
                steps[index].instruction = Instruction::LiveRuneChoice;
            } else if wire["type"] == "mark_passive_used" {
                steps[index].instruction = Instruction::LiveMarkPassive;
            } else if wire["type"] == "place_token" {
                steps[index].instruction = Instruction::LiveTokenPlace;
            } else if wire["type"] == "place_tokens_in_line" || wire["type"] == "place_token_trail"
            {
                steps[index].instruction = Instruction::LiveTokenPath {
                    trail: wire["type"] == "place_token_trail",
                };
            } else if wire["type"] == "place_turret" || wire["type"] == "remove_turret" {
                steps[index].instruction = Instruction::LiveTurret {
                    remove: wire["type"] == "remove_turret",
                };
            } else if wire["type"] == "offer_rock_ultimate" {
                steps[index].instruction = Instruction::LiveRockUltimate;
            } else if wire["type"] == "place_token_batch" {
                steps[index].instruction = Instruction::LiveTokenBatch;
            } else if wire["type"] == "remove_token" {
                steps[index].instruction = Instruction::LiveTokenRemove;
            } else if wire["type"] == "retrieve_card" {
                steps[index].instruction = Instruction::LiveRetrieve;
            } else if wire["type"] == "discard_card" {
                steps[index].instruction = Instruction::LiveDiscard;
            } else if wire["type"] == "force_discard_or_defeat" {
                steps[index].instruction = Instruction::LiveDiscardOrDefeat;
            } else if wire["type"] == "swap_with_deck_card" {
                steps[index].instruction = Instruction::LiveDeckCardSwap;
            } else if wire["type"] == "swap_item_card" {
                steps[index].instruction = Instruction::LiveItemSwap;
            } else if wire["type"] == "swap_resolved_cards" {
                steps[index].instruction = Instruction::LiveResolvedCardSwap;
            } else if wire["type"] == "swap_card" {
                steps[index].instruction = Instruction::LiveCardSwap;
            } else if wire["type"] == "force_discard_by_color" {
                steps[index].instruction = Instruction::LiveColorDiscard;
            } else if wire["type"] == "force_discard" {
                steps[index].instruction = Instruction::LiveForceDiscard;
            } else if wire["type"] == "mine_path_choice" {
                steps[index].instruction = Instruction::LiveMineChoice;
            } else if wire["type"] == "trigger_mine" {
                steps[index].instruction = Instruction::LiveTriggerMines;
            } else if wire["type"] == "force_defense_card_movement" {
                steps[index].instruction = Instruction::LiveForceDefenseMovement;
            } else if wire["type"] == "move_sequence" {
                steps[index].instruction = Instruction::LiveMoveSequence;
            } else if wire["type"] == "schedule_journey_return" {
                steps[index].instruction = Instruction::LiveJourney;
            } else if wire["type"] == "schedule_action_control" {
                steps[index].instruction = Instruction::LiveActionControl;
            } else if wire["type"] == "set_card_initiative" {
                steps[index].instruction = Instruction::LiveSetInitiative;
            } else if wire["type"] == "move_toward_target" {
                steps[index].instruction = Instruction::LiveMoveToward;
            } else if wire["type"] == "co_directional_drag" {
                steps[index].instruction = Instruction::LiveDrag;
            } else if wire["type"] == "move_token" {
                steps[index].instruction = Instruction::LiveTokenMove;
            } else if wire["type"] == "move_unit" {
                steps[index].instruction = Instruction::LiveMove;
            } else if matches!(
                wire["type"].as_str(),
                Some(
                    "record_hex"
                        | "compute_hex"
                        | "check_adjacency"
                        | "compute_distance"
                        | "resolve_turn_start_hex"
                )
            ) {
                steps[index].instruction =
                    Instruction::LiveCoordinates(match wire["type"].as_str().unwrap() {
                        "record_hex" => CoordinateOp::Record,
                        "compute_distance" => CoordinateOp::Distance,
                        "resolve_turn_start_hex" => CoordinateOp::TurnStart,
                        "compute_hex" => CoordinateOp::Compute,
                        _ => CoordinateOp::Adjacency,
                    });
            } else if wire["type"] == "fast_travel_sequence" || wire["type"] == "fast_travel_unit" {
                steps[index].instruction = Instruction::LiveTravel {
                    sequence: wire["type"] == "fast_travel_sequence",
                };
            } else if wire["type"] == "perform_primary_action" {
                steps[index].instruction = Instruction::LivePerformPrimary;
            } else if wire["type"] == "perform_card_action" {
                steps[index].instruction = Instruction::LivePerformCard;
            } else if wire["type"] == "resolve_card" {
                steps[index].instruction = Instruction::LiveCardAction;
            } else if wire["type"] == "razzle_mirrored_push" {
                for key in [
                    "razzle_mirror_dest",
                    "Select an adjacent unit to push",
                    "Choose push distance",
                ] {
                    symbols.intern(key);
                }
                steps[index].instruction = Instruction::LiveMirrorPush;
            } else if wire["type"] == "remove_hero_piece" {
                steps[index].instruction = Instruction::LiveRemovePiece;
            } else if wire["type"] == "spawn_hero_piece" {
                steps[index].instruction = Instruction::LiveSpawnPiece;
            } else if wire["type"] == "set_acting_piece" {
                steps[index].instruction = Instruction::LiveSetPiece;
            } else if wire["type"] == "choose_acting_piece" {
                steps[index].instruction = Instruction::LiveChoosePiece;
            } else if wire["type"] == "push_unit" {
                steps[index].instruction = Instruction::LivePush;
            } else if wire["type"] == "may_repeat_once" {
                let templates = wire["steps_template"]
                    .as_array()
                    .ok_or_else(|| unsupported("repeat templates missing"))?
                    .iter()
                    .map(|child| BoundObjects::compile_wire(child, steps, symbols))
                    .collect::<Result<Vec<_>>>()?;
                steps[index].instruction = Instruction::LiveRepeat { templates };
            } else if wire["type"] == "resolve_card_text" {
                steps[index].instruction = Instruction::LivePrimaryText;
            } else if wire["type"] == "resolve_defense_text"
                || wire["type"] == "resolve_on_block_effect"
            {
                steps[index].instruction = Instruction::LiveDefenseBody {
                    on_block: wire["type"] == "resolve_on_block_effect",
                };
            } else if matches!(
                wire["type"].as_str(),
                Some(
                    "gain_item"
                        | "gain_coins"
                        | "lose_coins"
                        | "steal_coins"
                        | "restore_spent_life_counter"
                        | "spend_additional_life_counter"
                )
            ) {
                steps[index].instruction = Instruction::LiveResource;
            } else if let Some(op) = match wire["type"].as_str() {
                Some("cast_spell") => Some(crate::spells::SpellOp::Cast),
                Some("wish_sequence") => Some(crate::spells::SpellOp::Wish),
                Some("prepare_spellbook") => Some(crate::spells::SpellOp::Prepare),
                Some("remove_prepared_spells") => Some(crate::spells::SpellOp::Remove),
                _ => None,
            } {
                steps[index].instruction = Instruction::LiveSpell(op);
            } else if wire["type"] == "convert_card_to_item" {
                steps[index].instruction = Instruction::LiveConvertCardItem;
            } else if wire["type"] == "apply_after_attack_card_text" {
                steps[index].instruction = Instruction::LiveAfterAttackText;
            } else if wire["type"] == "check_unit_on_board" {
                steps[index].instruction = Instruction::LiveUnitOnBoard;
            } else if wire["type"] == "check_unit_type" {
                steps[index].instruction = Instruction::LiveUnitTypeCheck;
            } else if wire["type"] == "combine_boolean_context" {
                steps[index].instruction = Instruction::LiveBooleanCombine;
            } else if wire["type"] == "check_distance" {
                steps[index].instruction = Instruction::LiveDistanceCheck;
            } else if wire["type"] == "check_unit_filters" {
                steps[index].instruction = Instruction::LiveCheckUnitFilters;
            } else if wire["type"] == "respawn_minion_at_hex" {
                steps[index].instruction = Instruction::LiveMinionRespawn;
            } else if wire["type"] == "check_solo_win" {
                steps[index].instruction = Instruction::LiveSoloWin;
            } else if wire["type"] == "check_zone_change" {
                steps[index].instruction = Instruction::LiveZoneChange;
            } else if wire["type"] == "count_adjacent_enemies" {
                steps[index].instruction = Instruction::LiveAdjacentEnemies;
            } else if wire["type"] == "count_cards" {
                steps[index].instruction = Instruction::LiveCardCount;
            } else if wire["type"] == "check_hero_defeated_this_round" {
                steps[index].instruction = Instruction::LiveRoundDefeat;
            } else if wire["type"] == "create_effect" {
                steps[index].instruction = Instruction::LiveCreateEffect;
            } else if wire["type"] == "place_marker" || wire["type"] == "remove_marker" {
                steps[index].instruction = Instruction::LiveMarker {
                    remove: wire["type"] == "remove_marker",
                };
            } else if wire["type"] == "swap_units" {
                steps[index].instruction = Instruction::LiveSwap;
            } else if wire["type"] == "place_unit" {
                steps[index].instruction = Instruction::LivePlace;
            } else if BoundObjects::parked_type(wire) {
                steps[index].instruction =
                    Instruction::ParkedPhase(symbols.intern(wire["type"].as_str().unwrap()));
            }
            if let Instruction::ForEach { templates, .. } = &steps[index].instruction {
                let children = templates.clone();
                for (index, child) in children.into_iter().zip(
                    wire["steps_template"]
                        .as_array()
                        .ok_or_else(|| unsupported("loop templates missing"))?,
                ) {
                    restore(index, child, steps, symbols)?;
                }
            }
            Ok(())
        }
        let mut sanitized = wire.clone();
        sanitize(&mut sanitized);
        let index = compile_step(&sanitized, steps, symbols)?;
        restore(index, wire, steps, symbols)?;
        Ok(index)
    }
    pub fn compile_scheduler(
        &mut self,
        steps: &mut Vec<CompiledStep>,
        symbols: &mut Symbols,
        primary: &SharedPrimaryPrograms,
    ) -> Result<()> {
        let mut recipe = |name: &str| -> Result<StepRecipe> {
            let mut wire = combat_template(match name {
                "return_movement" => "move_unit",
                "return_placement" => "place_unit",
                "attack_select" => "select_unit",
                "pre_action_select" => "select_hex",
                other => other,
            })?;
            if matches!(name, "return_movement" | "return_placement") {
                wire["is_mandatory"] = json!(false);
                if name == "return_movement" {
                    wire["is_movement_action"] = json!(false);
                }
            }
            if name == "attack_select" {
                let mut range = combat_template("range")?;
                range["max_range"] = json!(1);
                wire["prompt"] = json!("Select Attack Target");
                wire["filters"] = json!([range, {"type":"team_filter", "relation":"ENEMY"}]);
            }
            if name == "pre_action_select" {
                let mut obstacle = combat_template("obstacle")?;
                obstacle["is_obstacle"] = json!(false);
                obstacle["exclude_id"] = json!("template");
                let mut path = combat_template("movement_path")?;
                path["unit_id"] = json!("template");
                wire["is_mandatory"] = json!(false);
                wire["filters"] = json!([obstacle, path]);
            }
            let instruction = Self::compile_wire(&wire, steps, symbols)?;
            let fields = Datum::from_json(&wire, symbols).map_err(unsupported)?;
            Ok(StepRecipe {
                instruction,
                fields,
            })
        };
        self.scheduler = Some(Arc::new(FinishingProgram {
            actor: recipe("set_actor")?,
            marker: recipe("finished_expiring_effect")?,
            phase_recipes: [
                "advance_turn",
                "end_phase",
                "find_actor",
                "minion_battle",
                "lane_push",
                "minion_removal",
                "displacement",
                "place_unit",
                "check_lane_push",
                "end_cleanup",
                "upgrades",
                "round_reset",
                "retrieve_unresolved",
                "tie",
                "resolve_card",
                "confirm_resolution",
                "finalize_hero",
                "respawn",
                "flip_coin",
                "return_minions",
                "return_movement",
                "return_placement",
                "mine_path_choice",
                "trigger_mine",
                "force_discard",
                "select_card",
                "discard_card",
                "passive",
                "offer_passive",
                "mark_passive",
                "defeat",
                "remove_unit",
                "minion_protection",
                "game_over",
                "attack_select",
                "pre_action_select",
                "move_unit",
                "mirror_push",
                "move_token",
                "set_actor",
                "reaction",
                "pre_movement",
                "pre_discard",
                "defense_text",
                "resolve_combat",
                "on_block",
                "restore_action",
                "cast_spell",
                "perform_card",
                "wish_sequence",
                "prepare_spellbook",
                "remove_prepared_spells",
            ]
            .into_iter()
            .map(|name| Ok((name, recipe(name)?)))
            .collect::<Result<Vec<_>>>()?,
            passive_bodies: passive_bodies::BodyRules::compile(steps, symbols)?,
            defense_bodies: Arc::new(defense_bodies::DefenseRules::compile(steps, symbols)?),
            primary_bodies: primary.rules.clone(),
            action_bodies: Arc::new(card_actions::ActionBindings::compile(steps, symbols)?),
            journey_finishing: journey::compile_finishing(steps, symbols)?,
            step_id: symbols.intern("step_id"),
            actor_id: symbols.intern("actor_id"),
        }));
        Ok(())
    }
    fn finishing_for_indices(
        &self,
        effects: &crate::effects::EffectStore,
        indices: &[usize],
    ) -> Result<Vec<(Symbol, ObjectId)>> {
        let nodes = self.effect_nodes()?;
        if nodes.len() != effects.instances.len() {
            return Err(unsupported("effect collection binding drift"));
        }
        let mut finishing = Vec::new();
        for &index in indices {
            let effect = &effects.instances[index];
            if !effect.definition.after_cards_played {
                let list = self.finishing_list(nodes[index])?;
                if self
                    .arena
                    .sequence(list)
                    .is_some_and(|steps| !steps.is_empty())
                {
                    finishing.push((effect.source, list));
                }
            }
        }
        Ok(finishing)
    }
    fn remove_effect_rows(&mut self, removed: &[usize]) -> Result<()> {
        let retained = self
            .effect_nodes()?
            .into_iter()
            .enumerate()
            .filter_map(|(index, object)| {
                (!removed.contains(&index)).then_some(ObjectValue::Object(object))
            })
            .collect();
        let list = self.arena.list(retained);
        self.arena
            .set_root(self.effects, list)
            .map_err(unsupported)?;
        self.readonly.take();
        Ok(())
    }
    fn generated_step(
        &mut self,
        recipe: &StepRecipe,
        program: &FinishingProgram,
        generation: u64,
        actor: Option<Symbol>,
    ) -> Result<ObjectId> {
        let object = self.arena.step(&recipe.fields).map_err(unsupported)?;
        self.arena
            .set_field(
                object,
                program.step_id,
                ObjectValue::GeneratedStep(generation),
            )
            .map_err(unsupported)?;
        if let Some(actor) = actor {
            self.arena
                .set_field(
                    object,
                    program.actor_id,
                    ObjectValue::Atom(Datum::Text(actor)),
                )
                .map_err(unsupported)?;
        }
        self.bind_copied_program(object, recipe.instruction);
        Ok(object)
    }
    pub fn bind_stored_programs(
        &mut self,
        source: &StateEnvelope,
        steps: &mut Vec<CompiledStep>,
        symbols: &mut Symbols,
    ) -> Result<()> {
        // Removed effect objects can remain visible through context aliases.
        // Rebind every reachable stored step, not just current effect rows.
        for binding in source
            .object_bindings
            .iter()
            .filter(|binding| binding.kind == ObjectKind::Step)
        {
            let path = binding
                .paths
                .first()
                .ok_or_else(|| unsupported("stored step path missing"))?;
            let wire = crate::codec::object_at_path(&source.state, path).map_err(unsupported)?;
            let ObjectValue::Object(object) =
                self.arena.value_at_path(path).map_err(unsupported)?
            else {
                return Err(unsupported("stored step handle missing"));
            };
            if !Self::supported(wire) || self.program(object).is_some() {
                continue;
            }
            let before = steps.len();
            match Self::compile_wire(wire, steps, symbols) {
                Ok(instruction) => self.bind_program(object, instruction, steps)?,
                Err(_) => steps.truncate(before), // Unused unsupported payloads remain data.
            }
        }
        Ok(())
    }
    fn effect_nodes(&self) -> Result<Vec<ObjectId>> {
        let Some(ObjectValue::Object(list)) = self.arena.root_field(self.effects) else {
            return Err(unsupported("effect collection missing"));
        };
        self.arena
            .sequence(list)
            .ok_or_else(|| unsupported("effect collection must be a list"))?
            .iter()
            .map(|value| {
                if let ObjectValue::Object(id) = value {
                    Ok(*id)
                } else {
                    Err(unsupported("effect model handle missing"))
                }
            })
            .collect()
    }
    fn finishing_list(&self, effect: ObjectId) -> Result<ObjectId> {
        let Some(ObjectValue::Object(list)) = self.arena.field(effect, self.finishing) else {
            return Err(unsupported("finishing list missing"));
        };
        if self.arena.sequence(list).is_none() {
            return Err(unsupported("finishing payload must be a list"));
        }
        Ok(list)
    }
    fn steps_in_list(&self, list: ObjectId) -> Result<Vec<ObjectId>> {
        self.arena
            .sequence(list)
            .ok_or_else(|| unsupported("step sequence missing"))?
            .iter()
            .map(|value| {
                if let ObjectValue::Object(id) = value
                    && self.program(*id).is_some()
                {
                    Ok(*id)
                } else {
                    Err(unsupported("stored step program is not compiled"))
                }
            })
            .collect()
    }
    fn remove_effect_nodes(&mut self, ids: &[Symbol]) -> Result<()> {
        let nodes = self.effect_nodes()?;
        let mut retained = Vec::new();
        for object in nodes {
            let Some(ObjectValue::Atom(Datum::Text(id))) = self.arena.field(object, self.effect_id)
            else {
                return Err(unsupported("effect identity missing"));
            };
            if !ids.contains(&id) {
                retained.push(ObjectValue::Object(object));
            }
        }
        let list = self.arena.list(retained);
        self.arena
            .set_root(self.effects, list)
            .map_err(unsupported)?;
        // Source replaces the list. Old aliases become detached mutable data.
        self.readonly.take();
        Ok(())
    }
    pub fn bind_program(
        &mut self,
        object: ObjectId,
        instruction: usize,
        steps: &[CompiledStep],
    ) -> Result<()> {
        if self.programs.contains_key(&object) {
            return Ok(());
        }
        Arc::make_mut(&mut self.programs).insert(object, instruction);
        if let Instruction::ForEach { templates, .. } | Instruction::LiveRepeat { templates } =
            &steps[instruction].instruction
        {
            let objects = self.template_objects(object)?;
            if objects.len() != templates.len() {
                return Err(unsupported("loop template binding drift"));
            }
            for (object, instruction) in objects.into_iter().zip(templates) {
                self.bind_program(object, *instruction, steps)?;
            }
        }
        Ok(())
    }
    fn template_objects(&self, step: ObjectId) -> Result<Vec<ObjectId>> {
        let Some(ObjectValue::Object(id)) = self.arena.field(step, self.templates) else {
            return Err(unsupported("loop template list missing"));
        };
        self.arena
            .sequence(id)
            .ok_or_else(|| unsupported("loop template is not a sequence"))?
            .iter()
            .map(|value| match value {
                ObjectValue::Object(id) if self.arena.kind(*id) == ObjectKind::Step => Ok(*id),
                _ => Err(unsupported("loop template step binding missing")),
            })
            .collect()
    }
    fn program(&self, step: ObjectId) -> Option<usize> {
        self.programs
            .get(&step)
            .or_else(|| self.copied_programs.get(&step))
            .copied()
    }
    fn bind_copied_program(&mut self, object: ObjectId, instruction: usize) {
        let previous = self.copied_programs.insert(object, instruction);
        if self.arena.is_recording() && previous != Some(instruction) {
            self.copied_binding_undo
                .0
                .push(CopiedBinding::Program(object, previous));
        }
    }
    fn restore_copied_bindings(&mut self, checkpoint: usize) {
        for binding in self.copied_binding_undo.0.drain(checkpoint..).rev() {
            match binding {
                CopiedBinding::Program(object, Some(instruction)) => {
                    self.copied_programs.insert(object, instruction);
                }
                CopiedBinding::Program(object, None) => {
                    self.copied_programs.remove(&object);
                }
                CopiedBinding::Protection(object) => {
                    self.copied_readonly.remove(&object);
                }
            }
        }
    }
    fn protected(&self, object: ObjectId) -> bool {
        if self.copied_readonly.contains(&object) {
            return true;
        }
        if let Some(readonly) = self.readonly.get() {
            return readonly.contains(&object);
        }
        if self.arena.fresh_context_object_is_private(object) {
            return false;
        }
        self.readonly
            .get_or_init(|| Arc::new(self.arena.readonly_objects()))
            .contains(&object)
    }
    fn copy_template(&mut self, step: ObjectId) -> Result<(ObjectId, usize)> {
        let (ObjectValue::Object(copy), objects) = self
            .arena
            .deep_copy(&ObjectValue::Object(step))
            .map_err(unsupported)?
        else {
            return Err(unsupported("copied template step missing"));
        };
        for (original, copied) in objects {
            if let Some(instruction) = self.program(original) {
                self.bind_copied_program(copied, instruction);
            }
        }
        // Protection belongs to the copied operand role, not its provenance.
        // A detached Set.value copy of a world list is ordinary mutable data.
        for object in self.arena.readonly_step_objects(copy) {
            if self.copied_readonly.insert(object) && self.arena.is_recording() {
                self.copied_binding_undo
                    .0
                    .push(CopiedBinding::Protection(object));
            }
        }
        let instruction = self
            .program(copy)
            .ok_or_else(|| unsupported("copied step program missing"))?;
        Ok((copy, instruction))
    }
    pub fn step(&self, index: usize) -> Result<ObjectId> {
        let ObjectValue::Object(id) = self
            .arena
            .value_at_path(&[
                ObjectPathPart::Key("execution_stack".into()),
                ObjectPathPart::Index(index),
            ])
            .map_err(unsupported)?
        else {
            return Err(unsupported("step object missing"));
        };
        if self.arena.kind(id) != ObjectKind::Step {
            return Err(unsupported("step kind binding missing"));
        }
        Ok(id)
    }
    pub(super) fn refresh_pending_request(&mut self, step: ObjectId, id: u64) -> Result<()> {
        self.arena
            .set_field(step, self.request, ObjectValue::Request(id))
            .map_err(unsupported)
    }

    pub fn symbol_from_context(&self, key: Symbol, symbols: &Symbols) -> Result<Option<Symbol>> {
        match self.arena.context_value(key) {
            Some(ObjectValue::Atom(Datum::Text(id))) if !symbols.name(id).is_empty() => {
                Ok(Some(id))
            }
            Some(value) if self.arena.truthy(&value) => {
                Err(unsupported("non-string actor/controller context"))
            }
            _ => Ok(None),
        }
    }
    fn mutable_stack(&self, key: Symbol) -> Result<Option<ObjectId>> {
        match self.arena.context_value(key) {
            None => Ok(None),
            Some(ObjectValue::Object(id)) if self.arena.is_mutable_list(id) => {
                if self.protected(id) {
                    return Err(unsupported(
                        "context stack aliases an unbound world/step operand",
                    ));
                }
                Ok(Some(id))
            }
            Some(value) if !self.arena.truthy(&value) => Ok(None),
            _ => Err(unsupported("context stack must be a mutable list")),
        }
    }
    fn restore_scope(&mut self) -> Result<()> {
        let Some(stack) = self.mutable_stack(self.action_stack)? else {
            return self
                .arena
                .remove_context(self.action_stack)
                .map_err(unsupported);
        };
        let snapshot = match self.arena.list_pop(stack).map_err(unsupported)? {
            None => {
                return self
                    .arena
                    .remove_context(self.action_stack)
                    .map_err(unsupported);
            }
            Some(ObjectValue::Object(snapshot)) => snapshot,
            _ => return Err(unsupported("action snapshot must be a dictionary")),
        };
        if self.arena.kind(snapshot) != ObjectKind::Dict {
            return Err(unsupported("action snapshot must be a dictionary"));
        }
        for key in self.action_keys {
            match self.arena.field(snapshot, key) {
                None => self.arena.remove_context(key).map_err(unsupported)?,
                Some(ObjectValue::Object(saved)) if self.arena.kind(saved) == ObjectKind::Dict => {
                    let present = self
                        .arena
                        .field(saved, self.present)
                        .ok_or_else(|| unsupported("snapshot presence flag missing"))?;
                    if self.arena.truthy(&present) {
                        let value = self
                            .arena
                            .field(saved, self.value)
                            .ok_or_else(|| unsupported("snapshot value missing"))?;
                        self.arena.set_context(key, value).map_err(unsupported)?;
                    } else {
                        self.arena.remove_context(key).map_err(unsupported)?;
                    }
                }
                _ => return Err(unsupported("action snapshot field must be a dictionary")),
            }
        }
        if self
            .arena
            .sequence(stack)
            .is_some_and(|values| values.is_empty())
        {
            self.arena
                .remove_context(self.action_stack)
                .map_err(unsupported)?;
        }
        Ok(())
    }
    fn push_scope(&mut self, action: Symbol, card: Symbol, owner: Symbol) -> Result<()> {
        let stack = if self.arena.context_value(self.action_stack).is_none() {
            let ObjectValue::Object(stack) = self.arena.allocate(&Datum::List(Vec::new())) else {
                unreachable!()
            };
            self.arena
                .set_context(self.action_stack, ObjectValue::Object(stack))
                .map_err(unsupported)?;
            stack
        } else {
            self.mutable_stack(self.action_stack)?
                .ok_or_else(|| unsupported("action snapshot stack must be a list"))?
        };
        let mut fields = Vec::new();
        for key in self.action_keys {
            let value = self.arena.context_value(key);
            let saved = self
                .arena
                .dictionary(vec![
                    (
                        self.present,
                        ObjectValue::Atom(Datum::Bool(value.is_some())),
                    ),
                    (self.value, value.unwrap_or(ObjectValue::Atom(Datum::Null))),
                ])
                .map_err(unsupported)?;
            fields.push((key, saved));
        }
        let snapshot = self.arena.dictionary(fields).map_err(unsupported)?;
        self.arena.append(stack, snapshot).map_err(unsupported)?;
        for (key, value) in self.action_keys[..5]
            .iter()
            .zip([action, card, card, owner, card])
        {
            self.arena
                .set_context(*key, ObjectValue::Atom(Datum::Text(value)))
                .map_err(unsupported)?;
        }
        Ok(())
    }
    fn restore_type(&mut self) -> Result<()> {
        // The source only pops a truthy stack. Empty aliases need no mutation
        // guard and must remain attached to the original list.
        if self
            .arena
            .context_value(self.type_stack)
            .is_some_and(|value| self.arena.truthy(&value))
            && let Some(stack) = self.mutable_stack(self.type_stack)?
            && let Some(value) = self.arena.list_pop(stack).map_err(unsupported)?
        {
            self.arena
                .set_context(self.action_keys[0], value)
                .map_err(unsupported)?;
        }
        Ok(())
    }
    fn clear_pending(&mut self, id: ObjectId) -> Result<()> {
        for key in [self.pending, self.request] {
            self.arena
                .set_field(id, key, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
        }
        Ok(())
    }
    fn token(&self, id: ObjectId) -> Result<Option<RequestToken>> {
        match self.arena.field(id, self.request) {
            None | Some(ObjectValue::Atom(Datum::Null)) => Ok(None),
            Some(ObjectValue::Atom(Datum::Text(id))) => Ok(Some(RequestToken::Imported(id))),
            Some(ObjectValue::Request(id)) => Ok(Some(RequestToken::Generated(id))),
            _ => Err(unsupported("invalid live step request token")),
        }
    }
    fn selected(&self, id: ObjectId, selection: Symbol) -> Option<ObjectValue> {
        let ObjectValue::Object(pending) = self.arena.field(id, self.pending)? else {
            return None;
        };
        self.arena.field(pending, selection)
    }
    fn write(&mut self, key: Symbol, value: Datum) -> Result<()> {
        self.arena
            .set_context(key, ObjectValue::Atom(value))
            .map_err(unsupported)
    }
}

impl NativeSession {
    pub(super) fn with_live_objects<T>(
        &mut self,
        action: impl FnOnce(&mut Self, &mut BoundObjects) -> Result<T>,
    ) -> Result<T> {
        let mut objects = self
            .objects
            .take()
            .ok_or_else(|| unsupported("operation requires live object bindings"))?;
        if objects.arena.is_recording() {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                action(self, &mut objects)
            }));
            self.objects = Some(objects);
            return match result {
                Ok(value) => value,
                Err(panic) => std::panic::resume_unwind(panic),
            };
        }
        let result = action(self, &mut objects);
        self.objects = Some(objects);
        result
    }
    pub(super) fn advance_objects(
        &mut self,
        budget: &ExecutionBudget<'_>,
        stop_after: Option<usize>,
        include_request: bool,
    ) -> Result<AdvanceResult> {
        let mut result = self.with_live_objects(|session, objects| {
            session.run_objects(objects, budget, stop_after, None, None)
        })?;
        if include_request {
            result.request = self.last_request.clone();
        }
        Ok(result)
    }
    pub(super) fn pending_attack_target(&self) -> bool {
        use phases::Field;
        let Some(objects) = &self.objects else {
            return false;
        };
        if !self
            .last_request
            .as_ref()
            .is_some_and(|request| matches!(request.choices, RequestChoices::Units { .. }))
        {
            return false;
        }
        let mut frames = self.stack.iter().rev();
        let Some(selector) = frames.next() else {
            return false;
        };
        if !matches!(
            self.steps[selector.instruction].instruction,
            Instruction::LiveUnitSelect
        ) {
            return false;
        }
        let Some(selector) = selector.object else {
            return false;
        };
        let Ok(output) = self.passive_text_bound(objects, selector, Field::Output) else {
            return false;
        };
        frames
            .find_map(|frame| {
                let key = match self.steps[frame.instruction].instruction {
                    Instruction::LiveCombatResolve => Field::TargetKey,
                    Instruction::LiveAttackSequence => Field::AttackTargetKey,
                    _ => return None,
                };
                Some(frame.object.is_some_and(|step| {
                    self.passive_text_bound(objects, step, key).ok() == Some(output)
                }))
            })
            .unwrap_or(false)
    }

    pub(super) fn pending_token_swap(&self) -> bool {
        use phases::Field;

        let Some(objects) = &self.objects else {
            return false;
        };
        let Some(request) = &self.last_request else {
            return false;
        };
        let RequestChoices::Units {
            tokens: true,
            values,
        } = &request.choices
        else {
            return false;
        };
        if !values.iter().any(|(id, _)| {
            self.world
                .entity(*id)
                .is_some_and(|entity| entity.kind == crate::world::EntityKind::Token)
        }) {
            return false;
        }
        let mut frames = self.stack.iter().rev();
        let Some(selector) = frames.next() else {
            return false;
        };
        let Some(swap) = frames.next() else {
            return false;
        };
        if !matches!(
            self.steps[selector.instruction].instruction,
            Instruction::LiveUnitSelect
        ) || !matches!(
            self.steps[swap.instruction].instruction,
            Instruction::LiveSwap
        ) {
            return false;
        }
        let (Some(selector), Some(swap)) = (selector.object, swap.object) else {
            return false;
        };
        let Ok(output) = self.passive_text_bound(objects, selector, Field::Output) else {
            return false;
        };
        [Field::UnitAKey, Field::UnitBKey].into_iter().any(|key| {
            self.passive_optional_text_bound(objects, swap, key)
                .ok()
                .flatten()
                == Some(output)
        })
    }

    pub(super) fn advance_search_objects(
        &mut self,
        budget: &ExecutionBudget<'_>,
        viewer: Symbol,
    ) -> Result<AdvanceResult> {
        self.with_live_objects(|session, objects| {
            session.run_objects(objects, budget, None, Some(viewer), None)
        })
    }

    pub(super) fn advance_search_objects_before_hand(
        &mut self,
        budget: &ExecutionBudget<'_>,
        viewer: Symbol,
        known: &[Symbol],
    ) -> Result<AdvanceResult> {
        self.with_live_objects(|session, objects| {
            session.run_objects(objects, budget, None, Some(viewer), Some(known))
        })
    }

    fn run_objects(
        &mut self,
        objects: &mut BoundObjects,
        budget: &ExecutionBudget<'_>,
        stop_after: Option<usize>,
        search_viewer: Option<Symbol>,
        hand_scope: Option<&[Symbol]>,
    ) -> Result<AdvanceResult> {
        let mut result = AdvanceResult::default();
        if self.planning.phase == Phase::GameOver {
            self.last_request = None;
            return Ok(result);
        }
        if !self.stack.is_empty() {
            budget.check(0)?;
        }
        self.last_request = None;
        let profile_steps = RULE_STEP_PROFILE.with_borrow(Option::is_some);
        while !self.stack.is_empty() {
            if stop_after.is_some_and(|limit| result.steps >= limit) {
                return Ok(result);
            }
            budget.check(result.steps)?;
            self.count_observer.clear();
            let top_instruction = self.stack.last().unwrap().instruction;
            // Retain only the immutable program, even if execution rewrites the stack.
            let mut rule_timer = profile_steps.then(|| RuleStepTimer {
                program: self.steps[top_instruction].clone(),
                start: Instant::now(),
                category: None,
                action_rules_ns: None,
                recipe_prep_ns: None,
                action_apply_ns: None,
            });
            let top_object = self.stack.last().unwrap().object;
            let conditions = &self.steps[top_instruction].conditions;
            let move_success = if matches!(
                self.steps[top_instruction].instruction,
                Instruction::LiveMove
            ) {
                self.movement_output_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live movement step missing"))?,
                    phases::Field::Success,
                )?
            } else {
                None
            };
            let active = if let Some(id) = top_object {
                self.passive_optional_text_bound(objects, id, phases::Field::ActiveIf)?
            } else {
                conditions.active
            };
            let skip = if let Some(id) = top_object {
                self.passive_optional_text_bound(objects, id, phases::Field::SkipIf)?
            } else {
                conditions.skip
            };
            let skipped = active
                .filter(|key| !self.symbols.name(*key).is_empty())
                .is_some_and(|key| {
                    !objects
                        .arena
                        .context_value(key)
                        .is_some_and(|value| objects.arena.truthy(&value))
                })
                || skip
                    .filter(|key| !self.symbols.name(*key).is_empty())
                    .is_some_and(|key| {
                        objects
                            .arena
                            .context_value(key)
                            .is_some_and(|value| !matches!(value, ObjectValue::Atom(Datum::Null)))
                    });
            if !skipped
                && let Some((viewer, known)) = search_viewer.zip(hand_scope)
                && self
                    .hidden_hand_reader(objects, top_instruction, top_object, viewer, known)?
                    .is_some()
            {
                return Ok(result);
            }
            if let Some(viewer) = search_viewer
                && !skipped
                && self.search_step_reads_concealed_state(
                    objects,
                    top_instruction,
                    top_object,
                    viewer,
                )?
            {
                return Ok(result);
            }
            let top_mandatory = match top_object.and_then(|id| {
                objects
                    .arena
                    .field(id, objects.phases.k(phases::Field::Mandatory))
            }) {
                None => conditions.mandatory,
                Some(ObjectValue::Atom(Datum::Bool(value))) => value,
                _ => return Err(unsupported("live mandatory flag must be boolean")),
            };
            let movement_inner_skip =
                move_success.is_some_and(|key| conditions.active == Some(key));
            let actor_choice = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::Flow(crate::turn::FlowStep::Next)
                ) {
                Some(self.actor_choice_bound(objects)?)
            } else {
                None
            };
            let return_plan = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveReturnMinions
                ) {
                Some(self.preflight_return_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live minion-return step missing"))?,
                )?)
            } else {
                None
            };
            let placement = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LivePlace
                ) {
                Some(self.preflight_place_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live placement step missing"))?,
                )?)
            } else {
                None
            };
            let travel_plan = if !skipped {
                if let Instruction::LiveTravel { sequence } =
                    self.steps[top_instruction].instruction
                {
                    Some(self.preflight_travel_bound(
                        objects,
                        top_object.ok_or_else(|| unsupported("live travel step missing"))?,
                        sequence,
                    )?)
                } else {
                    None
                }
            } else {
                None
            };
            let perform_primary = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LivePerformPrimary
                ) {
                Some(
                    self.preflight_perform_primary_bound(
                        objects,
                        top_object
                            .ok_or_else(|| unsupported("live primary performance step missing"))?,
                    )?,
                )
            } else {
                None
            };
            let perform_card = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LivePerformCard
                ) {
                Some(self.preflight_perform_card_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live copied-card step missing"))?,
                )?)
            } else {
                None
            };
            let card_action = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveCardAction
                ) {
                Some(self.preflight_card_action_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live card-action step missing"))?,
                    rule_timer.as_mut(),
                )?)
            } else {
                None
            };
            let acting_piece = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveChoosePiece
                ) {
                Some(self.preflight_acting_piece_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live acting-piece step missing"))?,
                )?)
            } else {
                None
            };
            let push = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LivePush
                ) {
                Some(self.preflight_push_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live push step missing"))?,
                )?)
            } else {
                None
            };
            let repeat = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveRepeat { .. }
                ) {
                Some(self.preflight_repeat_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live repeat step missing"))?,
                )?)
            } else {
                None
            };
            let journey = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveJourney
                ) {
                Some(self.preflight_journey_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("journey step missing"))?,
                )?)
            } else {
                None
            };
            let action_control = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveActionControl
                ) {
                Some(self.preflight_action_control_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("action control step missing"))?,
                )?)
            } else {
                None
            };
            let initiative = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveSetInitiative
                ) {
                Some(self.preflight_initiative_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("initiative step missing"))?,
                )?)
            } else {
                None
            };
            let co_movement = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveDrag | Instruction::LiveTokenMove
                ) {
                Some(self.preflight_co_movement_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("co-movement step missing"))?,
                    matches!(
                        self.steps[top_instruction].instruction,
                        Instruction::LiveDrag
                    ),
                )?)
            } else {
                None
            };
            let move_toward = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveMoveToward
                ) {
                self.preflight_move_toward_bound(objects, top_object.unwrap())?
            } else {
                None
            };
            let coordinates = if !skipped
                && let Instruction::LiveCoordinates(op) = self.steps[top_instruction].instruction
            {
                Some(self.preflight_coordinates_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live coordinate step missing"))?,
                    op,
                )?)
            } else {
                None
            };
            let primary_text = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LivePrimaryText
                ) {
                Some(self.preflight_primary_text_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live primary text step missing"))?,
                )?)
            } else {
                None
            };
            let defense_body = if !skipped
                && let Instruction::LiveDefenseBody { on_block } =
                    self.steps[top_instruction].instruction
            {
                Some(self.preflight_defense_bound(objects, on_block)?)
            } else {
                None
            };
            let resource = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveResource
                ) {
                Some(self.preflight_resource_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live resource step missing"))?,
                )?)
            } else {
                None
            };
            let effect_creation = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveCreateEffect
                ) {
                Some(self.preflight_create_effect_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live create effect step missing"))?,
                )?)
            } else {
                None
            };
            let marker = if !skipped
                && let Instruction::LiveMarker { remove } = self.steps[top_instruction].instruction
            {
                Some(self.preflight_marker_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live marker step missing"))?,
                    remove,
                )?)
            } else {
                None
            };
            let swap = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveSwap
                ) {
                Some(self.preflight_swap_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live swap step missing"))?,
                )?)
            } else {
                None
            };
            let movement = if !skipped
                && !movement_inner_skip
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveMove
                ) {
                Some(self.preflight_move_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let mine_choice = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveMineChoice
                ) {
                Some(self.preflight_mine_choice_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let mine_trigger = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveTriggerMines
                ) {
                Some(self.preflight_mine_trigger_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let sequence = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveMoveSequence
                ) {
                Some(self.preflight_move_sequence_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let forced_movement = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveForceDefenseMovement
                ) {
                self.preflight_force_defense_movement_bound(objects, top_object.unwrap())?
            } else {
                None
            };
            let hex_selection = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveHexSelect
                ) {
                Some(self.preflight_hex_selection_bound(
                    objects,
                    top_object.unwrap(),
                    top_mandatory,
                )?)
            } else {
                None
            };
            let token_place = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveTokenPlace
                ) {
                Some(self.preflight_token_place_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let token_path = if !skipped
                && let Instruction::LiveTokenPath { trail } =
                    self.steps[top_instruction].instruction
            {
                Some(self.preflight_token_path_bound(objects, top_object.unwrap(), trail)?)
            } else {
                None
            };
            let token_batch = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveTokenBatch
                ) {
                Some(self.preflight_token_batch_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let turret = if !skipped
                && let Instruction::LiveTurret { remove } = self.steps[top_instruction].instruction
            {
                Some(self.preflight_turret_bound(objects, top_object.unwrap(), remove)?)
            } else {
                None
            };
            let rock_ultimate = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveRockUltimate
                ) {
                self.preflight_rock_ultimate_bound(objects, top_object.unwrap())?
            } else {
                None
            };
            let token_remove = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveTokenRemove
                ) {
                self.preflight_token_remove_bound(objects, top_object.unwrap())?
            } else {
                None
            };
            let convert_item = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveConvertCardItem
                ) {
                self.preflight_convert_item_bound(objects, top_object.unwrap())?
            } else {
                None
            };
            let after_attack = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveAfterAttackText
                ) {
                self.preflight_after_attack_bound(objects, top_object.unwrap())?
            } else {
                None
            };
            let retrieve = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveRetrieve
                ) {
                self.preflight_retrieve_bound(objects, top_object.unwrap())?
            } else {
                None
            };
            let minion_respawn = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveMinionRespawn
                ) {
                Some(self.preflight_minion_respawn_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::Flow(crate::turn::FlowStep::Cleanup)
                )
            {
                result.events.extend(self.cleanup_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live cleanup step missing"))?,
                )?);
                result.steps += 1;
                continue;
            }
            if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::Flow(crate::turn::FlowStep::Respawn(_))
                )
            {
                let events = self.hero_respawn_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live respawn step missing"))?,
                )?;
                result.events.extend(events);
                result.steps += 1;
                if self.last_request.is_some() {
                    return Ok(result);
                }
                continue;
            }
            if !skipped
                && let Instruction::Flow(
                    ref flow @ (crate::turn::FlowStep::Upgrades | crate::turn::FlowStep::Reset),
                ) = self.steps[top_instruction].instruction
            {
                self.round_transition_bound(
                    objects,
                    top_object.ok_or_else(|| unsupported("live round transition missing"))?,
                    matches!(flow, crate::turn::FlowStep::Reset),
                )?;
                result.steps += 1;
                if self.last_request.is_some() {
                    return Ok(result);
                }
                continue;
            }
            let battle_plan = if !skipped {
                if let Instruction::Battle(family) = self.steps[top_instruction].instruction.clone()
                {
                    Some(self.preflight_battle_bound(objects, top_object.unwrap(), family)?)
                } else {
                    None
                }
            } else {
                None
            };
            let spell_plan = if !skipped {
                if let Instruction::LiveSpell(op) = self.steps[top_instruction].instruction {
                    Some(self.preflight_spell_bound(objects, top_object.unwrap(), op)?)
                } else {
                    None
                }
            } else {
                None
            };
            let query = if !skipped {
                match self.steps[top_instruction].instruction {
                    Instruction::LiveQuery { collect } => {
                        Some(self.preflight_query_bound(objects, top_object.unwrap(), collect)?)
                    }
                    Instruction::LiveCheckUnitFilters => {
                        Some(self.preflight_unit_filters_query_bound(objects, top_object.unwrap())?)
                    }
                    Instruction::LiveUnitOnBoard => {
                        Some(self.preflight_unit_on_board_bound(objects, top_object.unwrap())?)
                    }
                    Instruction::LiveUnitTypeCheck => {
                        Some(self.preflight_unit_type_check_bound(objects, top_object.unwrap())?)
                    }
                    Instruction::LiveBooleanCombine => {
                        Some(self.preflight_boolean_combine_bound(objects, top_object.unwrap())?)
                    }
                    Instruction::LiveDistanceCheck => {
                        Some(self.preflight_distance_check_bound(objects, top_object.unwrap())?)
                    }
                    Instruction::LiveZoneChange => {
                        Some(self.preflight_zone_change_bound(objects, top_object.unwrap())?)
                    }
                    Instruction::LiveAdjacentEnemies => {
                        Some(self.preflight_adjacent_enemies_bound(objects, top_object.unwrap())?)
                    }
                    Instruction::LiveCardCount => {
                        Some(self.preflight_card_count_bound(objects, top_object.unwrap())?)
                    }
                    Instruction::LiveRoundDefeat => {
                        Some(self.preflight_round_defeat_bound(objects, top_object.unwrap())?)
                    }
                    Instruction::LiveContextCondition => {
                        Some(self.preflight_context_condition_bound(objects, top_object.unwrap())?)
                    }
                    _ => None,
                }
            } else {
                None
            };
            let unit_selection = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveUnitSelect
                ) {
                Some(self.preflight_unit_selection_bound(
                    objects,
                    top_object.unwrap(),
                    top_mandatory,
                )?)
            } else {
                None
            };
            let attack = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveAttackSequence
                ) {
                Some(self.preflight_attack_sequence_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let actor = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::SetActor { .. }
                ) {
                Some(self.preflight_actor_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let pre_move = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LivePreMovement
                ) {
                self.preflight_pre_movement_bound(objects, top_object.unwrap())?
            } else {
                None
            };
            let pre_discard = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LivePreDiscard
                ) {
                Some(self.preflight_pre_discard_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let reaction = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveReaction
                ) {
                Some(self.preflight_reaction_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let combat_resolve = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveCombatResolve
                ) {
                Some(self.preflight_combat_resolution_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let adjacent = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveAdjacentSnapshot
                ) {
                Some(self.preflight_adjacent_snapshot_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let target_defeated = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveTargetDefeated
                ) {
                Some(self.preflight_target_defeated_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let defeat = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveDefeat
                ) {
                self.preflight_defeat_bound(objects, top_object.unwrap())?
            } else {
                None
            };
            let removal = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveRemoveUnit
                ) {
                self.preflight_remove_unit_bound(objects, top_object.unwrap())?
            } else {
                None
            };
            let solo_win = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveSoloWin
                ) {
                self.preflight_solo_win_bound(objects, top_object.unwrap())?
            } else {
                None
            };
            let game_over = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveGameOver
                ) {
                Some(self.preflight_game_over_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let protection = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveProtection
                ) {
                Some(self.preflight_protection_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let multi_selection = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveMultiSelect
                ) {
                Some(self.preflight_multi_selection_bound(
                    objects,
                    top_object.unwrap(),
                    top_mandatory,
                )?)
            } else {
                None
            };
            let rune = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveRuneChoice
                ) {
                Some(self.preflight_rune_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let passive = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LivePassive
                ) {
                Some(self.preflight_passive_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let offer = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveOfferPassive
                ) {
                Some(self.preflight_offer_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let mark = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveMarkPassive
                ) {
                Some(self.preflight_mark_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let discard = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveDiscard
                ) {
                Some(self.preflight_discard_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let discard_or_defeat = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveDiscardOrDefeat
                ) {
                self.preflight_discard_or_defeat_bound(objects, top_object.unwrap())?
            } else {
                None
            };
            let color_discard = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveColorDiscard
                ) {
                self.preflight_color_discard_bound(objects, top_object.unwrap())?
            } else {
                None
            };
            let forced_discard = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveForceDiscard
                ) {
                Some(self.preflight_forced_discard_bound(objects, top_object.unwrap())?)
            } else {
                None
            };
            let card_selection = if !skipped
                && matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::LiveCardSelect
                ) {
                Some(self.preflight_card_selection_bound(
                    objects,
                    top_object.unwrap(),
                    top_mandatory,
                )?)
            } else {
                None
            };
            if !skipped {
                if matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::Flow(crate::turn::FlowStep::Advance)
                ) {
                    objects.phases.effect_ids(
                        &objects.arena,
                        top_object.ok_or_else(|| unsupported("live advance step missing"))?,
                    )?;
                }
                if matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::Flow(crate::turn::FlowStep::Confirm(_))
                ) {
                    self.preflight_confirm_bound(
                        objects,
                        top_object.ok_or_else(|| unsupported("live confirmation step missing"))?,
                    )?;
                }
                if matches!(
                    self.steps[top_instruction].instruction,
                    Instruction::Flow(crate::turn::FlowStep::Finalize(_))
                ) {
                    self.preflight_finalize_bound(
                        objects,
                        top_object.ok_or_else(|| unsupported("live finalization step missing"))?,
                    )?;
                }
                self.check_bound_phase(
                    objects,
                    &self.steps[top_instruction].instruction,
                    actor_choice.as_ref(),
                )?;
            }
            // CountMatch restores a non-null previous origin, but Python's
            // finally block removes an explicit null just like an absent key.
            // Apply that cleanup only after a successful staged evaluation and
            // before the instruction writes its own output (possibly this key).
            if self.count_observer.observed()
                && let Some(key) = self.symbols.lookup("_cmf_origin_hex")
                && matches!(
                    objects.arena.context_value(key),
                    Some(ObjectValue::Atom(Datum::Null))
                )
            {
                objects.arena.remove_context(key).map_err(unsupported)?;
            }
            result.steps += 1;
            let mut frame = self.stack.pop().unwrap();
            let id = frame
                .object
                .ok_or_else(|| unsupported("live step binding missing"))?;
            if !matches!(objects.arena.list_pop(objects.stack).map_err(unsupported)?, Some(ObjectValue::Object(top)) if top == id)
            {
                return Err(unsupported("stack/frame object binding drift"));
            }
            // Pending state is on the actual shared step, never the frame copy.
            frame.request = objects.token(id)?;
            // The handler skips first. MoveUnit.resolve then clears success
            // and repeats its own skip check below; preserve both boundaries.
            if !skipped && let Some(key) = move_success {
                objects.arena.remove_context(key).map_err(unsupported)?;
            }
            let compiled = &self.steps[frame.instruction];
            if skipped
                || active
                    .filter(|key| !self.symbols.name(*key).is_empty())
                    .is_some_and(|key| {
                        !objects
                            .arena
                            .context_value(key)
                            .is_some_and(|value| objects.arena.truthy(&value))
                    })
                || skip
                    .filter(|key| !self.symbols.name(*key).is_empty())
                    .is_some_and(|key| {
                        objects
                            .arena
                            .context_value(key)
                            .is_some_and(|value| !matches!(value, ObjectValue::Atom(Datum::Null)))
                    })
            {
                continue;
            }
            let mandatory = top_mandatory;
            let mut request = None;
            match &compiled.instruction {
                Instruction::Set { .. } => {
                    let key = self.passive_text_bound(objects, id, phases::Field::FlagKey)?;
                    let value = objects
                        .arena
                        .field(id, objects.value)
                        .ok_or_else(|| unsupported("live Set.value missing"))?;
                    objects.arena.set_context(key, value).map_err(unsupported)?;
                }
                Instruction::Add { .. } => {
                    let key = self.passive_text_bound(objects, id, phases::Field::FlagKey)?;
                    let amount = match objects
                        .arena
                        .field(id, objects.phases.k(phases::Field::Amount))
                    {
                        Some(ObjectValue::Atom(Datum::Int(value))) => value,
                        Some(ObjectValue::Atom(Datum::Bool(value))) => i64::from(value),
                        _ => {
                            return Err(unsupported(
                                "live context addition amount must be an integer",
                            ));
                        }
                    };
                    let value = match objects.arena.context_value(key) {
                        Some(ObjectValue::Atom(Datum::Float(v))) => Datum::Float(v + amount as f64),
                        Some(ObjectValue::Atom(Datum::Int(v))) => Datum::Int(
                            v.checked_add(amount)
                                .ok_or_else(|| unsupported("integer addition outside i64"))?,
                        ),
                        Some(ObjectValue::Atom(Datum::Bool(v))) => Datum::Int(
                            i64::from(v)
                                .checked_add(amount)
                                .ok_or_else(|| unsupported("integer addition outside i64"))?,
                        ),
                        _ => Datum::Int(amount),
                    };
                    objects.write(key, value)?;
                }
                Instruction::Record { .. } => {
                    let read_key = |name| match self
                        .symbols
                        .lookup(name)
                        .and_then(|field| objects.arena.field(id, field))
                    {
                        Some(ObjectValue::Atom(Datum::Text(key))) => Ok(key),
                        _ => Err(unsupported("record target key missing")),
                    };
                    let input = read_key("input_key")?;
                    let output = read_key("output_list_key")?;
                    if let Some(value) = objects
                        .arena
                        .context_value(input)
                        .filter(|value| objects.arena.truthy(value))
                    {
                        let target = match objects.arena.context_value(output) {
                            None => {
                                let list = objects.arena.allocate(&Datum::List(Vec::new()));
                                objects
                                    .arena
                                    .set_context(output, list.clone())
                                    .map_err(unsupported)?;
                                Some(list)
                            }
                            value => value,
                        };
                        if let Some(ObjectValue::Object(target)) = target
                            && objects.arena.is_mutable_list(target)
                        {
                            if objects.protected(target) {
                                return Err(unsupported(
                                    "context append aliases an unbound world/step operand",
                                ));
                            }
                            objects.arena.append(target, value).map_err(unsupported)?;
                        }
                    }
                }
                Instruction::LiveCardSelect => {
                    request = self.card_selection_bound(
                        objects,
                        id,
                        card_selection
                            .ok_or_else(|| unsupported("live card selection plan missing"))?,
                        mandatory,
                    )?;
                }
                Instruction::LiveForceDefenseMovement => {
                    if let Some(plan) = forced_movement {
                        self.force_defense_movement_bound(objects, plan)?;
                    }
                }
                Instruction::LiveMoveSequence => self.move_sequence_bound(
                    objects,
                    sequence.ok_or_else(|| unsupported("move sequence plan missing"))?,
                    mandatory,
                )?,
                Instruction::LiveHexSelect => {
                    request = self.hex_selection_bound(
                        objects,
                        id,
                        hex_selection.ok_or_else(|| unsupported("hex selection plan missing"))?,
                        mandatory,
                    )?
                }
                Instruction::LiveTokenPlace => result.events.extend(self.token_place_bound(
                    objects,
                    token_place.ok_or_else(|| unsupported("token placement plan missing"))?,
                )?),
                Instruction::LiveTokenPath { .. } => self.token_path_bound(
                    objects,
                    id,
                    token_path.ok_or_else(|| unsupported("token path plan missing"))?,
                )?,
                Instruction::LiveTokenBatch => self.token_batch_bound(
                    objects,
                    token_batch.ok_or_else(|| unsupported("batch plan missing"))?,
                    mandatory,
                )?,
                Instruction::LiveTurret { .. } => result.events.extend(self.turret_bound(
                    objects,
                    turret.ok_or_else(|| unsupported("turret plan missing"))?,
                    mandatory,
                )?),
                Instruction::LiveRockUltimate => {
                    self.rock_ultimate_bound(objects, rock_ultimate)?
                }
                Instruction::LiveTokenRemove => result
                    .events
                    .extend(self.token_remove_bound(objects, token_remove)?),
                Instruction::LiveConvertCardItem => result
                    .events
                    .extend(self.convert_item_bound(objects, convert_item)?),
                Instruction::LiveAfterAttackText => {
                    self.after_attack_bound(objects, after_attack)?
                }
                Instruction::LiveRetrieve => result
                    .events
                    .extend(self.retrieve_card_bound(objects, retrieve)?),
                Instruction::LiveQuery { .. }
                | Instruction::LiveContextCondition
                | Instruction::LiveCheckUnitFilters
                | Instruction::LiveDistanceCheck
                | Instruction::LiveUnitTypeCheck
                | Instruction::LiveUnitOnBoard
                | Instruction::LiveBooleanCombine
                | Instruction::LiveAdjacentEnemies
                | Instruction::LiveZoneChange
                | Instruction::LiveCardCount
                | Instruction::LiveRoundDefeat => {
                    self.query_bound(
                        objects,
                        query.ok_or_else(|| unsupported("live query plan missing"))?,
                    )?;
                }
                Instruction::LiveUnitSelect => {
                    request = self.unit_selection_bound(
                        objects,
                        id,
                        unit_selection.ok_or_else(|| unsupported("unit selection plan missing"))?,
                        mandatory,
                    )?;
                }
                Instruction::LiveAttackSequence => self.attack_sequence_bound(
                    objects,
                    id,
                    attack.ok_or_else(|| unsupported("attack plan missing"))?,
                    mandatory,
                )?,
                Instruction::LivePreMovement => self.pre_movement_bound(objects, pre_move)?,
                Instruction::LivePreDiscard => self.pre_discard_bound(
                    objects,
                    id,
                    pre_discard.ok_or_else(|| unsupported("pre-discard plan missing"))?,
                )?,
                Instruction::LiveReaction => {
                    request = self.reaction_bound(
                        objects,
                        id,
                        reaction.ok_or_else(|| unsupported("reaction plan missing"))?,
                    )?
                }
                Instruction::LiveCombatResolve => result.events.extend(
                    self.combat_resolution_bound(
                        objects,
                        combat_resolve
                            .ok_or_else(|| unsupported("combat resolution plan missing"))?,
                    )?,
                ),
                Instruction::LiveAdjacentSnapshot => self.adjacent_snapshot_bound(
                    objects,
                    adjacent.ok_or_else(|| unsupported("adjacent snapshot plan missing"))?,
                )?,
                Instruction::LiveTargetDefeated => {
                    let (key, value) =
                        target_defeated.ok_or_else(|| unsupported("target defeat plan missing"))?;
                    objects
                        .arena
                        .set_context(key, ObjectValue::Atom(Datum::Bool(value)))
                        .map_err(unsupported)?;
                }
                Instruction::LiveDefeat => {
                    result.events.extend(self.defeat_bound(objects, defeat)?)
                }
                Instruction::LiveRemoveUnit => result
                    .events
                    .extend(self.remove_unit_bound(objects, removal)?),
                Instruction::LiveSoloWin => {
                    if let Some(outcome) = solo_win {
                        self.solo_win_bound(objects, outcome)?;
                    }
                }
                Instruction::LiveGameOver => result.events.push(self.game_over_bound(
                    objects,
                    game_over.ok_or_else(|| unsupported("game over plan missing"))?,
                )?),
                Instruction::LiveProtection => {
                    let (offer, events) = self.protection_bound(
                        objects,
                        protection.ok_or_else(|| unsupported("protection plan missing"))?,
                    )?;
                    request = offer;
                    result.events.extend(events);
                }
                Instruction::LiveMultiSelect => {
                    request = self.multi_selection_bound(
                        objects,
                        id,
                        multi_selection
                            .ok_or_else(|| unsupported("multi selection plan missing"))?,
                    )?;
                }
                Instruction::LiveRuneInscription => {
                    request = self.inscribe_runes_bound(objects, id, &mut result.events)?;
                }
                Instruction::LiveDirectionalMove => {
                    self.directional_move_bound(objects, id, &mut result.events)?;
                }
                Instruction::LiveChooseCardColor => {
                    request = self.choose_card_color_bound(objects, id)?;
                }
                Instruction::LiveLaugh => {
                    request = self.laugh_bound(objects, id, &mut result.events)?;
                }
                Instruction::LiveRevealHandCard => {
                    result
                        .events
                        .extend(self.reveal_hand_card_bound(objects, id)?);
                }
                Instruction::LiveCardGuess { reveal } => {
                    request = self.card_guess_bound(objects, id, *reveal, &mut result.events)?;
                }
                Instruction::LiveRuneChoice => {
                    request = self.rune_bound(
                        objects,
                        id,
                        rune.ok_or_else(|| unsupported("rune selection plan missing"))?,
                    )?
                }
                Instruction::LivePassive => self.passive_bound(
                    objects,
                    passive.ok_or_else(|| unsupported("passive plan missing"))?,
                )?,
                Instruction::LiveOfferPassive => {
                    request = self.offer_bound(
                        objects,
                        offer.ok_or_else(|| unsupported("passive offer plan missing"))?,
                    )?
                }
                Instruction::LiveMarkPassive => self.mark_passive_bound(
                    objects,
                    mark.ok_or_else(|| unsupported("passive mark plan missing"))?,
                )?,
                Instruction::LiveDiscard => {
                    self.discard_bound(
                        objects,
                        discard.ok_or_else(|| unsupported("discard plan missing"))?,
                    )?;
                }
                Instruction::LiveDiscardOrDefeat => {
                    self.discard_or_defeat_bound(objects, discard_or_defeat)?;
                }
                Instruction::LiveItemSwap => {
                    result
                        .events
                        .extend(self.swap_item_card_bound(objects, id)?);
                }
                Instruction::LiveDeckCardSwap => result
                    .events
                    .extend(self.swap_deck_card_bound(objects, id)?),
                Instruction::LiveResolvedCardSwap => {
                    result
                        .events
                        .extend(self.swap_resolved_cards_bound(objects, id)?);
                }
                Instruction::LiveCardSwap => self.swap_cards_bound(objects, id)?,
                Instruction::LiveColorDiscard => {
                    self.color_discard_bound(objects, color_discard)?;
                }
                Instruction::LiveForceDiscard => {
                    self.forced_discard_bound(
                        objects,
                        forced_discard.ok_or_else(|| unsupported("forced discard plan missing"))?,
                    )?;
                }
                Instruction::LiveMineChoice => {
                    request = self.mine_choice_bound(
                        objects,
                        id,
                        mine_choice.ok_or_else(|| unsupported("live mine choice plan missing"))?,
                    )?;
                }
                Instruction::LiveTriggerMines => {
                    result.events.extend(
                        self.mine_trigger_bound(
                            objects,
                            mine_trigger
                                .ok_or_else(|| unsupported("live mine trigger plan missing"))?,
                        )?,
                    );
                }
                Instruction::LiveMove => {
                    if let Some(event) = self.move_bound(
                        objects,
                        id,
                        movement.ok_or_else(|| unsupported("live movement plan missing"))?,
                        mandatory,
                    )? {
                        result.events.push(event);
                    }
                }
                Instruction::LiveTravel { .. } => {
                    if let Some(event) = self.travel_bound(
                        objects,
                        travel_plan.ok_or_else(|| unsupported("live travel plan missing"))?,
                        mandatory,
                    )? {
                        result.events.push(event);
                    }
                }
                Instruction::LivePerformPrimary => {
                    request = self.perform_primary_bound(
                        objects,
                        id,
                        perform_primary
                            .ok_or_else(|| unsupported("primary performance plan missing"))?,
                    )?;
                }
                Instruction::LiveMinionRespawn => {
                    request = self.minion_respawn_bound(
                        objects,
                        id,
                        minion_respawn.ok_or_else(|| unsupported("minion respawn plan missing"))?,
                        &mut result.events,
                    )?;
                }
                Instruction::LiveSpell(_) => {
                    request = self.spell_bound(
                        objects,
                        id,
                        spell_plan.ok_or_else(|| unsupported("spell lifecycle plan missing"))?,
                        &mut result.events,
                    )?;
                }
                Instruction::LivePerformCard => {
                    request = self.perform_card_bound(
                        objects,
                        id,
                        perform_card.ok_or_else(|| unsupported("copied-card plan missing"))?,
                    )?;
                }
                Instruction::LiveCardAction => {
                    let started = rule_timer.as_ref().map(|_| Instant::now());
                    request = self.card_action_bound(
                        objects,
                        card_action.ok_or_else(|| unsupported("card-action plan missing"))?,
                    )?;
                    if let Some((profile, started)) = rule_timer.as_mut().zip(started) {
                        profile.action_apply_ns = Some(started.elapsed().as_nanos());
                    }
                }
                Instruction::LiveChoosePiece => {
                    request = self.acting_piece_bound(
                        objects,
                        id,
                        acting_piece.ok_or_else(|| unsupported("acting-piece plan missing"))?,
                    )?;
                }
                Instruction::LiveMirrorPush => {
                    request = self.mirror_push_bound(objects, id, mandatory)?;
                }
                Instruction::LiveRemovePiece => {
                    request = self.remove_piece_bound(objects, id, &mut result.events)?;
                }
                Instruction::LiveSpawnPiece => {
                    request = self.spawn_piece_bound(objects, id, &mut result.events)?;
                }
                Instruction::LiveSetPiece => {
                    let value = |name| {
                        self.symbols
                            .lookup(name)
                            .and_then(|key| objects.arena.field(id, key))
                    };
                    let hero = self.truthy_symbol_bound(objects, value("hero_id"))?;
                    let mut piece = self.truthy_symbol_bound(objects, value("piece_id"))?;
                    if piece.is_none()
                        && let Some(key) = self.truthy_symbol_bound(objects, value("piece_key"))?
                    {
                        piece =
                            self.truthy_symbol_bound(objects, objects.arena.context_value(key))?;
                    }
                    if let (Some(hero), Some(piece)) = (hero, piece) {
                        if self
                            .world
                            .piece_ids(hero)
                            .any(|candidate| candidate == piece)
                        {
                            objects
                                .arena
                                .set_root(
                                    objects.phases.k(phases::Field::Piece),
                                    ObjectValue::Atom(Datum::Text(piece)),
                                )
                                .map_err(unsupported)?;
                            self.world.acting_piece = Some(piece);
                        } else if mandatory {
                            self.abort_objects(objects)?;
                        }
                    }
                }
                Instruction::LivePush => {
                    if let Some(event) = self.push_bound(
                        objects,
                        push.ok_or_else(|| unsupported("live push plan missing"))?,
                        mandatory,
                    )? {
                        result.events.push(event);
                    }
                }
                Instruction::LiveRepeat { .. } => {
                    request = self.repeat_bound(
                        objects,
                        id,
                        repeat.ok_or_else(|| unsupported("live repeat plan missing"))?,
                    )?;
                }
                Instruction::LiveJourney => {
                    if let Some(event) = self.journey_bound(
                        objects,
                        journey.ok_or_else(|| unsupported("journey plan missing"))?,
                    )? {
                        result.events.push(event);
                    }
                }
                Instruction::LiveActionControl => {
                    if let Some(event) = self.action_control_bound(
                        objects,
                        action_control.ok_or_else(|| unsupported("action control plan missing"))?,
                    )? {
                        result.events.push(event);
                    }
                }
                Instruction::LiveSetInitiative => {
                    if let Some(event) = self.initiative_bound(
                        objects,
                        initiative.ok_or_else(|| unsupported("initiative plan missing"))?,
                    )? {
                        result.events.push(event);
                    }
                }
                Instruction::LiveDrag | Instruction::LiveTokenMove => {
                    if let Some(event) = self.co_movement_bound(
                        objects,
                        co_movement.ok_or_else(|| unsupported("co-movement plan missing"))?,
                    )? {
                        result.events.push(event);
                    }
                }
                Instruction::LiveMoveToward => {
                    if let Some(plan) = move_toward {
                        self.move_toward_bound(objects, plan)?;
                    }
                }
                Instruction::LiveCoordinates(_) => {
                    self.coordinates_bound(
                        objects,
                        coordinates.ok_or_else(|| unsupported("live coordinate plan missing"))?,
                    )?;
                }
                Instruction::LivePrimaryText => {
                    self.primary_text_bound(
                        objects,
                        primary_text
                            .ok_or_else(|| unsupported("live primary text plan missing"))?,
                    )?;
                }
                Instruction::LiveDefenseBody { .. } => {
                    self.defense_bound(
                        objects,
                        defense_body.ok_or_else(|| unsupported("live defense plan missing"))?,
                    )?;
                }
                Instruction::LiveResource => {
                    if let Some(event) = self.resource_bound(
                        objects,
                        resource.ok_or_else(|| unsupported("live resource plan missing"))?,
                        &mut result.events,
                    )? {
                        result.events.push(event);
                    }
                }
                Instruction::LiveCreateEffect => {
                    if let Some(event) = self.create_effect_bound(
                        objects,
                        effect_creation
                            .ok_or_else(|| unsupported("live effect creation plan missing"))?,
                    )? {
                        result.events.push(event);
                    }
                }
                Instruction::LiveMarker { .. } => {
                    if let Some(event) = self.marker_bound(
                        objects,
                        marker.ok_or_else(|| unsupported("live marker plan missing"))?,
                    )? {
                        result.events.push(event);
                    }
                }
                Instruction::LiveSwap => {
                    if let Some(event) = self.swap_bound(
                        objects,
                        swap.ok_or_else(|| unsupported("live swap plan missing"))?,
                        mandatory,
                    )? {
                        result.events.push(event);
                    }
                }
                Instruction::LivePlace => {
                    if let Some(event) = self.place_bound(
                        objects,
                        placement.ok_or_else(|| unsupported("live placement plan missing"))?,
                        mandatory,
                    )? {
                        result.events.push(event);
                    }
                }
                Instruction::LiveReturnMinions => {
                    request = self.return_minions_bound(
                        objects,
                        id,
                        return_plan
                            .ok_or_else(|| unsupported("live minion-return plan missing"))?,
                    )?;
                }
                Instruction::Battle(_) => {
                    request = self.battle_bound(
                        objects,
                        id,
                        battle_plan.ok_or_else(|| unsupported("arena battle plan missing"))?,
                    )?;
                }
                Instruction::Flow(crate::turn::FlowStep::Advance) => {
                    let ids = objects.phases.effect_ids(&objects.arena, id)?;
                    self.preflight_bound_advance(objects)?;
                    self.remove_effects_bound(objects, &ids)?;
                    self.advance_turn_bound(objects)?;
                }
                Instruction::Flow(crate::turn::FlowStep::Next) => {
                    self.apply_actor_choice_bound(
                        objects,
                        actor_choice.ok_or_else(|| unsupported("actor decision missing"))?,
                    )?;
                }
                Instruction::Flow(crate::turn::FlowStep::Confirm(_)) => {
                    request = self.confirm_bound(objects, id)?;
                }
                Instruction::Flow(crate::turn::FlowStep::Finalize(_)) => {
                    let hero = self.finalize_bound(objects, id)?;
                    result.events.push(NativeEvent::TurnEnded(hero));
                }
                Instruction::Flow(crate::turn::FlowStep::Tie(_)) => {
                    request = self.tie_bound(objects, id)?;
                }
                Instruction::RetrieveUnresolved { prompt, .. } => {
                    let prompt = *prompt;
                    let (next, event) = self.retrieve_bound(objects, id, prompt)?;
                    request = next;
                    result.events.extend(event);
                }
                Instruction::FlipCoin => {
                    self.team = if self.team == Team::Red {
                        Team::Blue
                    } else {
                        Team::Red
                    };
                    let color = if self.team == Team::Blue {
                        phases::Field::Blue
                    } else {
                        phases::Field::Red
                    };
                    objects
                        .arena
                        .set_root(
                            objects.phases.k(phases::Field::Coin),
                            ObjectValue::Atom(Datum::Text(objects.phases.k(color))),
                        )
                        .map_err(unsupported)?;
                    result.events.push(NativeEvent::TieBreakerFlipped {
                        blue: self.team == Team::Blue,
                    });
                }
                Instruction::Flow(crate::turn::FlowStep::EndPhase) => {
                    self.end_phase_bound(objects)?;
                }
                Instruction::FinishedEffect => {}
                Instruction::RestoreContext { .. } => objects.restore_scope()?,
                Instruction::RestoreAction => objects.restore_type()?,
                Instruction::SetActor { .. } => self.actor_bound(
                    objects,
                    actor.ok_or_else(|| unsupported("actor plan missing"))?,
                )?,
                Instruction::ForEach { .. } => {
                    let operand = |name| match self
                        .symbols
                        .lookup(name)
                        .and_then(|key| objects.arena.field(id, key))
                    {
                        Some(ObjectValue::Atom(Datum::Text(value))) => Ok(value),
                        _ => Err(unsupported("live loop context key missing")),
                    };
                    let list = operand("list_key")?;
                    let item = operand("item_key")?;
                    let index = match objects.arena.field(id, objects.index) {
                        Some(ObjectValue::Atom(Datum::Int(index))) => index,
                        _ => return Err(unsupported("live loop index missing")),
                    };
                    let (len, value) = match objects.arena.context_value(list) {
                        None => continue,
                        Some(ObjectValue::Object(list)) => {
                            let values = objects
                                .arena
                                .sequence(list)
                                .ok_or_else(|| unsupported("ForEach source requires a sequence"))?;
                            let len = i64::try_from(values.len())
                                .map_err(|_| unsupported("loop length outside i64"))?;
                            if index >= len {
                                continue;
                            }
                            let offset = if index < 0 {
                                len.checked_add(index)
                            } else {
                                Some(index)
                            };
                            let value = offset
                                .and_then(|index| usize::try_from(index).ok())
                                .and_then(|index| values.get(index))
                                .cloned()
                                .ok_or_else(|| {
                                    EngineError::Rule("ForEach index outside list".into())
                                })?;
                            (len, value)
                        }
                        _ => return Err(unsupported("ForEach source requires a sequence")),
                    };
                    objects
                        .arena
                        .set_context(item, value)
                        .map_err(unsupported)?;
                    let next = index
                        .checked_add(1)
                        .ok_or_else(|| unsupported("loop index outside i64"))?;
                    objects
                        .arena
                        .set_field(id, objects.index, ObjectValue::Atom(Datum::Int(next)))
                        .map_err(unsupported)?;
                    let mut children = Vec::new();
                    for template in objects.template_objects(id)? {
                        let (object, instruction) = objects.copy_template(template)?;
                        Self::validate_returned_step_bound(objects, object)?;
                        let mut child = self.steps[instruction].initial.clone();
                        child.object = Some(object);
                        child.pending = None;
                        children.push(child);
                    }
                    if next < len {
                        objects
                            .arena
                            .append(objects.stack, ObjectValue::Object(id))
                            .map_err(unsupported)?;
                        self.stack.push(frame);
                    }
                    for child in children.into_iter().rev() {
                        objects
                            .arena
                            .append(objects.stack, ObjectValue::Object(child.object.unwrap()))
                            .map_err(unsupported)?;
                        self.stack.push(child);
                    }
                    continue;
                }
                Instruction::Log => {}
                Instruction::Number { labels, .. } => {
                    let output = self.passive_text_bound(objects, id, phases::Field::Output)?;
                    let prompt = self.passive_text_bound(objects, id, phases::Field::Prompt)?;
                    let auto = self.spatial_bool_bound(objects, id, phases::Field::Auto)?;
                    let owner =
                        self.spatial_text_bound(objects, id, phases::Field::OverridePlayer)?;
                    let owner_key =
                        self.spatial_text_bound(objects, id, phases::Field::OverridePlayerKey)?;
                    let discard =
                        self.spatial_text_bound(objects, id, phases::Field::DecisionPurpose)?
                            == Some(objects.phases.k(phases::Field::DiscardPurpose));
                    let Some(ObjectValue::Object(options)) =
                        objects.arena.field(id, objects.number_options)
                    else {
                        return Err(unsupported("live number options missing"));
                    };
                    let options = objects
                        .arena
                        .sequence(options)
                        .ok_or_else(|| unsupported("live number options must be a list"))?
                        .iter()
                        .map(|value| match value {
                            ObjectValue::Atom(Datum::Int(value)) => Ok(*value),
                            _ => Err(unsupported("live number option must be an integer")),
                        })
                        .collect::<Result<Vec<_>>>()?;
                    if options.is_empty() {
                        if mandatory {
                            self.abort_objects(objects)?;
                        } else {
                            objects.arena.remove_context(output).map_err(unsupported)?;
                        }
                        continue;
                    }
                    let selected = objects.selected(id, self.selection_key);
                    if auto && options.len() == 1 && mandatory {
                        objects.write(output, Datum::Int(options[0]))?;
                        objects.clear_pending(id)?;
                        continue;
                    }
                    if matches!(selected, Some(ObjectValue::Atom(Datum::Text(text))) if text == self.skip)
                        && !mandatory
                    {
                        objects.arena.remove_context(output).map_err(unsupported)?;
                        objects.clear_pending(id)?;
                        continue;
                    }
                    let number = match selected {
                        Some(ObjectValue::Atom(value)) if !matches!(value, Datum::Null) => {
                            value.to_integer(&self.symbols).map_err(unsupported)?
                        }
                        _ => None,
                    };
                    if let Some(number) = number.filter(|number| options.contains(number)) {
                        objects.write(output, Datum::Int(number))?;
                        objects.clear_pending(id)?;
                        continue;
                    }
                    let raw_owner = prompt_actor(
                        owner,
                        owner_key,
                        self.planning.actor,
                        &self.symbols,
                        |key| objects.symbol_from_context(key, &self.symbols),
                    )?;
                    request = Some(NativeRequest {
                        controlled: None,
                        id: RequestToken::Generated(0),
                        owner: self.normalized_owner(raw_owner.unwrap_or(self.no_actor)),
                        prompt,
                        movement_range: None,
                        pre_action_range: None,
                        choices: RequestChoices::Numbers {
                            values: options.clone(),
                            labels: labels.clone(),
                        },
                        can_skip: !mandatory,
                        discard,
                    });
                }
                _ => return Err(unsupported("unbound shared-object instruction")),
            }
            if let Some(request) = request {
                self.issue_bound_request(objects, id, frame, request)?;
                return Ok(result);
            }
        }
        Ok(result)
    }
    /// Journal the live arena and typed rows while staging the remaining session state.
    fn transition_bound<T>(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        apply: impl FnOnce(&mut Self, &mut BoundObjects, Frame) -> Result<T>,
    ) -> Result<T> {
        let rows = self.world.take_rows();
        let mut snapshot = self.clone();
        self.world.put_rows(rows);
        let world_checkpoint = self.world.begin_rows_rollback();
        if self.count_observer.observed() {
            snapshot.count_observer.record();
            self.count_observer.clear();
        }
        let parent_recording = objects.arena.is_recording();
        let checkpoint = objects.arena.begin_rollback();
        let readonly = objects.readonly.clone();
        let programs = objects.programs.clone();
        let copied_bindings = objects.copied_binding_undo.0.len();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut frame = self
                .stack
                .pop()
                .ok_or_else(|| unsupported("staged transition frame missing"))?;
            if !matches!(objects.arena.list_pop(objects.stack).map_err(unsupported)?, Some(ObjectValue::Object(id)) if id == step)
            {
                return Err(unsupported("staged transition stack drift"));
            }
            frame.request = objects.token(step)?;
            apply(self, objects, frame)
        }));
        match result {
            Ok(Ok(value)) => {
                objects.arena.commit_rollback(checkpoint);
                if !parent_recording {
                    objects.copied_binding_undo.0.truncate(copied_bindings);
                }
                self.world.commit_rows(world_checkpoint);
                Ok(value)
            }
            other => {
                objects.arena.rollback_to(checkpoint);
                objects.readonly = readonly;
                objects.programs = programs;
                objects.restore_copied_bindings(copied_bindings);
                self.world
                    .restore_rows(world_checkpoint, &mut snapshot.world);
                *self = snapshot;
                match other {
                    Ok(error) => error,
                    Err(panic) => std::panic::resume_unwind(panic),
                }
            }
        }
    }
    fn issue_bound_request(
        &mut self,
        objects: &mut BoundObjects,
        id: ObjectId,
        mut frame: Frame,
        request: NativeRequest,
    ) -> Result<()> {
        let request = self.issue(&mut frame, request)?;
        let token = match request.id {
            RequestToken::Imported(id) => ObjectValue::Atom(Datum::Text(id)),
            RequestToken::Generated(id) => ObjectValue::Request(id),
        };
        objects
            .arena
            .set_field(id, objects.request, token)
            .map_err(unsupported)?;
        objects
            .arena
            .append(objects.stack, ObjectValue::Object(id))
            .map_err(unsupported)?;
        self.stack.push(frame);
        self.last_request = Some(request);
        Ok(())
    }
    pub(super) fn unstick_abort(&mut self) -> Result<()> {
        self.with_live_objects(|session, objects| {
            if session.last_request.take().is_some() {
                session.pop_aborted_step(objects)?;
            }
            session.abort_objects(objects)
        })
    }

    pub(super) fn unstick_end_turn(&mut self) -> Result<()> {
        use phases::Field;
        self.with_live_objects(|session, objects| {
            objects
                .arena
                .clear_list(objects.stack)
                .map_err(unsupported)?;
            session.stack.clear();
            session.last_request = None;
            let step = if let Some(actor) = session.planning.actor {
                session.hero_phase_step_bound(objects, "finalize_hero", actor)?
            } else {
                objects
                    .arena
                    .clear_dictionary(objects.arena.context_id())
                    .map_err(unsupported)?;
                session.context.clear();
                session.planning.owner = None;
                session.world.acting_piece = None;
                for field in [
                    objects.phases.k(Field::Owner),
                    objects.phases.k(Field::Piece),
                ] {
                    objects
                        .arena
                        .set_root(field, ObjectValue::Atom(Datum::Null))
                        .map_err(unsupported)?;
                }
                session.phase_step(objects, "find_actor")?
            };
            session.push_live_steps_bound(objects, &[step])
        })
    }

    pub(super) fn unstick_force_actor(&mut self, actor: Symbol) -> Result<()> {
        self.with_live_objects(|session, objects| {
            for field in [objects.actor, objects.phases.k(phases::Field::Owner)] {
                objects
                    .arena
                    .set_root(field, ObjectValue::Atom(Datum::Text(actor)))
                    .map_err(unsupported)?;
            }
            session.planning.actor = Some(actor);
            session.planning.owner = Some(actor);
            Ok(())
        })?;
        self.invalidate_table_prompt()
    }

    pub(super) fn invalidate_table_prompt(&mut self) -> Result<()> {
        if self.last_request.is_none() {
            return Ok(());
        }
        self.with_live_objects(|session, objects| {
            let frame = session
                .stack
                .last_mut()
                .ok_or_else(|| unsupported("pending step missing"))?;
            let step = frame
                .object
                .ok_or_else(|| unsupported("pending step binding missing"))?;
            objects
                .arena
                .set_field(step, objects.request, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
            frame.request = None;
            session.last_request = None;
            Ok(())
        })
    }

    fn pop_aborted_step(&mut self, objects: &mut BoundObjects) -> Result<Frame> {
        let frame = self
            .stack
            .pop()
            .ok_or_else(|| unsupported("aborted step missing"))?;
        let id = frame
            .object
            .ok_or_else(|| unsupported("aborted step object missing"))?;
        if !matches!(objects.arena.list_pop(objects.stack).map_err(unsupported)?, Some(ObjectValue::Object(top)) if top == id)
        {
            return Err(unsupported("aborted stack/frame binding drift"));
        }
        Ok(frame)
    }

    fn abort_objects(&mut self, objects: &mut BoundObjects) -> Result<()> {
        let truthy = |field| {
            objects
                .arena
                .context_value(objects.phases.k(field))
                .is_some_and(|value| objects.arena.truthy(&value))
        };
        let pending_combat = if truthy(phases::Field::DefenseCardId)
            && truthy(phases::Field::PrimaryDefense)
        {
            self.stack.iter().enumerate().rev().take_while(|(_, frame)| {
                let instruction = &self.steps[frame.instruction].instruction;
                !matches!(
                    instruction,
                    Instruction::FinishedEffect
                        | Instruction::LiveSpell(crate::spells::SpellOp::Wish)
                        | Instruction::Flow(crate::turn::FlowStep::Confirm(_) | crate::turn::FlowStep::Finalize(_))
                ) && !matches!(instruction, Instruction::ParkedPhase(family)
                    if self.symbols.name(*family) == "finalize_hero_turn")
            }).find_map(|(index, frame)| {
                matches!(self.steps[frame.instruction].instruction, Instruction::LiveCombatResolve)
                    .then_some(index)
            }).map(|index| {
                let restore = self.stack.get(index + 1).and_then(|frame| {
                    matches!(self.steps[frame.instruction].instruction, Instruction::SetActor { .. })
                        .then_some(frame.object).flatten()
                }).and_then(|step| objects.arena.field(step, objects.phases.k(phases::Field::ActorKey)));
                let restores_attacker = matches!(restore, Some(ObjectValue::Atom(Datum::Text(key)))
                    if self.symbols.name(key) == "_pre_defense_actor");
                index + 1 + usize::from(restores_attacker)
            })
        } else {
            None
        };
        while let Some(frame) = self.stack.last() {
            if let Some(keep) = pending_combat {
                if self.stack.len() <= keep {
                    return Ok(());
                }
            } else {
                if matches!(
                    self.steps[frame.instruction].instruction,
                    Instruction::RestoreContext { other: true }
                ) {
                    return Ok(());
                }
                if matches!(
                    self.steps[frame.instruction].instruction,
                    Instruction::LiveSpell(crate::spells::SpellOp::Wish)
                        | Instruction::FinishedEffect
                        | Instruction::Flow(
                            crate::turn::FlowStep::Confirm(_) | crate::turn::FlowStep::Finalize(_)
                        )
                ) || matches!(self.steps[frame.instruction].instruction, Instruction::ParkedPhase(family) if self.symbols.name(family) == "finalize_hero_turn")
                {
                    break;
                }
            }
            let frame = self.pop_aborted_step(objects)?;
            if matches!(
                self.steps[frame.instruction].instruction,
                Instruction::RestoreContext { .. }
            ) {
                objects.restore_scope()?;
            }
        }
        while objects
            .arena
            .context_value(objects.action_stack)
            .is_some_and(|value| objects.arena.truthy(&value))
        {
            objects.restore_scope()?;
        }
        Ok(())
    }
    fn sync_effect_card_flags(
        &mut self,
        objects: &mut BoundObjects,
        before: &[bool],
    ) -> Result<()> {
        for (card, before) in self.world.cards().iter().zip(before) {
            if card.active == *before {
                continue;
            }
            objects
                .arena
                .set_physical_field(
                    card.source_instance,
                    objects.active,
                    Datum::Bool(card.active),
                )
                .map_err(unsupported)?;
        }
        Ok(())
    }
    /// Natural duration expiry returns the original stored step handles. It
    /// does not schedule them or perform phase advancement.
    pub fn expire_effects(
        &mut self,
        duration: crate::effects::Duration,
    ) -> Result<Vec<(Symbol, ObjectId)>> {
        self.with_live_objects(|session, objects| session.expire_effects_bound(objects, duration))
    }
    fn expire_effects_bound(
        &mut self,
        objects: &mut BoundObjects,
        duration: crate::effects::Duration,
    ) -> Result<Vec<(Symbol, ObjectId)>> {
        let indices = self.world.effects.duration_indices(duration);
        let finishing = objects.finishing_for_indices(&self.world.effects, &indices)?;
        let before = self
            .world
            .cards()
            .iter()
            .map(|card| card.active)
            .collect::<Vec<_>>();
        let expiry = self.world.expire_duration_effects(duration);
        objects.remove_effect_nodes(&expiry.ids)?;
        self.sync_effect_card_flags(objects, &before)?;
        Ok(finishing)
    }
    pub fn collect_turn_expiry(&self) -> Result<LiveEffectExpiry> {
        let objects = self
            .objects
            .as_ref()
            .ok_or_else(|| unsupported("turn expiry requires live object bindings"))?;
        self.collect_turn_expiry_bound(objects)
    }
    fn collect_turn_expiry_bound(&self, objects: &BoundObjects) -> Result<LiveEffectExpiry> {
        let indices = self
            .world
            .effects
            .turn_expiry_indices(self.round.round, self.world.combat.turn);
        let finishing = objects.finishing_for_indices(&self.world.effects, &indices)?;
        let ids = indices
            .iter()
            .map(|index| self.world.effects.instances[*index].id)
            .collect();
        Ok(LiveEffectExpiry { finishing, ids })
    }
    pub fn expire_turn_effects(&mut self) -> Result<LiveEffectExpiry> {
        let expiry = self.collect_turn_expiry()?;
        self.remove_effects(&expiry.ids)?;
        Ok(expiry)
    }
    /// Queue the real after-reveal trigger payloads with fresh actor/abort
    /// wrappers and original mutable step handles. No JSON runs in this path.
    pub fn queue_after_cards_played(&mut self) -> Result<()> {
        self.with_live_objects(|session, objects| {
            let steps = session.collect_after_cards_played_bound(objects)?;
            session.push_live_steps_bound(objects, &steps)
        })
    }
    fn collect_after_cards_played_bound(
        &mut self,
        objects: &mut BoundObjects,
    ) -> Result<Vec<ObjectId>> {
        let nodes = objects.effect_nodes()?;
        if nodes.len() != self.world.effects.instances.len() {
            return Err(unsupported("effect collection binding drift"));
        }
        let mut removed = Vec::new();
        let mut finishing = Vec::new();
        for (index, effect) in self.world.effects.instances.iter().enumerate() {
            if !effect.definition.after_cards_played {
                continue;
            }
            removed.push(index);
            if self.round.round == effect.created_round
                && self.world.combat.turn.checked_sub(1) == Some(effect.created_turn)
            {
                let list = objects.finishing_list(nodes[index])?;
                let steps = objects.steps_in_list(list)?;
                if !steps.is_empty() {
                    finishing.push((effect.source, steps));
                }
            }
        }
        if removed.is_empty() {
            return Ok(Vec::new());
        }
        // Validate every runnable payload and generation before mutating state.
        self.phase_generation_check(finishing.len(), 0)?;
        let before = self
            .world
            .cards()
            .iter()
            .map(|card| card.active)
            .collect::<Vec<_>>();
        self.world.remove_after_cards_played_effects();
        objects.remove_effect_rows(&removed)?;
        self.sync_effect_card_flags(objects, &before)?;
        self.wrap_finishings(objects, finishing)
    }
    /// Removal by ID is premature: no finishing payloads are returned or queued.
    pub fn remove_effects(&mut self, ids: &[Symbol]) -> Result<()> {
        self.with_live_objects(|session, objects| session.remove_effects_bound(objects, ids))
    }
    fn remove_effects_bound(&mut self, objects: &mut BoundObjects, ids: &[Symbol]) -> Result<()> {
        objects.remove_effect_nodes(ids)?;
        let before = self
            .world
            .cards()
            .iter()
            .map(|card| card.active)
            .collect::<Vec<_>>();
        self.world.remove_effects(ids);
        self.sync_effect_card_flags(objects, &before)
    }
    fn push_live_steps(&mut self, steps: &[ObjectId]) -> Result<()> {
        self.with_live_objects(|session, objects| session.push_live_steps_bound(objects, steps))
    }
    fn push_live_steps_bound(
        &mut self,
        objects: &mut BoundObjects,
        steps: &[ObjectId],
    ) -> Result<()> {
        // Validate the batch before appending any frame. The common
        // single-step case validates as it goes and needs just one lookup.
        if steps.len() > 1
            && steps
                .iter()
                .any(|object| objects.program(*object).is_none())
        {
            return Err(unsupported("stored step program is not compiled"));
        }
        for &object in steps.iter().rev() {
            let instruction = objects
                .program(object)
                .ok_or_else(|| unsupported("stored step program is not compiled"))?;
            let mut frame = self.steps[instruction].initial.clone();
            frame.object = Some(object);
            frame.pending = None;
            objects
                .arena
                .append(objects.stack, ObjectValue::Object(object))
                .map_err(unsupported)?;
            self.stack.push(frame);
        }
        objects.readonly.take();
        Ok(())
    }
    /// Boundary adapter for the source push_steps helper. Engine callers use
    /// the typed handles returned by expiry rather than resolving JSON paths.
    pub fn queue_steps(&mut self, path: &[ObjectPathPart]) -> Result<()> {
        let objects = self
            .objects
            .as_ref()
            .ok_or_else(|| unsupported("stored step scheduling requires live object bindings"))?;
        let ObjectValue::Object(list) = objects.arena.value_at_path(path).map_err(unsupported)?
        else {
            return Err(unsupported("step sequence missing"));
        };
        let steps = objects.steps_in_list(list)?;
        self.push_live_steps(&steps)
    }
    pub fn queue_finishing_steps(&mut self, list: ObjectId) -> Result<()> {
        let objects = self
            .objects
            .as_ref()
            .ok_or_else(|| unsupported("stored step scheduling requires live object bindings"))?;
        let steps = objects.steps_in_list(list)?;
        self.push_live_steps(&steps)
    }
    /// Same shallow scope operation used when the source starts a nested action.
    /// The actual action body/legality remains a separate engine operation.
    pub fn push_action_context(
        &mut self,
        action: Symbol,
        card: Symbol,
        owner: Symbol,
    ) -> Result<()> {
        let objects = self
            .objects
            .as_mut()
            .ok_or_else(|| unsupported("action scopes require live object bindings"))?;
        objects.push_scope(action, card, owner)
    }
    pub(super) fn submit_objects(&mut self, request: RequestToken, selection: Datum) -> Result<()> {
        if let Some(frame) = self.stack.last()
            && matches!(
                self.steps[frame.instruction].instruction,
                Instruction::LiveRuneInscription
            )
            && let Some(objects) = self.objects.as_ref()
            && let Some(id) = frame.object
            && objects.token(id)? == Some(request)
        {
            self.validate_inscription_bound(objects, id, &selection)?;
        }

        if let Some(frame) = self.stack.last()
            && matches!(
                self.steps[frame.instruction].instruction,
                Instruction::Flow(crate::turn::FlowStep::Upgrades)
            )
            && let Some(objects) = self.objects.as_ref()
            && let Some(id) = frame.object
            && objects.token(id)? == Some(request)
        {
            self.validate_upgrade_submission_bound(&selection)?;
        }
        let frame = self
            .stack
            .last_mut()
            .ok_or_else(|| EngineError::InvalidInput("No pending step to receive input".into()))?;
        let objects = self
            .objects
            .as_mut()
            .ok_or_else(|| unsupported("object arena missing"))?;
        let id = frame
            .object
            .ok_or_else(|| unsupported("live step binding missing"))?;
        if objects.token(id)? != Some(request) {
            return Err(EngineError::InvalidInput(
                "stale or unissued input request".into(),
            ));
        }
        let pending = objects.arena.allocate_input(self.selection_key, &selection);
        objects
            .arena
            .set_field(id, objects.pending, pending)
            .map_err(unsupported)?;
        objects
            .arena
            .set_field(id, objects.request, ObjectValue::Atom(Datum::Null))
            .map_err(unsupported)?;
        frame.request = None;
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn fixture_case(name: &str) -> Value {
    let base = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
    let index: Value = serde_json::from_reader(std::io::BufReader::new(
        crate::fixture_cache::open(base.join("object-execution.json")).unwrap(),
    ))
    .unwrap();
    let entry = index["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap();
    serde_json::from_reader(std::io::BufReader::new(
        crate::fixture_cache::open(base.join(entry["path"].as_str().unwrap())).unwrap(),
    ))
    .unwrap()
}

#[cfg(test)]
mod brogan_tests;

#[cfg(test)]
mod spellbook_tests;

#[cfg(test)]
mod spell_body_tests;

#[cfg(test)]
mod rollback_tests {
    use super::test_support::object_session;

    #[test]
    fn resolution_position_score_orders_targets_and_counts_minion_support() {
        use crate::world::{EntityKind, MinionKind};
        let mut session = object_session("object-execution-card-planning-cycle");
        let hero = session.world.heroes[0].clone();
        let red = session.symbols.lookup("RED");
        let gold = session.symbols.lookup("GOLD");
        let mut empty = hero.clone();
        empty.hand.clear();
        let world = &mut session.world;
        let enemy_hero = world
            .heroes
            .iter()
            .find(|row| row.team != hero.team)
            .unwrap()
            .id;
        let minion = |friendly, kind| {
            world
                .definitions
                .entities
                .iter()
                .find(|entity| {
                    entity.kind == EntityKind::Minion
                        && entity
                            .team
                            .is_some_and(|team| (team == hero.team) == friendly)
                        && entity.minion_kind == Some(kind)
                })
                .unwrap()
                .id
        };
        let enemy_melee = minion(false, MinionKind::Melee);
        let enemy_ranged = minion(false, MinionKind::Ranged);
        let friendly_melee = minion(true, MinionKind::Melee);
        let origin = world.tile_hexes()[0];
        let adjacent = world
            .tile_hexes()
            .iter()
            .copied()
            .find(|hex| origin.distance(*hex) == 1)
            .unwrap();
        let two_away = world
            .tile_hexes()
            .iter()
            .copied()
            .find(|hex| origin.distance(*hex) == 2)
            .unwrap();
        for (entity, _) in world.locations.clone() {
            world.remove_entity(entity).unwrap();
        }
        world.place_entity(hero.id, origin).unwrap();
        assert_eq!(session.hero_position_score(&hero, true, red, gold), 0);
        session.world.place_entity(enemy_melee, adjacent).unwrap();
        assert_eq!(session.hero_position_score(&hero, true, red, gold), 12);
        assert_eq!(session.hero_position_score(&hero, false, red, gold), 4);
        assert_eq!(session.hero_position_score(&empty, true, red, gold), 4);
        session.world.remove_entity(enemy_melee).unwrap();
        session.world.place_entity(enemy_hero, adjacent).unwrap();
        assert_eq!(session.hero_position_score(&hero, true, red, gold), 8);
        session.world.remove_entity(enemy_hero).unwrap();
        session
            .world
            .place_entity(friendly_melee, adjacent)
            .unwrap();
        assert_eq!(session.hero_position_score(&empty, true, red, gold), 1);
        session.world.place_entity(enemy_ranged, two_away).unwrap();
        assert_eq!(session.hero_position_score(&empty, true, red, gold), 0);
    }

    #[test]
    fn resolution_search_returns_live_move_and_restores_root_when_bounded() {
        use super::super::{
            DecisionMove, ExecutionBudget, OpponentPolicy, SearchLimits, SequenceVerdict,
        };
        let mut session = object_session("object-execution-confirmation-repeated-step");
        let budget = ExecutionBudget::default();
        let request = session.advance(&budget).unwrap().request.unwrap();
        let before = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
        let complete = session
            .search_resolution(
                request.owner,
                SearchLimits {
                    max_depth: 64,
                    max_nodes: 10,
                    opponent_policy: OpponentPolicy::Adversarial,
                    evaluation: Default::default(),
                },
                &budget,
            )
            .unwrap();
        assert_eq!(complete.completed_depth, 1);
        assert!(complete.score.is_some());
        assert!(complete.resolved);
        assert_eq!(complete.nodes, 2);
        let bounded = session
            .search_resolution(
                request.owner,
                SearchLimits {
                    max_depth: 2,
                    max_nodes: 1,
                    opponent_policy: OpponentPolicy::Adversarial,
                    evaluation: Default::default(),
                },
                &budget,
            )
            .unwrap();
        assert_eq!(bounded.completed_depth, 0);
        assert_eq!(bounded.nodes, 1);
        assert!(!bounded.resolved);
        assert_eq!(bounded.decision, complete.decision);
        assert!(
            matches!(bounded.decision, DecisionMove::Input { request: Some(token), .. } if token == request.id)
        );
        let checked = session
            .check_sequences(&[vec![bounded.decision]], &budget)
            .unwrap();
        assert_eq!(checked.verdicts, [SequenceVerdict::Legal { checked: 1 }]);
        assert_eq!(
            serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
            before
        );
    }

    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn generated_respawn_options_are_legal_and_leave_the_live_prompt_intact() {
        use super::super::{DecisionMove, ExecutionBudget, SequenceVerdict};
        let mut session = object_session("object-execution-hero-respawn-live-ordinary-first");
        let budget = ExecutionBudget::default();
        let request = session.advance(&budget).unwrap().request.unwrap();
        let other = session.symbols.lookup("hero_hanu").unwrap();
        assert!(session.resolution_choices(other).unwrap().is_empty());
        let moves = session.resolution_choices(request.owner).unwrap();
        let wire = session.request_json(&request);
        let expected = wire["options"].as_array().unwrap();
        assert!(!moves.is_empty());
        assert_eq!(moves.len(), expected.len());
        for (decision, option) in moves.iter().zip(expected) {
            let DecisionMove::Input {
                player,
                request: token,
                selection,
            } = decision
            else {
                panic!("expected input decision");
            };
            assert_eq!(*player, request.owner);
            assert_eq!(*token, Some(request.id));
            let value = option["metadata"].get("hex").unwrap_or(&option["id"]);
            assert_eq!(selection.to_json(&session.symbols), *value);
        }
        let before = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
        let sequences = moves
            .into_iter()
            .map(|decision| vec![decision])
            .collect::<Vec<_>>();
        let result = session.check_sequences(&sequences, &budget).unwrap();
        assert_eq!(result.applied_decisions, 0);
        assert_eq!(
            result.verdicts,
            vec![SequenceVerdict::Legal { checked: 1 }; sequences.len()]
        );
        assert_eq!(
            serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
            before
        );
    }

    #[test]
    fn sequence_checks_share_planning_prefixes_and_prune_failures() {
        use super::super::{DecisionMove, EngineError, ExecutionBudget, SequenceVerdict};
        let mut session = object_session("object-execution-planning-command-pass");
        let hero = session.symbols.lookup("hero_arien").unwrap();
        let pass = DecisionMove::Pass { hero };
        let before = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
        let sequences = [
            vec![],
            vec![pass.clone()],
            vec![pass.clone(), pass.clone()],
            vec![pass.clone(), pass.clone(), pass.clone()],
            vec![pass],
        ];
        let batch = session
            .check_sequences(&sequences, &ExecutionBudget::default())
            .unwrap();
        assert_eq!(batch.applied_decisions, 1);
        assert_eq!(batch.verdicts[0], SequenceVerdict::Legal { checked: 0 });
        assert_eq!(batch.verdicts[1], SequenceVerdict::Legal { checked: 1 });
        assert!(matches!(
            batch.verdicts[2],
            SequenceVerdict::Illegal { failed_at: 1, .. }
        ));
        assert_eq!(batch.verdicts[2], batch.verdicts[3]);
        assert_eq!(batch.verdicts[1], batch.verdicts[4]);
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let budget = ExecutionBudget {
            cancelled: Some(&cancelled),
            ..ExecutionBudget::default()
        };
        assert!(matches!(
            session.check_sequences(&sequences, &budget),
            Err(EngineError::Cancelled)
        ));
        assert_eq!(
            serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
            before
        );
    }

    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn session_rollback_restores_world_rows_across_nested_staging() {
        let mut session = object_session("object-execution-hero-respawn-live-ordinary-first");
        let initial = session.world.cards()[0].clone();
        let id = crate::cards::CardId(initial.source_instance);
        let initial_gold = session.world.heroes[0].gold;
        let before = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
        session
            .with_rollback(|outer| {
                outer.world.card_mut(id).unwrap().initiative_override = Some(41);
                outer.world.heroes[0].gold = 41;
                outer.with_rollback(|inner| {
                    let mut staged = inner.world.staged_clone();
                    staged.card_mut(id).unwrap().initiative_override = Some(42);
                    staged.card_mut(id).unwrap().initiative_override = Some(43);
                    staged.heroes[0].gold = 43;
                    inner.world = staged;
                    assert_eq!(inner.world.heroes[0].gold, 43);
                    assert_eq!(inner.world.card(id).unwrap().initiative_override, Some(43));
                    Ok(())
                })?;
                assert_eq!(outer.world.card(id).unwrap().initiative_override, Some(41));
                assert_eq!(outer.world.heroes[0].gold, 41);
                assert_eq!(
                    outer.clone().world.card(id).unwrap().initiative_override,
                    Some(41)
                );
                Ok(())
            })
            .unwrap();
        assert_eq!(
            session.world.card(id).unwrap().initiative_override,
            initial.initiative_override
        );
        assert_eq!(session.world.heroes[0].gold, initial_gold);
        assert_eq!(
            serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
            before
        );
    }

    #[test]
    fn world_fingerprint_tracks_combat_and_lane_counters_across_rollback() {
        let mut session = object_session("object-execution-live-battle-red-push-first");
        let before = session.world_state_fingerprint();
        let team = session.world.heroes[0].team;
        let (_, lives) = session
            .world
            .combat
            .team_lives()
            .find(|(id, _)| *id == team)
            .unwrap();
        session
            .with_rollback(|branch| {
                branch.world.combat.set_team_life(team, lives - 1).unwrap();
                let combat = branch.world_state_fingerprint();
                assert_ne!(combat, before);
                branch.with_rollback(|branch| {
                    branch.world.waves[0].1 -= 1;
                    assert_ne!(branch.world_state_fingerprint(), combat);
                    Ok(())
                })?;
                assert_eq!(branch.world_state_fingerprint(), combat);
                Ok(())
            })
            .unwrap();
        assert_eq!(session.world_state_fingerprint(), before);
    }

    #[test]
    fn fingerprints_track_entity_overrides_and_pending_round_state() {
        let mut session = object_session("object-execution-arena-travel-edge-barrier");
        let actor = session.symbols.lookup("hero_arien").unwrap();
        let token = session.symbols.lookup("mine").unwrap();
        let board_entity = session.symbols.lookup("travel-spatial").unwrap();
        let initial_world = session.world_state_fingerprint();
        let initial_execution = session.execution_fingerprint();
        session
            .with_rollback(|branch| {
                let immune = branch.world.entity(token).unwrap().immune_enemy_actions;
                branch
                    .world
                    .set_token_properties(token, Some(actor), !immune)
                    .unwrap();
                let token_world = branch.world_state_fingerprint();
                assert_ne!(token_world, initial_world);
                branch.with_rollback(|branch| {
                    branch
                        .world
                        .upsert_board_entity(board_entity, board_entity, Some(actor))
                        .unwrap();
                    assert_ne!(branch.world_state_fingerprint(), token_world);
                    branch.round.pending.push((actor, 1));
                    assert_ne!(branch.execution_fingerprint(), initial_execution);
                    Ok(())
                })?;
                assert_eq!(branch.world_state_fingerprint(), token_world);
                assert_eq!(branch.execution_fingerprint(), initial_execution);
                Ok(())
            })
            .unwrap();
        assert_eq!(session.world_state_fingerprint(), initial_world);
    }

    #[test]
    fn fingerprints_distinguish_generated_planning_and_resolution_branches() {
        use super::super::{DecisionMove, ExecutionBudget, NativeSession};
        use std::collections::HashMap;

        let mut root = object_session("object-execution-card-planning-cycle");
        let budget = ExecutionBudget::default();
        let player = root.world.heroes[0].id;
        let rows = root.joint_plans(player, true, 4).unwrap().plans;
        let replies = root.joint_plans(player, false, 4).unwrap().plans;
        let key = |worker: &mut NativeSession| {
            (
                worker.world_state_fingerprint(),
                worker.execution_fingerprint(),
            )
        };
        let before = key(&mut root);
        let mut states = HashMap::new();
        let mut repeated = 0;
        let mut check = |worker: &mut NativeSession| {
            let fingerprint = key(worker);
            let state = (
                serde_json::to_value(worker.export_envelope().unwrap()).unwrap(),
                worker
                    .last_request
                    .as_ref()
                    .map(|request| worker.request_json(request)),
            );
            match states.entry(fingerprint) {
                std::collections::hash_map::Entry::Occupied(entry) => {
                    assert_eq!(
                        *entry.get(),
                        state,
                        "different states have the same fingerprint"
                    );
                    repeated += 1;
                }
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(state);
                }
            }
        };
        for _ in 0..2 {
            for row in &rows {
                for reply in &replies {
                    let joint: Vec<_> = row.iter().chain(reply).copied().collect();
                    root.with_joint_planning(&joint, |branch| {
                        branch.advance_to_decision(&budget)?;
                        check(branch);
                        if let Some((owner, _)) = branch.search_player()? {
                            for choice in branch.resolution_choices(owner)?.into_iter().take(2) {
                                let DecisionMove::Input {
                                    request: Some(token),
                                    selection,
                                    ..
                                } = choice
                                else {
                                    panic!("expected a live resolution choice")
                                };
                                branch.with_consumed_request(|branch| {
                                    branch.submit(token, selection)?;
                                    branch.advance_to_decision(&budget)?;
                                    check(branch);
                                    Ok(())
                                })?;
                            }
                        }
                        Ok(())
                    })
                    .unwrap();
                    assert_eq!(key(&mut root), before);
                }
            }
        }
        assert!(states.len() > rows.len() * replies.len());
        assert!(repeated >= states.len());
    }

    #[test]
    fn fingerprints_preserve_ordered_state_after_reordered_simultaneous_commitments() {
        use super::super::ExecutionBudget;

        let root = object_session("object-execution-card-planning-cycle");
        let choices: Vec<_> = root
            .world
            .heroes
            .iter()
            .map(|hero| root.first_planning_commitment(hero.id).unwrap().unwrap())
            .collect();
        let mut forward = root.clone();
        let mut reversed = root;
        for (worker, order) in [
            (&mut forward, (0..choices.len()).collect::<Vec<_>>()),
            (&mut reversed, (0..choices.len()).rev().collect::<Vec<_>>()),
        ] {
            for index in order {
                let choice = choices[index];
                worker
                    .commit_card(choice.hero, choice.first.unwrap())
                    .unwrap();
            }
            worker
                .advance_to_decision(&ExecutionBudget::default())
                .unwrap();
        }
        let forward_state = forward.semantic_state().unwrap();
        let reversed_state = reversed.semantic_state().unwrap();
        assert_ne!(
            forward_state["public_revealed_cards"],
            reversed_state["public_revealed_cards"]
        );
        assert_ne!(
            forward_state["unresolved_hero_ids"],
            reversed_state["unresolved_hero_ids"]
        );
        assert_ne!(
            forward.world_state_fingerprint(),
            reversed.world_state_fingerprint()
        );
    }

    #[test]
    #[cfg_attr(
        not(feature = "full-parity"),
        ignore = "full-parity: frozen late-generation payload is not bundled; see docs/testing.md"
    )]
    fn session_rollback_recovers_arena_when_live_dispatch_panics() {
        let mut session = object_session("object-execution-hero-respawn-live-ordinary-first");
        let before = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session.with_rollback(|branch| {
                branch.with_live_objects::<()>(|branch, objects| {
                    objects.arena.list_pop(objects.stack).unwrap();
                    branch.next_step += 1;
                    panic!("interrupted live dispatch");
                })
            })
        }));
        assert!(panic.is_err());
        assert_eq!(
            serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
            before
        );
        assert!(session.advance_prefix(1).unwrap().request.is_some());
    }
}
