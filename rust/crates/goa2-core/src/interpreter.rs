//! Native stack execution. Definitions are compiled once; advancing uses typed
//! instructions/registers and does not serialize JSON or call Python.
//! Unrecognized instruction families reject at import. Dynamic rule closures
//! fail explicitly when reached; no approximate rule fallback is used.
use crate::{
    actions::{ActionResolution, ActionRules, ActionStep, Passive},
    attack::{AttackBinding, AttackRules, AttackSpec},
    card_selection::{CardSelectSpec, ForceBinding},
    codec::{Phase, StateEnvelope},
    combat::{CombatError, CombatEvent, CombatRules, DefeatBinding, GameOutcome},
    data::{Datum, Registers, Symbol, Symbols},
    geometry::Hex,
    markers::{MarkerEvent, MarkerPlacement, MarkerRules},
    primary::{PrimaryOperands, PrimaryValues},
    reactions::{DefenseOp, ReactionRules, Window},
    spatial::{HexFilter, Origin, SpatialContext, compile_filter},
    stats::{CardStats, MarkerKind, Stat},
    targeting::{UnitCandidates, UnitFilter},
    world::{EntityKind, NativeWorld},
};
mod arena_choices;
mod arena_match;
mod budget;
pub mod live_game;
mod loadout;
mod object_execution;
mod observation;
mod offline_match;
mod protocol;
pub use arena_match::{ArenaChoice, ArenaMatch, RecordedArenaChoice};
pub use budget::{ExecutionBudget, SearchNodeBudget, WorkBudget};
pub use loadout::{
    LoadoutHypothesis, PublicUpgradeBatch, SampledHand, UpgradeCheckpoint,
    replay_public_upgrade_batches,
};
pub use offline_match::{OfflineMatch, OfflinePolicy};
pub use protocol::{AdvanceResult, NativeEvent, NativeRequest, RequestChoices, RequestToken};
mod greedy;
pub use greedy::{GreedyPlan, GreedySearch, GreedySearchOptions, GreedyStep};
mod search;
pub use observation::{
    BotTeamObservation, HeroObservation, PlayerObservation, PrivateCards, PublicCard,
    TeamKnowledgeMode, VisibleCommitment,
};
mod sequences;
pub use object_execution::LiveEffectExpiry;
pub use search::{
    BotPlanningScope, BotTeamPlanningSearch, ForecastNodeShare, GameTurnRollout, JointPlans,
    OpponentPolicy, PlanningRolloutSample, PlanningSearch, PlanningSearchOptions,
    ResolutionReuseCensus, ResolutionScoreKind, ResolutionSearch, ResolutionStop,
    SampledPlanningSearch, SearchEvaluation, SearchLimits,
};
pub use sequences::{DecisionMove, PlanningCommitment, SequenceBatch, SequenceVerdict};

use serde_json::{Value, json};
use std::{sync::Arc, time::Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineError {
    Unsupported(String),
    InvalidInput(String),
    Rule(String),
    Cancelled,
    Deadline,
    NodeLimit,
    StepLimit,
}
impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for EngineError {}
type Result<T> = std::result::Result<T, EngineError>;
fn unsupported(message: impl Into<String>) -> EngineError {
    EngineError::Unsupported(message.into())
}

fn prompt_actor(
    owner: Option<Symbol>,
    key: Option<Symbol>,
    actor: Option<Symbol>,
    symbols: &Symbols,
    read: impl FnOnce(Symbol) -> Result<Option<Symbol>>,
) -> Result<Option<Symbol>> {
    if let Some(owner) = owner.filter(|id| !symbols.name(*id).is_empty()) {
        return Ok(Some(owner));
    }
    if let Some(key) = key.filter(|id| !symbols.name(*id).is_empty()) {
        return Ok(read(key)?.or(actor));
    }
    Ok(actor)
}

fn scope_actor(
    literal: Option<Symbol>,
    key: Option<Symbol>,
    symbols: &Symbols,
    read: impl FnOnce(Symbol) -> Result<Option<Symbol>>,
) -> Result<Option<Symbol>> {
    if let Some(key) = key.filter(|id| !symbols.name(*id).is_empty()) {
        read(key)
    } else {
        Ok(literal)
    }
}

#[derive(Clone, Debug, Hash)]
struct EffectBinding {
    card: Symbol,
    context: Option<(Symbol, Datum)>,
    range: Option<i64>,
    primary: Option<PrimaryValues>,
}
#[derive(Clone, Debug)]
struct PrimaryProgram {
    bodies: Vec<(Symbol, Vec<usize>)>,
    movement: usize,
    attack: usize,
    defense: usize,
    skill: usize,
}
#[derive(Clone, Debug)]
struct DefenseProgram {
    enter: usize,
    exit: usize,
    flag: usize,
    place: usize,
    remove: usize,
    reflect_select: Option<usize>,
    reflect_discard: Option<usize>,
    deflect_select: Option<usize>,
    deflect_discard: Option<usize>,
}
#[derive(Clone, Debug)]
struct FlowProgram {
    entries: Vec<(&'static str, usize)>,
}
#[derive(Clone, Debug)]
struct BattleProgram {
    remove: usize,
    place: usize,
    victory: usize,
    push: usize,
    casualties: usize,
    displace: usize,
}
#[derive(Clone, Debug)]
struct PassiveBody {
    effect: Symbol,
    owner: Symbol,
    templates: Vec<usize>,
}
#[derive(Clone, Debug, Hash)]
struct Frame {
    instruction: usize,
    object: Option<crate::objects::ObjectId>,
    pending: Option<Datum>,
    request: Option<RequestToken>,
    index: i64,
    repeat_checked: bool,
    unit_override: Option<Option<Symbol>>,
    generated: Option<u64>,
    defeat_prompt_victim: Option<Symbol>,
    range_override: Option<i32>,
    through_override: Option<bool>,
    defeat_override: Option<DefeatBinding>,
    outcome_override: Option<GameOutcome>,
    discard_override: Option<(Symbol, Symbol)>,
    attack_binding: Option<AttackBinding>,
    attack_followup: Option<Symbol>,
    effect: Option<EffectBinding>,
    offer_override: Option<crate::passives::Offer>,
    forced: Option<ForceBinding>,
    mine_trigger: Option<(Option<Symbol>, Symbol, Option<Symbol>)>,
    mine_snapshot: Option<(Vec<Symbol>, Symbol)>,
    protection: Option<crate::protection::ProtectionBinding>,
    mine_prechosen: bool,
    pre_move: Option<(Symbol, i32)>,
    pre_processed: Vec<Symbol>,
    action: Option<ActionStep>,
    multi: Vec<Symbol>,
    zone_return: Option<crate::minion_return::ReturnBinding>,
    battle: Option<crate::battle::BattleStep>,
    continuation: Option<crate::planning::Continuation>,
}
#[derive(Clone, Debug)]
struct Conditions {
    active: Option<Symbol>,
    skip: Option<Symbol>,
    mandatory: bool,
}
#[derive(Clone, Copy, Debug)]
enum CoordinateOp {
    TurnStart,
    Distance,
    Record,
    Compute,
    Adjacency,
}
#[derive(Clone, Debug)]
enum Instruction {
    ParkedPhase(Symbol),
    LiveReturnMinions,
    LivePlace,
    LiveSwap,
    LiveCreateEffect,
    LiveResource,
    LivePrimaryText,
    LiveCoordinates(CoordinateOp),
    LivePush,
    LiveTravel {
        sequence: bool,
    },
    LivePerformPrimary,
    LivePerformCard,
    LiveCardAction,
    LiveChoosePiece,
    LiveRepeat {
        templates: Vec<usize>,
    },
    LiveDefenseBody {
        on_block: bool,
    },
    LiveMarker {
        remove: bool,
    },
    LiveMove,
    LiveTokenMove,
    LiveDrag,
    LiveMoveToward,
    LiveSetInitiative,
    LiveActionControl,
    LiveJourney,
    LiveMineChoice,
    LiveTriggerMines,
    LiveForceDiscard,
    LiveColorDiscard,
    LiveCardSwap,
    LiveResolvedCardSwap,
    LiveDeckCardSwap,
    LiveItemSwap,
    LiveDiscardOrDefeat,
    LiveDiscard,
    LiveRetrieve,
    LiveTokenPlace,
    LiveTokenRemove,
    LiveTokenBatch,
    LiveRockUltimate,
    LiveTurret {
        remove: bool,
    },
    LiveTokenPath {
        trail: bool,
    },
    LivePassive,
    LiveOfferPassive,
    LiveMarkPassive,
    LiveRuneChoice,
    LiveChooseCardColor,
    LiveDirectionalMove,
    LiveRevealHandCard,
    LiveLaugh,
    LiveCardGuess {
        reveal: bool,
    },
    LiveRuneInscription,
    LiveHexSelect,
    LiveUnitSelect,
    LiveMultiSelect,
    LiveDefeat,
    LiveCombatResolve,
    LiveAttackSequence,
    LiveReaction,
    LivePreMovement,
    LivePreDiscard,
    LiveAdjacentSnapshot,
    LiveTargetDefeated,
    LiveRemoveUnit,
    LiveGameOver,
    LiveProtection,
    LiveContextCondition,
    LiveSpell(crate::spells::SpellOp),
    LiveCheckUnitFilters,
    LiveDistanceCheck,
    LiveUnitTypeCheck,
    LiveUnitOnBoard,
    LiveBooleanCombine,
    LiveConvertCardItem,
    LiveAfterAttackText,
    LiveMinionRespawn,
    LiveCardCount,
    LiveAdjacentEnemies,
    LiveZoneChange,
    LiveSoloWin,
    LiveRoundDefeat,
    LiveQuery {
        collect: bool,
    },
    LiveMoveSequence,
    LiveForceDefenseMovement,
    LiveCardSelect,
    MinionProtection {
        spec: crate::protection::ProtectionBinding,
        remove: usize,
        discard: usize,
    },
    CreateEffect(Arc<crate::effect_steps::EffectStepSpec>),
    MineChoice {
        unit: Option<Symbol>,
        key: Option<Symbol>,
        destination: Symbol,
        range: i32,
        output: Symbol,
        prompt: Symbol,
    },
    TriggerMines {
        ids: Option<Vec<Symbol>>,
        victim: Option<Symbol>,
        ids_key: Symbol,
        victim_key: Symbol,
        rollback: Symbol,
        blast: Symbol,
        discard: Option<usize>,
    },
    Flow(crate::turn::FlowStep),
    Battle(crate::battle::BattleStep),
    ReturnMinions {
        processed: Vec<Symbol>,
        movement: usize,
        placement: usize,
    },
    Push {
        spec: crate::displacement::PushSpec,
        passive: usize,
    },
    ComputeHex {
        origin: Option<Symbol>,
        target: Symbol,
        scale: i64,
        output: Symbol,
    },
    Multi {
        output: Symbol,
        filters: Vec<UnitFilter>,
        tokens: bool,
        prompt: Symbol,
        min: i64,
        max: i64,
    },
    RemoveToken {
        literal: Option<Symbol>,
        key: Option<Symbol>,
    },
    ResolveCard {
        hero: Symbol,
        program: Arc<Vec<(&'static str, usize)>>,
    },
    CardText {
        hero: Symbol,
        program: Arc<PrimaryProgram>,
    },
    Place {
        unit: Option<Symbol>,
        key: Option<Symbol>,
        destination: Option<Hex>,
        destination_key: Symbol,
    },
    Swap {
        a: Option<Symbol>,
        b: Option<Symbol>,
        a_key: Option<Symbol>,
        b_key: Option<Symbol>,
    },
    CheckAdjacency {
        a: Option<Symbol>,
        b: Option<Symbol>,
        a_key: Option<Symbol>,
        b_key: Option<Symbol>,
        output: Symbol,
    },
    RecordHex {
        unit: Option<Symbol>,
        key: Option<Symbol>,
        output: Symbol,
    },
    Log,
    CardSelect(Arc<CardSelectSpec>),
    ForceDiscard {
        victim: Option<Symbol>,
        key: Option<Symbol>,
        basic: Option<bool>,
        immunity: Option<Symbol>,
        select: usize,
        discard: usize,
    },
    DiscardOrDefeat {
        key: Symbol,
        killer: Option<Symbol>,
        killer_key: Option<Symbol>,
        immunity: Option<Symbol>,
        select: usize,
        discard: usize,
        defeat: usize,
    },
    Attack {
        spec: AttackSpec,
        select: usize,
        children: Vec<usize>,
    },
    CheckDefeated {
        target: Symbol,
        output: Symbol,
        by_actor: bool,
    },
    Reaction {
        target: Symbol,
        discard: usize,
        passive: usize,
    },
    Discard {
        hero: Option<Symbol>,
        hero_key: Option<Symbol>,
        card: Option<Symbol>,
        card_key: Option<Symbol>,
        played: bool,
        passive: usize,
    },
    Passive {
        hero: Option<Symbol>,
        trigger: Symbol,
    },
    OfferPassive {
        spec: crate::passives::Offer,
        mark: usize,
    },
    MarkPassive {
        card: Symbol,
        provider: Option<Symbol>,
    },
    DefenseText {
        on_block: bool,
        program: DefenseProgram,
    },
    PlaceMarker {
        kind: MarkerKind,
        target: Option<Symbol>,
        key: Option<Symbol>,
        value: i64,
        passive: usize,
    },
    RemoveMarker(MarkerKind),
    RestoreAction,
    FinishedEffect,
    RestoreContext {
        other: bool,
    },
    PreMovement {
        hero: Option<Symbol>,
        key: Option<Symbol>,
        children: Vec<usize>,
    },
    PreDiscard {
        hero: Option<Symbol>,
        key: Option<Symbol>,
        abort: bool,
        force: usize,
        defeat: usize,
    },

    Combat {
        damage: i64,
        target: Symbol,
        defeat: usize,
    },
    Defeat {
        protection: Option<usize>,
        victim: Option<Symbol>,
        key: Option<Symbol>,
        killer: Option<Symbol>,
        assist: i64,
        remove: usize,
        over: usize,
    },
    Remove {
        unit: Option<Symbol>,
        key: Option<Symbol>,
    },
    GameOver(GameOutcome),
    Set {
        key: Symbol,
        value: Datum,
    },
    Add {
        key: Symbol,
        amount: i64,
    },
    SetActor {
        literal: Option<Symbol>,
        key: Option<Symbol>,
        save: Symbol,
    },
    Record {
        input: Symbol,
        output: Symbol,
    },
    FlipCoin,
    Retrieve {
        card: Symbol,
        hero_key: Option<Symbol>,
        all: bool,
    },
    RetrieveUnresolved {
        hero: Symbol,
        prompt: Symbol,
    },
    Move {
        mine_choice: Option<usize>,
        mine_trigger: Option<usize>,
        prechosen: bool,
        unit: Option<Symbol>,
        unit_key: Option<Symbol>,
        destination: Option<Hex>,
        destination_key: Symbol,
        range: i32,
        through: bool,
        straight: bool,
        crossed: Option<Symbol>,
        success: Option<Symbol>,
        is_action: bool,
    },
    Unit {
        output: Symbol,
        filters: Vec<UnitFilter>,
        tokens: bool,
        prompt: Symbol,
        auto: bool,
        owner: Option<Symbol>,
        owner_key: Option<Symbol>,
        discard: bool,
    },
    ChoosePiece {
        hero: Symbol,
        prompt: Symbol,
    },
    LiveSetPiece,
    LiveSpawnPiece,
    LiveRemovePiece,
    LiveMirrorPush,
    SetPiece {
        hero: Symbol,
        piece: Option<Symbol>,
        key: Option<Symbol>,
    },
    Number {
        output: Symbol,
        options: Vec<i64>,
        labels: Vec<(i64, Symbol)>,
        prompt: Symbol,
        auto: bool,
        owner: Option<Symbol>,
        owner_key: Option<Symbol>,
        discard: bool,
    },
    Hex {
        output: Symbol,
        filters: Vec<HexFilter>,
        prompt: Symbol,
        auto: bool,
        owner: Option<Symbol>,
        owner_key: Option<Symbol>,
        discard: bool,
    },
    FastTravel {
        unit: Option<Symbol>,
        destination: Option<Hex>,
        destination_key: Symbol,
        change: bool,
    },
    TravelSequence {
        unit: Option<Symbol>,
        destination: Symbol,
        select: usize,
        travel: usize,
    },
    MovementSequence {
        unit: Option<Symbol>,
        destination: Symbol,
        select: usize,
        movement: usize,
        range: i32,
        stat: Option<Stat>,
    },
    Confirm {
        output: Symbol,
        prompt: Symbol,
        owner: Option<Symbol>,
    },
    Query {
        spec: crate::queries::Query,
        collect: bool,
        output: Symbol,
    },
    ContextCondition {
        input: Symbol,
        comparison: crate::queries::Comparison,
        threshold: i64,
        output: Symbol,
    },
    Repeat {
        max: i64,
        prompt: Symbol,
        templates: Vec<usize>,
        check: usize,
    },
    ValidateRepeat {
        actor: Option<Symbol>,
        and_key: Option<Symbol>,
        output: Symbol,
    },
    ForEach {
        list: Symbol,
        item: Symbol,
        templates: Vec<usize>,
    },
}
#[derive(Clone, Debug)]
struct CompiledStep(Arc<StepProgram>);

#[derive(Clone, Debug)]
struct StepProgram {
    primary_template: bool,
    conditions: Conditions,
    instruction: Instruction,
    wire: Value,
    initial: Frame,
}
impl std::ops::Deref for CompiledStep {
    type Target = StepProgram;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for CompiledStep {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(&mut self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Team {
    Red,
    Blue,
}
#[derive(Clone, Debug)]
pub struct NativeSession {
    // These JSON definitions are touched only by compile/export, never advance.
    source: Arc<StateEnvelope>,
    steps: Arc<Vec<CompiledStep>>,
    symbols: Arc<Symbols>,
    context: Registers,
    count_observer: crate::data::FilterObservation,
    objects: Option<Box<object_execution::BoundObjects>>,
    world: NativeWorld,
    combat_rules: CombatRules,
    marker_rules: MarkerRules,
    attack_rules: AttackRules,
    // Live frontend metadata. Offline oracle traces compare the frozen rule state.
    track_attack_source: bool,
    action_rules: ActionRules,
    primary_operands: PrimaryOperands,
    return_rules: Arc<crate::minion_return::ReturnRules>,
    battle_rules: Option<Arc<crate::battle::BattleRules>>,
    battle_program: Option<Arc<BattleProgram>>,
    reaction_rules: Arc<ReactionRules>,
    passive_rules: Arc<crate::passives::PassiveRules>,
    passive_offer: Option<usize>,
    protection_program: Option<usize>,
    primary_blast_guard: bool,
    passive_bodies: Arc<Vec<PassiveBody>>,
    stack: Vec<Frame>,
    planning: crate::planning::Planning,
    boundary: crate::turn::Boundary,
    round: crate::round::RoundState,
    round_rules: Option<Arc<crate::round::RoundRules>>,
    turn_rules: Option<Arc<crate::turn::TurnRules>>,
    flow_program: Option<Arc<FlowProgram>>,
    team: Team,
    next_request: u64,
    next_step: u64,
    pub last_request: Option<NativeRequest>,
    sampled_guess_steps: Option<[crate::objects::ObjectId; 2]>,
    selection_key: Symbol,
    action_stack_key: Symbol,
    yes: Symbol,
    skip: Symbol,
    no_actor: Symbol,
    coordinates: [Symbol; 3],
}
impl NativeSession {
    /// Prepare process-wide rule data before starting a timed search request.
    pub fn prepare_search() -> Result<()> {
        crate::templates::instructions().map_err(unsupported)?;
        crate::primary::catalog().map_err(unsupported)?;
        object_execution::primary_operand_catalog()?;
        object_execution::shared_primary_programs()?;
        Ok(())
    }

    pub fn compile(source: StateEnvelope) -> Result<Self> {
        let (phase, mut expanded) = source.expand_for_restore().map_err(unsupported)?;
        if !matches!(
            phase,
            Phase::Planning | Phase::Resolution | Phase::Cleanup | Phase::LevelUp | Phase::GameOver
        ) {
            return Err(unsupported("phase flow not yet compiled"));
        }
        let primary = (!source.object_bindings.is_empty())
            .then(object_execution::shared_primary_programs)
            .transpose()?;
        let mut objects = object_execution::BoundObjects::compile_validated(
            &source,
            primary.map_or_else(Symbols::default, |programs| programs.symbols.clone()),
        )?;
        let raw = &source.state;
        let has_effects = raw["active_effects"]
            .as_array()
            .is_some_and(|v| !v.is_empty());
        let effect_steps_only = matches!(
            phase,
            Phase::Planning | Phase::Resolution | Phase::Cleanup | Phase::LevelUp
        ) && raw["execution_stack"].as_array().is_some_and(|stack| {
            stack.iter().all(|step| {
                matches!(
                    step["type"].as_str(),
                    Some(
                        "create_effect"
                            | "remove_token"
                            | "check_adjacency"
                            | "compute_distance"
                            | "place_unit"
                            | "swap_units"
                            | "set_actor"
                            | "validate_repeat"
                            | "move_unit"
                            | "move_sequence"
                            | "mine_path_choice"
                            | "trigger_mine"
                            | "force_discard"
                            | "discard_card"
                            | "check_passive_abilities"
                            | "offer_passive"
                            | "mark_passive_used"
                            | "defeat_unit"
                            | "resolve_combat"
                            | "remove_unit"
                            | "trigger_game_over"
                            | "force_discard_or_defeat"
                            | "check_minion_protection"
                            | "reaction_window"
                            | "resolve_defense_text"
                            | "resolve_on_block_effect"
                            | "restore_action_type"
                            | "place_marker"
                            | "remove_marker"
                            | "attack_sequence"
                            | "multi_select"
                            | "count"
                            | "collect_units"
                            | "check_context_condition"
                            | "check_target_defeated"
                            | "resolve_pre_action_movement"
                            | "resolve_pre_action_discard"
                            | "return_minion_to_zone"
                            | "minion_battle"
                            | "choose_minion_removal"
                            | "check_lane_push"
                            | "lane_push"
                            | "resolve_displacement"
                            | "end_phase_cleanup"
                            | "resolve_upgrades"
                            | "round_reset"
                            | "ask_confirmation"
                            | "set_context_flag"
                            | "log_message"
                            | "finished_expiring_effect"
                    )
                ) || (step["type"] == "select_step"
                    && matches!(
                        step["target_type"].as_str(),
                        Some("HEX" | "NUMBER" | "CARD" | "UNIT" | "UNIT_OR_TOKEN")
                    ))
            })
        });
        let inert_terminal = phase == Phase::GameOver
            && raw["execution_stack"]
                .as_array()
                .is_some_and(|stack| stack.is_empty());
        if objects.is_none() && has_effects && !effect_steps_only && !inert_terminal {
            return Err(unsupported(
                "active-effect gameplay/phase closure not yet compiled",
            ));
        }
        let mut symbols = objects
            .as_ref()
            .map_or_else(Symbols::default, |objects| (*objects.arena.symbols).clone());
        let selection_key = symbols.intern("selection");
        let action_stack_key = symbols.intern("action_context_stack");
        let yes = symbols.intern("YES");
        let skip = symbols.intern("SKIP");
        let no_actor = symbols.intern("None");
        symbols.intern("system");
        let coordinates = ["q", "r", "s"].map(|name| symbols.intern(name));
        let planning = if has_effects {
            crate::planning::Planning::compile_effect_boundary(&source, phase, &mut symbols)
        } else {
            crate::planning::Planning::compile_flow(&source, phase, &mut symbols)
        }
        .map_err(unsupported)?;
        let team = match raw["tie_breaker_team"].as_str() {
            Some("RED") => Team::Red,
            Some("BLUE") => Team::Blue,
            // An unset live value remains null in the authoritative arena.
            // Source priority checks use RED equality and otherwise favor BLUE;
            // flipping that unset coin also follows the BLUE -> RED branch.
            None if raw["tie_breaker_team"].is_null() && objects.is_some() => Team::Blue,
            _ => return Err(unsupported("invalid tie-breaker team")),
        };
        let combat_rules = CombatRules::compile(&mut symbols);
        let marker_rules = MarkerRules::compile(&mut symbols);
        let attack_rules = AttackRules::compile(&mut symbols);
        let action_rules = ActionRules::compile(&mut symbols);
        let primary_operands = PrimaryOperands::compile_with_live(
            &mut symbols,
            object_execution::primary_operand_catalog()?,
        )
        .map_err(unsupported)?;
        let reaction_rules = Arc::new(ReactionRules::compile(&mut symbols).map_err(unsupported)?);
        let mut context_items = Vec::new();
        for (key, value) in raw["execution_context"]
            .as_object()
            .ok_or_else(|| unsupported("invalid execution context"))?
        {
            if objects.is_none() {
                context_items.push((
                    symbols.intern(key),
                    Datum::from_json(value, &mut symbols).map_err(unsupported)?,
                ));
            }
        }
        // Primary programs occupy the same prefix in every arena-backed session.
        // Live continuations are appended and can patch templates via copy-on-write.
        let mut steps = primary.map_or_else(Vec::new, |programs| programs.steps.clone());
        let mut stack = Vec::new();
        for (index, wire) in raw["execution_stack"]
            .as_array()
            .ok_or_else(|| unsupported("invalid execution stack"))?
            .iter()
            .enumerate()
        {
            let id = if objects.is_some() {
                object_execution::BoundObjects::compile_wire(wire, &mut steps, &mut symbols)?
            } else {
                compile_step(wire, &mut steps, &mut symbols)?
            };
            let mut frame = steps[id].initial.clone();
            if let Some(objects) = &mut objects {
                let object = objects.step(index)?;
                objects.bind_program(object, id, &steps)?;
                frame.object = Some(object);
                frame.pending = None;
            }
            stack.push(frame);
        }
        if let (Some(objects), Some(primary)) = (&mut objects, primary) {
            objects.bind_stored_programs(&source, &mut steps, &mut symbols)?;
            objects.compile_scheduler(&mut steps, &mut symbols, primary)?;
        }
        if objects.is_none()
            && steps
                .iter()
                .any(|step| matches!(step.instruction, Instruction::RestoreContext { .. }))
        {
            return Err(unsupported(
                "action-context restoration requires live object bindings",
            ));
        }
        let may_upgrade_cards = steps.iter().any(|s| {
            matches!(
                s.instruction,
                Instruction::Flow(crate::turn::FlowStep::Cleanup | crate::turn::FlowStep::Upgrades)
            )
        });
        let has_flow = objects.is_none()
            && ((!has_effects && phase == Phase::Planning)
                || steps
                    .iter()
                    .any(|step| matches!(step.instruction, Instruction::Flow(_))));
        let flow_program = if has_flow {
            Some(Arc::new(compile_flow_program(
                &mut steps,
                &mut symbols,
                has_effects,
            )?))
        } else {
            None
        };
        let boundary =
            crate::turn::Boundary::compile(&source, &mut symbols).map_err(unsupported)?;
        let round =
            crate::round::RoundState::compile(&source, &mut symbols).map_err(unsupported)?;
        let round_rules = if has_flow || objects.is_some() {
            Some(Arc::new(
                crate::round::RoundRules::compile(&mut symbols).map_err(unsupported)?,
            ))
        } else {
            None
        };
        // The arena owns pending execution for shared-object sessions. The
        // world only needs the active effects without their finishing programs.
        if objects.is_some()
            && let Some(effects) = expanded["active_effects"].as_array_mut()
        {
            for effect in effects {
                effect["finishing_steps"] = json!([]);
            }
        }
        let world =
            NativeWorld::compile_expanded(&source, &expanded, &mut symbols).map_err(unsupported)?;
        drop(expanded);
        // Projectile selection is a dependency of a possible on-block body,
        // not of every defense window. Do not compile unreachable targeting.
        if steps.iter().any(|s| {
            matches!(
                s.instruction,
                Instruction::DefenseText { on_block: true, .. }
            )
        }) {
            let mut projectile = |effect: &str, deflect: bool| -> Result<Option<(usize, usize)>> {
                let possible = if has_effects && !may_upgrade_cards {
                    // Bounded roots without upgrades consume held cards;
                    // discard is retained for an imported on-block frame.
                    world.heroes.iter().any(|hero| {
                        hero.hand
                            .iter()
                            .chain(&hero.discard)
                            .chain(hero.played.iter().flatten())
                            .chain(hero.current.iter())
                            .any(|id| {
                                world.card(*id).is_some_and(|card| {
                                    symbols.name(world.definitions.cards[card.definition].effect)
                                        == effect
                                })
                            })
                    })
                } else {
                    // Cleanup/upgrade and full ordinary roots can introduce deck cards.
                    world
                        .definitions
                        .cards
                        .iter()
                        .any(|d| symbols.name(d.effect) == effect)
                };
                if possible {
                    Ok(Some(compile_projectile(deflect, &mut steps, &mut symbols)?))
                } else {
                    Ok(None)
                }
            };
            let reflect = projectile("reflect_projectiles", false)?;
            let deflect = projectile("deflect_projectiles", true)?;
            for step in steps.iter_mut().filter(|step| {
                matches!(
                    step.instruction,
                    Instruction::DefenseText { on_block: true, .. }
                )
            }) {
                if let Instruction::DefenseText {
                    on_block: true,
                    program,
                } = &mut step.instruction
                {
                    program.reflect_select = reflect.map(|p| p.0);
                    program.reflect_discard = reflect.map(|p| p.1);
                    program.deflect_select = deflect.map(|p| p.0);
                    program.deflect_discard = deflect.map(|p| p.1);
                }
            }
        }
        let mut passive_bodies = Vec::new();
        let wild = symbols.intern("wild_hunt");
        let gun = symbols.intern("big_sodding_gun");
        let before_action = symbols.intern("before_action");
        let after_push = symbols.intern("after_push");
        let needs_trigger = |trigger| {
            (has_flow && !has_effects)
                || steps.iter().any(|step| match &step.instruction {
                    Instruction::Passive { trigger: kind, .. } => *kind == trigger,
                    Instruction::OfferPassive { spec, .. } => spec.trigger == trigger,
                    _ => false,
                })
        };
        let needs_wild = needs_trigger(before_action);
        let needs_gun = needs_trigger(after_push);
        if needs_wild
            && (world.definitions.cards.iter().any(|d| d.effect == wild)
                || world
                    .effects
                    .instances
                    .iter()
                    .any(|e| e.granted_passive == Some(wild)))
        {
            let recipes = crate::templates::instructions().map_err(unsupported)?["passive_bodies"]
                ["wild_hunt"]
                .as_array()
                .ok_or_else(|| unsupported("frozen Wild Hunt recipe missing"))?
                .clone();
            for hero in &world.heroes {
                let mut templates = Vec::new();
                for recipe in &recipes {
                    let mut recipe = recipe.clone();
                    bind_passive_owner(&mut recipe, symbols.name(hero.id));
                    templates.push(compile_step(&recipe, &mut steps, &mut symbols)?);
                }
                passive_bodies.push(PassiveBody {
                    effect: wild,
                    owner: hero.id,
                    templates,
                });
            }
        }
        if needs_gun
            && (world.definitions.cards.iter().any(|d| d.effect == gun)
                || world
                    .effects
                    .instances
                    .iter()
                    .any(|e| e.granted_passive == Some(gun)))
        {
            let template = compile_step(
                &combat_template("discard_or_defeat")?,
                &mut steps,
                &mut symbols,
            )?;
            for hero in &world.heroes {
                passive_bodies.push(PassiveBody {
                    effect: gun,
                    owner: hero.id,
                    templates: vec![template],
                });
            }
        }
        let needs_protection = steps
            .iter()
            .any(|s| matches!(s.instruction, Instruction::MinionProtection { .. }))
            || steps
                .iter()
                .any(|s| matches!(s.instruction, Instruction::Defeat { .. }))
                && (world.effects.instances.iter().any(|e| {
                    e.definition.behavior.kind == crate::effect_rules::Kind::MinionProtection
                }) || steps
                    .iter()
                    .any(|s| matches!(s.instruction, Instruction::CreateEffect(_))));
        let protection_program = if needs_protection {
            let program = compile_step(
                &combat_template("minion_protection")?,
                &mut steps,
                &mut symbols,
            )?;
            for step in steps
                .iter_mut()
                .filter(|step| matches!(step.instruction, Instruction::Defeat { .. }))
            {
                if let Instruction::Defeat { protection, .. } = &mut step.instruction {
                    *protection = Some(program);
                }
            }
            Some(program)
        } else {
            None
        };
        let may_create_effects = steps.iter().any(|step| {
            !step.primary_template && matches!(step.instruction, Instruction::CreateEffect(_))
        });
        let may_deploy = steps
            .iter()
            .any(|s| matches!(s.instruction, Instruction::Place { .. }))
            && (world.definitions.entities.iter().any(|e| e.passable)
                || world.definitions.supply_passable.iter().any(|(_, p, _)| *p));
        let move_links = steps
            .iter()
            .enumerate()
            .filter_map(|(i, step)| {
                if let Instruction::Move {
                    destination_key,
                    range,
                    prechosen,
                    ..
                } = step.instruction
                {
                    (world.has_deployed_passable() || prechosen || may_deploy).then_some((
                        i,
                        destination_key,
                        range,
                        prechosen,
                    ))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for (i, destination, range, prechosen) in move_links {
            let mut choice = combat_template("mine_path_choice")?;
            choice["destination_key"] = json!(symbols.name(destination));
            choice["range_val"] = json!(range);
            let choice = compile_step(&choice, &mut steps, &mut symbols)?;
            let trigger =
                compile_step(&combat_template("trigger_mine")?, &mut steps, &mut symbols)?;
            // These potential IDs specialize only the discard linker. Each
            // runtime child replaces them with its actual literal queue.
            let mut possible_ids = world
                .definitions
                .entities
                .iter()
                .filter(|e| {
                    e.token_type.is_some() && (may_deploy || world.position(e.id).is_some())
                })
                .map(|e| e.id)
                .collect::<Vec<_>>();
            if prechosen
                && let Some(ids) = raw["execution_context"]["triggered_mine_ids"].as_array()
            {
                possible_ids.extend(
                    ids.iter()
                        .filter_map(Value::as_str)
                        .map(|id| symbols.intern(id)),
                );
            }
            if let Instruction::TriggerMines { ids, .. } = &mut steps[trigger].instruction {
                *ids = Some(possible_ids);
            }

            if let Instruction::Move {
                mine_choice,
                mine_trigger,
                ..
            } = &mut steps[i].instruction
            {
                *mine_choice = Some(choice);
                *mine_trigger = Some(trigger);
            }
        }
        let mine_links = steps
            .iter()
            .enumerate()
            .filter_map(|(index, step)| {
                if let Instruction::TriggerMines {
                    ids,
                    blast,
                    victim_key,
                    ..
                } = &step.instruction
                {
                    let possible = ids.as_ref().map_or_else(
                        || {
                            world
                                .definitions
                                .entities
                                .iter()
                                .any(|e| e.token_type == Some(*blast))
                        },
                        |ids| {
                            ids.iter().any(|id| {
                                world
                                    .entity(*id)
                                    .is_some_and(|e| e.token_type == Some(*blast))
                            })
                        },
                    );
                    Some((index, *victim_key, possible))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let primary_blast_guard = mine_links.iter().any(|(_, _, possible)| *possible);
        for (index, key, possible) in mine_links {
            if !possible {
                continue;
            }
            if may_create_effects {
                return Err(unsupported(
                    "blast mine dynamically-created effect closure not compiled",
                ));
            }
            let mut forced = combat_template("force_discard")?;
            forced["victim_key"] = json!(symbols.name(key));
            let child = compile_step(&forced, &mut steps, &mut symbols)?;
            if let Instruction::TriggerMines { discard, .. } = &mut steps[index].instruction {
                *discard = Some(child);
            }
        }
        let passive_rules = Arc::new(
            crate::passives::PassiveRules::compile(&world, &mut symbols).map_err(unsupported)?,
        );
        let passive_triggers = steps
            .iter()
            .filter_map(|s| {
                if let Instruction::Passive { trigger, .. } = s.instruction {
                    Some(trigger)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let needs_passive_offer = if has_effects {
            let mut needed = false;
            for hero in &world.heroes {
                for trigger in &passive_triggers {
                    let offers = passive_rules
                        .scan(&world, Some(hero.id), Some(hero.id), *trigger)
                        .map_err(unsupported)?;
                    for offer in &offers {
                        if let Some((_, effect)) = passive_rules.resolve(&world, hero.id, *offer)
                            && !passive_bodies
                                .iter()
                                .any(|body| body.effect == effect && body.owner == hero.id)
                        {
                            return Err(unsupported(
                                "eligible effect-backed passive body not compiled",
                            ));
                        }
                    }
                    needed |= !offers.is_empty();
                }
            }
            needed
        } else {
            !passive_triggers.is_empty()
        };
        let passive_offer = if needs_passive_offer {
            Some(compile_step(
                &combat_template("offer_passive")?,
                &mut steps,
                &mut symbols,
            )?)
        } else {
            None
        };
        let turn_rules = (has_flow || objects.is_some())
            .then(|| Arc::new(crate::turn::TurnRules::compile(&world, &mut symbols)));
        let has_battle = steps
            .iter()
            .any(|s| matches!(s.instruction, Instruction::Battle(_)));
        let battle_program = if has_battle {
            Some(Arc::new(compile_battle_program(&mut steps, &mut symbols)?))
        } else {
            None
        };
        let battle_rules =
            has_battle.then(|| Arc::new(crate::battle::BattleRules::compile(&world, &mut symbols)));
        let return_rules = Arc::new(crate::minion_return::ReturnRules::compile(
            &world,
            &mut symbols,
        ));
        for step in &steps {
            if matches!(
                step.instruction,
                Instruction::Defeat { .. } | Instruction::Combat { .. }
            ) {
                for hero in &world.heroes {
                    world
                        .lifecycle_hero(hero.id)
                        .map_err(|e| unsupported(e.to_string()))?;
                }
            }
            if matches!(
                step.instruction,
                Instruction::FastTravel { .. } | Instruction::TravelSequence { .. }
            ) {
                world.ensure_fast_travel_closure().map_err(unsupported)?;
            }
            if let Instruction::ChoosePiece { hero, .. } = step.instruction
                && world
                    .hero(hero)
                    .is_some_and(|h| world.entity(h.id).is_some_and(|e| e.multi_piece))
                && world.has_uncompiled_auras
            {
                return Err(unsupported(
                    "acting-piece action-prevention aura closure not compiled",
                ));
            }
            if let Instruction::RetrieveUnresolved { hero, .. } = step.instruction
                && world.hero_index(hero).is_some()
            {
                world
                    .lifecycle_hero(hero)
                    .map_err(|e| unsupported(e.to_string()))?;
            }
            if let Instruction::Move { is_action, .. } = step.instruction {
                world.ensure_move_closure(is_action).map_err(unsupported)?;
            }
        }
        let mut context = Registers::new(&symbols);
        for (key, value) in context_items {
            context.set(key, value);
        }
        if context
            .get(action_stack_key)
            .is_some_and(|v| v.truthy(&symbols))
        {
            return Err(unsupported(
                "nested action-context restoration not yet compiled",
            ));
        }
        let mut next_request = steps
            .iter()
            .filter_map(|step| match step.initial.request {
                Some(RequestToken::Imported(id)) => symbols
                    .name(id)
                    .strip_prefix("native-request-")
                    .and_then(|v| v.parse::<u64>().ok()),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        let mut next_step = steps
            .iter()
            .filter_map(|step| {
                step.wire["step_id"]
                    .as_str()
                    .and_then(|id| id.strip_prefix("native-step-"))
                    .and_then(|id| id.split('-').next())
                    .and_then(|id| id.parse::<u64>().ok())
            })
            .max()
            .unwrap_or(0);
        if let Some(resume) = source.native_resume {
            if resume.request_high_water < next_request || resume.step_high_water < next_step {
                return Err(unsupported(
                    "native resume counters precede live continuation identities",
                ));
            }
            next_request = resume.request_high_water;
            next_step = resume.step_high_water;
        }
        if next_request == u64::MAX {
            return Err(unsupported("request generation counter exhausted"));
        }
        if next_step == u64::MAX {
            return Err(unsupported("step generation counter exhausted"));
        }
        if objects.is_some() {
            // Live pending payloads belong only to arena step objects. Avoid
            // retaining/cloning a stale aggregate in immutable frame templates.
            for step in steps
                .iter_mut()
                .filter(|step| step.initial.pending.is_some())
            {
                step.initial.pending = None;
            }
        }
        symbols.freeze_compiled();
        let symbols = Arc::new(symbols);
        if let Some(objects) = &mut objects {
            objects.arena.symbols = symbols.clone();
        }
        Ok(Self {
            source: Arc::new(source),
            steps: Arc::new(steps),
            symbols,
            context,
            count_observer: crate::data::FilterObservation::default(),
            objects: objects.map(Box::new),
            world,
            combat_rules,
            marker_rules,
            attack_rules,
            track_attack_source: false,
            action_rules,
            primary_operands,
            return_rules,
            battle_rules,
            battle_program,
            reaction_rules,
            passive_rules,
            passive_offer,
            protection_program,
            primary_blast_guard,
            passive_bodies: Arc::new(passive_bodies),
            stack,
            planning,
            boundary,
            round,
            round_rules,
            turn_rules,
            flow_program,
            team,
            next_request,
            next_step,
            last_request: None,
            sampled_guess_steps: None,
            selection_key,
            action_stack_key,
            yes,
            skip,
            no_actor,
            coordinates,
        })
    }
    fn cleanup_count_origin(&mut self) {
        if self.count_observer.observed()
            && let Some(key) = self.symbols.lookup("_cmf_origin_hex")
            && matches!(self.context.get(key), Some(Datum::Null))
        {
            self.context.remove(key);
        }
        self.count_observer.clear();
    }
    fn multi_choices(
        &mut self,
        tokens: bool,
        filters: &[UnitFilter],
        frame: &Frame,
    ) -> Result<Vec<Symbol>> {
        let context = SpatialContext {
            count_observer: Some(&self.count_observer),
            world: &self.world,
            registers: &self.context,
            symbols: &self.symbols,
            actor: self.planning.actor,
            frame: frame.unit_override,
            coordinates: self.coordinates,
            range_override: None,
            through_override: None,
        };
        let choices = self
            .world
            .select_units(
                if tokens {
                    UnitCandidates::UnitsAndTokens
                } else {
                    UnitCandidates::Units
                },
                self.planning.actor,
                filters,
                &context,
            )
            .map_err(unsupported)?
            .into_iter()
            .filter(|id| !frame.multi.contains(id))
            .collect();
        self.cleanup_count_origin();
        Ok(choices)
    }
    fn normalized_owner(&self, actor: Symbol) -> Symbol {
        self.world.hero(actor).map_or(actor, |hero| hero.id)
    }
    fn symbol_from_context(&self, key: Symbol) -> Result<Option<Symbol>> {
        if let Some(objects) = &self.objects {
            return objects.symbol_from_context(key, &self.symbols);
        }
        match self.context.get(key) {
            Some(Datum::Text(id)) if !self.symbols.name(*id).is_empty() => Ok(Some(*id)),
            Some(value) if value.truthy(&self.symbols) => {
                Err(unsupported("non-string actor/controller context"))
            }
            _ => Ok(None),
        }
    }
    fn prompt_owner(&self, owner: Option<Symbol>, owner_key: Option<Symbol>) -> Result<Symbol> {
        let raw = prompt_actor(
            owner,
            owner_key,
            self.planning.actor,
            &self.symbols,
            |key| self.symbol_from_context(key),
        )?;
        Ok(self.normalized_owner(raw.unwrap_or(self.no_actor)))
    }
    fn flow_frame(&mut self, name: &str) -> Result<Frame> {
        let id = self
            .flow_program
            .as_ref()
            .and_then(|p| p.entries.iter().find(|(n, _)| *n == name).map(|(_, i)| *i))
            .ok_or_else(|| unsupported("flow program missing"))?;
        let mut frame = self.generated_frame(id, None, None)?;
        frame.unit_override = None;
        Ok(frame)
    }
    fn schedule_flow_names(&mut self, names: &[&str]) -> Result<()> {
        for name in names.iter().rev() {
            let frame = self.flow_frame(name)?;
            self.stack.push(frame);
        }
        Ok(())
    }
    fn schedule_continuations(&mut self) -> Result<()> {
        use crate::planning::Continuation as C;
        for cont in std::mem::take(&mut self.planning.continuation) {
            let name = match &cont {
                C::Retrieve(_) => "retrieve_unresolved",
                C::FindActor => "find_actor",
                C::Tie(_) => "tie",
                C::Respawn(_) => "respawn",
                C::Resolve(_) => "resolve_card",
                C::Confirm(_) => "confirm_resolution",
                C::Finalize(_) => "finalize_hero",
                C::FlipCoin => "flip_coin",
            };
            let mut frame = self.flow_frame(name)?;
            frame.continuation = Some(cont);
            self.stack.push(frame);
        }
        Ok(())
    }
    fn advance_turn(&mut self, clear_log: bool) -> Result<()> {
        if clear_log {
            self.world.history.clear_discards();
        }
        loop {
            if self.world.combat.turn < 4 {
                self.world.combat.turn += 1;
                self.boundary.snapshot(&self.world);
                self.planning.phase = Phase::Planning;
                if self
                    .planning
                    .auto_pass(&mut self.world)
                    .map_err(card_error)?
                {
                    self.world.history.clear_discards();
                    continue;
                }
            } else {
                self.planning.phase = Phase::Cleanup;
                self.schedule_flow_names(&["end_phase"])?;
            }
            return Ok(());
        }
    }
    fn abort_action(&mut self) {
        let boundary = |i: &Instruction| {
            matches!(
                i,
                Instruction::Flow(
                    crate::turn::FlowStep::Confirm(_) | crate::turn::FlowStep::Finalize(_)
                ) | Instruction::FinishedEffect
            )
        };
        let flag = |name: &str| {
            self.symbols
                .lookup(name)
                .and_then(|k| self.context.get(k))
                .is_some_and(|v| v.truthy(&self.symbols))
        };
        if flag("defense_card_id") && flag("is_primary_defense") {
            let mut target = None;
            for (i, f) in self.stack.iter().enumerate().rev() {
                let step = &self.steps[f.instruction].instruction;
                if boundary(step) {
                    break;
                }
                if matches!(step, Instruction::Combat { .. }) {
                    target = Some(i);
                    break;
                }
            }
            if let Some(mut i) = target {
                if let Some(f) = self.stack.get(i + 1)
                    && matches!(&self.steps[f.instruction].instruction,Instruction::SetActor{key:Some(k),..} if self.symbols.name(*k)=="_pre_defense_actor")
                {
                    i += 1;
                }
                self.stack.truncate(i + 1);
                return;
            }
        }
        if let Some(i) = self
            .stack
            .iter()
            .rposition(|f| boundary(&self.steps[f.instruction].instruction))
        {
            self.stack.truncate(i + 1);
        } else {
            self.stack.clear();
        }
    }
    fn schedule_battle(&mut self, ops: Vec<crate::battle::BattleOp>) -> Result<()> {
        use crate::battle::{BattleOp, BattleStep};
        let p = self
            .battle_program
            .as_ref()
            .ok_or_else(|| unsupported("battle program missing"))?
            .clone();
        for op in ops.into_iter().rev() {
            let next = match op {
                BattleOp::Step(step) => {
                    let id = match &step {
                        BattleStep::Push(_) => p.push,
                        BattleStep::Casualties(_) => p.casualties,
                        BattleStep::Displace(_) => p.displace,
                        _ => return Err(unsupported("unexpected generated battle step")),
                    };
                    let mut frame = self.generated_frame(id, None, None)?;
                    frame.battle = Some(step);
                    frame
                }
                BattleOp::Remove(id) => self.generated_frame(p.remove, Some(id), None)?,
                BattleOp::Place(unit, destination) => {
                    let mut frame = self.generated_frame(p.place, None, None)?;
                    frame.unit_override = None;
                    frame.zone_return = Some(crate::minion_return::ReturnBinding::Relocate {
                        unit,
                        destination,
                        range: None,
                    });
                    frame
                }
                BattleOp::GameOver(outcome) => {
                    let mut frame = self.generated_frame(p.victory, None, None)?;
                    frame.outcome_override = Some(outcome);
                    frame
                }
            };
            self.stack.push(next);
        }
        Ok(())
    }
    fn push_template_copies(
        &mut self,
        templates: &[usize],
        effect: Option<EffectBinding>,
    ) -> Result<()> {
        for &id in templates.iter().rev() {
            let mut child = if self.steps[id].wire["step_id"]
                .as_str()
                .is_some_and(|s| s.starts_with("native-step-"))
            {
                self.generated_frame(id, None, None)?
            } else {
                self.steps[id].initial.clone()
            };
            child.effect = effect.clone();
            if effect.as_ref().is_some_and(|e| e.primary.is_some()) {
                child.unit_override = None;
            }
            self.stack.push(child);
        }
        Ok(())
    }
    fn generated_frame(
        &mut self,
        id: usize,
        unit: Option<Symbol>,
        range: Option<i32>,
    ) -> Result<Frame> {
        self.next_step = self
            .next_step
            .checked_add(1)
            .ok_or_else(|| unsupported("step generation counter exhausted"))?;
        let mut frame = self.steps[id].initial.clone();
        frame.generated = Some(self.next_step);
        frame.unit_override = Some(unit);
        frame.range_override = range;
        Ok(frame)
    }
    fn issue(&mut self, frame: &mut Frame, mut request: NativeRequest) -> Result<NativeRequest> {
        let token = if let Some(token) = frame.request {
            token
        } else {
            self.next_request = self
                .next_request
                .checked_add(1)
                .ok_or_else(|| unsupported("request generation counter exhausted"))?;
            RequestToken::Generated(self.next_request)
        };
        frame.request = Some(token);
        request.id = token;
        if let Some(controller) =
            self.world
                .action_controller(request.owner, self.planning.actor, self.planning.owner)
        {
            request.controlled = Some(request.owner);
            request.owner = controller;
        }
        Ok(request)
    }
    pub fn advance(&mut self, budget: &ExecutionBudget<'_>) -> Result<AdvanceResult> {
        self.advance_inner(budget, None)
    }
    /// Offline diagnostic boundary, preserving events before the stop. This
    /// does not weaken the real execution budget used by advance/search.
    pub fn advance_prefix(&mut self, steps: usize) -> Result<AdvanceResult> {
        self.advance_inner(&ExecutionBudget::default(), Some(steps))
    }
    fn advance_inner(
        &mut self,
        budget: &ExecutionBudget<'_>,
        stop_after: Option<usize>,
    ) -> Result<AdvanceResult> {
        if self.objects.is_some() {
            return self.advance_objects(budget, stop_after, true);
        }
        let mut result = AdvanceResult::default();
        if !self.stack.is_empty() && self.planning.phase != Phase::GameOver {
            budget.check(0)?;
        }
        self.last_request = None;
        if self.planning.phase == Phase::GameOver {
            return Ok(result);
        }
        while !self.stack.is_empty() {
            if stop_after.is_some_and(|limit| result.steps >= limit) {
                return Ok(result);
            }
            budget.check(result.steps)?;
            self.count_observer.clear();
            result.steps += 1;
            let frame = self.stack.last().unwrap();
            let compiled = &self.steps[frame.instruction];
            let conditions = &compiled.conditions;
            if frame
                .attack_followup
                .or(conditions.active)
                .is_some_and(|key| {
                    !self
                        .context
                        .get(key)
                        .is_some_and(|v| v.truthy(&self.symbols))
                })
                || conditions.skip.is_some_and(|key| {
                    self.context
                        .get(key)
                        .is_some_and(|v| !matches!(v, Datum::Null))
                })
            {
                self.stack.pop();
                continue;
            }
            // A catalog body is not reachable merely because it was compiled.
            // Keep the unsupported dense mine/effect combination guarded when
            // an actual primary effect frame reaches execution, before popping.
            if self.primary_blast_guard
                && compiled.primary_template
                && matches!(compiled.instruction, Instruction::CreateEffect(_))
            {
                return Err(unsupported(
                    "blast mine dynamically-created effect closure not compiled",
                ));
            }
            if matches!(compiled.instruction, Instruction::LivePerformCard) {
                return Err(unsupported(
                    "copied-card action requires authoritative object bindings",
                ));
            }
            if matches!(compiled.instruction, Instruction::LiveSpell(_)) {
                return Err(unsupported(
                    "spell lifecycle requires authoritative object bindings",
                ));
            }
            if matches!(compiled.instruction, Instruction::LiveResource) {
                return Err(unsupported(
                    "resource step requires authoritative object bindings",
                ));
            }
            if matches!(
                compiled.instruction,
                Instruction::LiveDrag
                    | Instruction::LiveMoveToward
                    | Instruction::LiveSetInitiative
                    | Instruction::LiveActionControl
                    | Instruction::LiveJourney
                    | Instruction::LiveAdjacentEnemies
                    | Instruction::LiveZoneChange
                    | Instruction::LiveSoloWin
                    | Instruction::LiveCardCount
                    | Instruction::LiveRoundDefeat
                    | Instruction::LiveCheckUnitFilters
                    | Instruction::LiveDistanceCheck
                    | Instruction::LiveUnitTypeCheck
                    | Instruction::LiveUnitOnBoard
                    | Instruction::LiveBooleanCombine
                    | Instruction::LiveConvertCardItem
                    | Instruction::LiveAfterAttackText
                    | Instruction::LiveMinionRespawn
            ) {
                return Err(unsupported(
                    "card and round queries require authoritative object bindings",
                ));
            }
            if matches!(compiled.instruction, Instruction::LiveTokenPlace) {
                return Err(unsupported(
                    "token placement requires authoritative object bindings",
                ));
            }
            let mut frame = self.stack.pop().unwrap();
            // Instructions are immutable and shared by cloned sessions. A clone
            // here contains only this small primitive payload, never a game state.
            let mut instruction = compiled.instruction.clone();
            if let Some(action) = &frame.action {
                bind_action(&mut instruction, action, &self.symbols)?;
            }
            if let Some(values) = frame.effect.as_ref().and_then(|e| e.primary.as_ref()) {
                bind_primary(&mut instruction, values, &self.primary_operands)?;
            }
            if let Some((snapshot, key, source)) = frame.mine_trigger {
                if let Instruction::ForceDiscard {
                    victim,
                    key: victim_key,
                    immunity,
                    ..
                } = &mut instruction
                {
                    *victim = snapshot;
                    *victim_key = Some(key);
                    *immunity = source;
                } else {
                    return Err(unsupported("invalid mine discard snapshot"));
                }
            }
            if let Some(binding) = &frame.protection {
                if let Instruction::MinionProtection { spec, .. } = &mut instruction {
                    *spec = binding.clone();
                } else {
                    return Err(unsupported("invalid protection continuation binding"));
                }
            }
            if let Some((ids, target)) = &frame.mine_snapshot {
                if let Instruction::TriggerMines {
                    ids: queue, victim, ..
                } = &mut instruction
                {
                    *queue = Some(ids.clone());
                    *victim = Some(*target);
                } else {
                    return Err(unsupported("invalid movement mine snapshot"));
                }
            }
            if let Some(binding) = &frame.zone_return {
                bind_zone_return(&mut instruction, binding)?;
            }
            if let Some(battle) = &frame.battle {
                instruction = Instruction::Battle(battle.clone());
            }
            if let Some(cont) = &frame.continuation {
                bind_continuation(&mut instruction, cont)?;
            }
            if let Some((hero, range)) = frame.pre_move {
                bind_pre_movement(&mut instruction, hero, range)?;
            }
            let mandatory = conditions.mandatory;
            let selected = frame
                .pending
                .as_ref()
                .filter(|v| v.truthy(&self.symbols))
                .and_then(|v| {
                    if let Datum::Object(fields) = v {
                        fields
                            .iter()
                            .find(|(key, _)| *key == self.selection_key)
                            .map(|(_, v)| v.clone())
                    } else {
                        None
                    }
                });
            if self.world.has_effects
                && !matches!(
                    instruction,
                    Instruction::CreateEffect(_)
                        | Instruction::Place { .. }
                        | Instruction::Swap { .. }
                        | Instruction::SetActor { .. }
                        | Instruction::ValidateRepeat { .. }
                        | Instruction::Move { .. }
                        | Instruction::MovementSequence { .. }
                        | Instruction::MineChoice { .. }
                        | Instruction::TriggerMines { .. }
                        | Instruction::ForceDiscard { .. }
                        | Instruction::Discard { .. }
                        | Instruction::CardSelect(_)
                        | Instruction::Passive { .. }
                        | Instruction::OfferPassive { .. }
                        | Instruction::MarkPassive { .. }
                        | Instruction::Unit { .. }
                        | Instruction::Multi { .. }
                        | Instruction::Attack { .. }
                        | Instruction::CheckDefeated { .. }
                        | Instruction::Query { .. }
                        | Instruction::ContextCondition { .. }
                        | Instruction::PreMovement { .. }
                        | Instruction::PreDiscard { .. }
                        | Instruction::ReturnMinions { .. }
                        | Instruction::Battle(_)
                        | Instruction::Flow(
                            crate::turn::FlowStep::Cleanup
                                | crate::turn::FlowStep::Upgrades
                                | crate::turn::FlowStep::Reset
                        )
                        | Instruction::Reaction { .. }
                        | Instruction::DefenseText { .. }
                        | Instruction::RestoreAction
                        | Instruction::PlaceMarker { .. }
                        | Instruction::RemoveMarker(_)
                        | Instruction::MinionProtection { .. }
                        | Instruction::Defeat { .. }
                        | Instruction::Combat { .. }
                        | Instruction::Remove { .. }
                        | Instruction::GameOver(_)
                        | Instruction::DiscardOrDefeat { .. }
                        | Instruction::Hex { .. }
                        | Instruction::Number { .. }
                        | Instruction::Confirm { .. }
                        | Instruction::CheckAdjacency { .. }
                        | Instruction::RemoveToken { .. }
                        | Instruction::Set { .. }
                        | Instruction::FinishedEffect
                        | Instruction::Log
                )
            {
                return Err(unsupported(
                    "active-effect gameplay consumers not yet compiled",
                ));
            }
            let mut request = None;
            match instruction {
                Instruction::MinionProtection {
                    spec,
                    remove,
                    discard,
                } => {
                    use crate::protection::ProtectionDecision;
                    let mut retry = None;
                    match self.world.protection_decision(&spec).map_err(unsupported)? {
                        ProtectionDecision::Missing => continue,
                        ProtectionDecision::Next(effect) => retry = Some(effect),
                        ProtectionDecision::Defeat => {
                            let events = self
                                .combat_rules
                                .protection_defeat(
                                    &mut self.world,
                                    &mut self.context,
                                    spec.minion,
                                    spec.killer,
                                    true,
                                )
                                .map_err(combat_error)?;
                            result
                                .events
                                .extend(events.into_iter().map(NativeEvent::Combat));
                            let child = self.generated_frame(remove, Some(spec.minion), None)?;
                            self.stack.push(child);
                        }
                        ProtectionDecision::Sacrifice {
                            effect,
                            hero,
                            token,
                        } => {
                            let from = self
                                .world
                                .remove_entity(token)
                                .map_err(EngineError::Rule)?
                                .ok_or_else(|| unsupported("protection token vanished"))?;
                            let removed = self.world.remove_token_effects(token);
                            result.events.push(NativeEvent::TokenRemoved {
                                actor: Some(hero),
                                token,
                                from,
                                effects_removed: removed,
                            });
                            result
                                .events
                                .push(NativeEvent::Combat(CombatEvent::Protected {
                                    hero,
                                    minion: spec.minion,
                                    effect,
                                    card: None,
                                    token: Some(token),
                                }));
                        }
                        ProtectionDecision::Cards {
                            effect,
                            hero,
                            cards,
                        } => {
                            if let Some(pending) = &frame.pending {
                                let selection = protection_selection(
                                    pending,
                                    &self.symbols,
                                    self.selection_key,
                                );
                                let card = selection.and_then(|v| {
                                    if let Datum::Text(id) = v {
                                        cards.iter().copied().find(|card| {
                                            self.world.definitions.cards
                                                [self.world.card(*card).unwrap().definition]
                                                .printed_id
                                                == id
                                        })
                                    } else {
                                        None
                                    }
                                });
                                if let Some(card) = card {
                                    let card = self.world.definitions.cards
                                        [self.world.card(card).unwrap().definition]
                                        .printed_id;
                                    let events = self
                                        .combat_rules
                                        .protection_defeat(
                                            &mut self.world,
                                            &mut self.context,
                                            spec.minion,
                                            spec.killer,
                                            false,
                                        )
                                        .map_err(combat_error)?;
                                    result
                                        .events
                                        .extend(events.into_iter().map(NativeEvent::Combat));
                                    result.events.push(NativeEvent::Combat(
                                        CombatEvent::Protected {
                                            hero,
                                            minion: spec.minion,
                                            effect,
                                            card: Some(card),
                                            token: None,
                                        },
                                    ));
                                    let mut child = self.generated_frame(discard, None, None)?;
                                    child.discard_override = Some((hero, card));
                                    self.stack.push(child);
                                } else {
                                    retry = Some(effect);
                                }
                            } else {
                                request = Some(NativeRequest {
                                    controlled: None,
                                    id: RequestToken::Generated(0),
                                    owner: hero,
                                    prompt: spec.minion,
                                    movement_range: None,
                                    pre_action_range: None,
                                    choices: RequestChoices::Protection(cards),
                                    can_skip: true,
                                    discard: true,
                                });
                            }
                        }
                    }
                    if let Some(effect) = retry {
                        let mut tried = spec.tried.to_vec();
                        tried.push(effect);
                        let mut child = self.generated_frame(
                            self.protection_program
                                .ok_or_else(|| unsupported("protection retry program missing"))?,
                            None,
                            None,
                        )?;
                        child.protection = Some(crate::protection::ProtectionBinding {
                            minion: spec.minion,
                            killer: spec.killer,
                            tried: Arc::from(tried),
                        });
                        self.stack.push(child);
                    }
                }
                Instruction::MineChoice {
                    unit,
                    key,
                    destination,
                    range,
                    output,
                    prompt,
                } => {
                    let range = frame.range_override.unwrap_or(range);
                    let unit = frame.unit_override.unwrap_or(unit);
                    let empty = Datum::List(Vec::new());
                    let Some(value) = self
                        .context
                        .get(destination)
                        .filter(|v| v.truthy(&self.symbols))
                    else {
                        self.context.set(output, empty);
                        continue;
                    };
                    if !matches!(value, Datum::Object(_)) {
                        self.context.set(output, empty);
                        continue;
                    }
                    let destination = datum_hex(value, &self.symbols, self.coordinates)?;
                    let unit = unit
                        .or(if let Some(key) = key {
                            self.symbol_from_context(key)?
                        } else {
                            None
                        })
                        .or(self.planning.actor);
                    let Some(unit) = unit else {
                        self.context.set(output, empty);
                        continue;
                    };
                    let team = self
                        .world
                        .entity(unit)
                        .filter(|e| matches!(e.kind, EntityKind::Hero | EntityKind::Piece))
                        .and_then(|e| e.team);
                    let Some((start, team)) = self.world.position(unit).zip(team) else {
                        self.context.set(output, empty);
                        continue;
                    };
                    let paths = self
                        .world
                        .mine_paths(start, range, self.planning.actor, Some(team), &[unit])
                        .map_err(unsupported)?;
                    let options = self
                        .world
                        .tile_index(destination)
                        .map(|i| paths[i].clone())
                        .unwrap_or_default();
                    let selected_index = match selected.as_ref() {
                        Some(Datum::Text(id)) => options
                            .iter()
                            .enumerate()
                            .find(|(i, _)| i.to_string() == self.symbols.name(*id))
                            .map(|(i, _)| i),
                        _ => None,
                    };
                    let choice = if options.len() <= 1 {
                        Some(0)
                    } else {
                        selected_index
                    };
                    if let Some(index) = choice {
                        self.context.set(
                            output,
                            Datum::List(
                                options
                                    .get(index)
                                    .map(|p| p.mines.iter().copied().map(Datum::Text).collect())
                                    .unwrap_or_default(),
                            ),
                        );
                        continue;
                    }
                    frame.pending = None;
                    request = Some(NativeRequest {
                        controlled: None,
                        id: RequestToken::Generated(0),
                        owner: self
                            .planning
                            .actor
                            .filter(|id| *id != self.world.definitions.effect_empty)
                            .unwrap_or(self.world.definitions.effect_empty),
                        prompt,
                        movement_range: None,
                        pre_action_range: None,
                        choices: RequestChoices::Mines(options),
                        can_skip: false,
                        discard: false,
                    });
                }
                Instruction::TriggerMines {
                    ids,
                    victim,
                    ids_key,
                    victim_key,
                    rollback,
                    blast,
                    discard,
                } => {
                    let ids = if let Some(ids) = ids {
                        ids
                    } else {
                        match self.context.get(ids_key) {
                            None | Some(Datum::Null) => Vec::new(),
                            Some(Datum::List(values)) => values
                                .iter()
                                .map(|v| {
                                    if let Datum::Text(id) = v {
                                        Ok(*id)
                                    } else {
                                        Err(unsupported("non-string mine identity"))
                                    }
                                })
                                .collect::<Result<Vec<_>>>()?,
                            Some(v) if !v.truthy(&self.symbols) => Vec::new(),
                            _ => return Err(unsupported("mine queue must be a native list")),
                        }
                    };
                    if ids.is_empty() {
                        continue;
                    }
                    if discard.is_none()
                        && ids.iter().any(|id| {
                            self.world
                                .entity(*id)
                                .is_some_and(|e| e.token_type == Some(blast))
                        })
                    {
                        return Err(unsupported(
                            "mine type changed; blast recipe requires recompilation",
                        ));
                    }

                    let victim = if let Some(victim) =
                        victim.filter(|id| *id != self.world.definitions.effect_empty)
                    {
                        Some(victim)
                    } else {
                        self.symbol_from_context(victim_key)?
                    };
                    let mut sources = Vec::new();
                    for id in ids {
                        let entity = self.world.entity(id);
                        let exists = entity.is_some();
                        let kind = entity.and_then(|e| e.token_type);
                        let is_blast = kind == Some(blast);
                        let owner = entity
                            .and_then(|e| e.owner)
                            .filter(|id| *id != self.world.definitions.effect_empty);
                        let from = self.world.remove_entity(id).map_err(EngineError::Rule)?;
                        if from.is_some() {
                            self.world.remove_token_effects(id);
                        }
                        if is_blast {
                            sources.push(owner);
                        }
                        if let Some(from) = from.filter(|_| exists) {
                            result.events.push(NativeEvent::MineTriggered {
                                actor: self
                                    .planning
                                    .actor
                                    .filter(|id| *id != self.world.definitions.effect_empty),
                                target: id,
                                from,
                                kind,
                                blast: is_blast,
                            });
                        }
                    }
                    self.context.set(ids_key, Datum::List(Vec::new()));
                    self.context.set(rollback, Datum::Bool(true));
                    for source in sources.into_iter().rev() {
                        let mut child = self.generated_frame(
                            discard.ok_or_else(|| unsupported("missing blast recipe"))?,
                            None,
                            None,
                        )?;
                        child.mine_trigger = Some((victim, victim_key, source));
                        self.stack.push(child);
                    }
                }
                Instruction::CreateEffect(spec) => {
                    let context = crate::effects::CreationContext {
                        round: self.round.round,
                        turn: self.world.combat.turn,
                        actor: self.planning.actor,
                        registers: &self.context,
                    };
                    if let Some(event) = spec
                        .execute(&mut self.world, &context, Arc::make_mut(&mut self.symbols))
                        .map_err(unsupported)?
                    {
                        result.events.push(NativeEvent::EffectCreated(event));
                    }
                }
                Instruction::Flow(step) => {
                    use crate::turn::FlowStep;
                    let rules = self
                        .turn_rules
                        .as_ref()
                        .ok_or_else(|| unsupported("turn rules missing"))?
                        .clone();
                    match step {
                        FlowStep::Next => {
                            if self
                                .planning
                                .next_actor(&mut self.world)
                                .map_err(card_error)?
                            {
                                self.advance_turn(true)?;
                            }
                            self.schedule_continuations()?;
                        }
                        FlowStep::Finalize(hero) => {
                            if let Some(index) = self.world.hero_index(hero) {
                                let printed = self.world.heroes[index]
                                    .current
                                    .and_then(|id| self.world.card(id))
                                    .map(|card| {
                                        self.world.definitions.cards[card.definition].printed_id
                                    });
                                self.world
                                    .resolve_current_card(hero, Some(self.world.combat.turn))
                                    .map_err(card_error)?;
                                if let Some(printed) = printed {
                                    self.world.effects.activate_card(printed, true);
                                }
                                let cards = self.world.heroes[index]
                                    .played
                                    .iter()
                                    .flatten()
                                    .copied()
                                    .chain(self.world.heroes[index].ultimate)
                                    .collect::<Vec<_>>();
                                for id in cards {
                                    if let Some(card) = self.world.card_mut(id)
                                        && card.passive_uses > 0
                                    {
                                        card.passive_uses = 0;
                                    }
                                }
                                let owner = self.world.heroes[index].id;
                                for effect in &mut self.world.effects.instances {
                                    if effect.source == owner && effect.passive_uses > 0 {
                                        effect.passive_uses = 0;
                                    }
                                }
                            }
                            self.boundary.finalize(hero, &self.symbols);
                            self.context.clear();
                            self.world.acting_piece = None;
                            self.planning.actor = None;
                            self.planning.owner = None;
                            self.schedule_flow_names(&[
                                "return_minions",
                                "check_lane_push",
                                "find_actor",
                            ])?;
                            result.events.push(NativeEvent::TurnEnded(hero));
                        }
                        FlowStep::Confirm(hero) => {
                            if [rules.frozen, rules.reanchor].iter().any(|k| {
                                self.context
                                    .get(*k)
                                    .is_some_and(|v| v.truthy(&self.symbols))
                            }) {
                                continue;
                            }
                            if let Some(pending) = frame
                                .pending
                                .as_ref()
                                .filter(|value| value.truthy(&self.symbols))
                            {
                                if !matches!(pending, Datum::Object(_)) {
                                    return Err(unsupported(
                                        "confirmation input must be a mapping",
                                    ));
                                }
                                frame.pending = None;
                                if selected == Some(Datum::Text(rules.confirm)) {
                                    continue;
                                }
                            }
                            request = Some(NativeRequest {
                                controlled: None,
                                id: RequestToken::Generated(0),
                                owner: hero,
                                prompt: rules.confirm_prompt,
                                movement_range: None,
                                pre_action_range: None,
                                choices: RequestChoices::ConfirmAction,
                                can_skip: false,
                                discard: false,
                            });
                        }
                        FlowStep::Tie(tied) => {
                            let favored = rules.teams[usize::from(self.team == Team::Blue)];
                            let choice = self
                                .planning
                                .resolve_tie(&mut self.world, &tied, favored, selected.as_ref())
                                .map_err(card_error)?;
                            if let Some(choice) = choice {
                                frame.pending = None;
                                let index = usize::from(choice.team == rules.teams[1]);
                                request = Some(NativeRequest {
                                    controlled: None,
                                    id: RequestToken::Generated(0),
                                    owner: rules.owners[index],
                                    prompt: rules.confirm_prompt,
                                    movement_range: None,
                                    pre_action_range: None,
                                    choices: RequestChoices::Actors {
                                        team: choice.team,
                                        values: choice.candidates,
                                    },
                                    can_skip: false,
                                    discard: false,
                                });
                            } else {
                                self.schedule_continuations()?;
                            }
                        }
                        FlowStep::Respawn(hero) => {
                            let Some(h) = self.world.hero(hero) else {
                                continue;
                            };
                            if self.world.has_board_presence(hero) {
                                continue;
                            }
                            if selected.as_ref().is_some_and(
                                |v| matches!(v,Datum::Text(id) if self.symbols.name(*id)=="PASS"),
                            ) {
                                self.context.set(rules.skipped, Datum::Bool(true));
                                continue;
                            }
                            let multi = self.world.entity(h.id).is_some_and(|e| e.multi_piece);
                            let candidates = rules
                                .respawn_candidates(&self.world, hero)
                                .map_err(unsupported)?;
                            if candidates.is_empty() {
                                continue;
                            }
                            if let Some(to) = selected
                                .as_ref()
                                .and_then(|v| v.hex(self.coordinates, &self.symbols).ok())
                                .filter(|h| candidates.contains(h))
                            {
                                if let Some(token) = crate::turn::TurnRules::token(&self.world, to)
                                {
                                    let from = self
                                        .world
                                        .remove_entity(token)
                                        .map_err(EngineError::Rule)?
                                        .unwrap();
                                    result.events.push(NativeEvent::RespawnCleared {
                                        hero,
                                        token,
                                        from,
                                    });
                                }
                                let unit = if multi {
                                    self.world
                                        .hero(hero)
                                        .and_then(|h| {
                                            h.pieces
                                                .iter()
                                                .copied()
                                                .find(|id| self.world.position(*id).is_none())
                                        })
                                        .ok_or_else(|| {
                                            EngineError::Rule("empty hero piece supply".into())
                                        })?
                                } else {
                                    hero
                                };
                                self.world
                                    .place_entity(unit, to)
                                    .map_err(EngineError::Rule)?;
                                continue;
                            }
                            let (_, choice, hex) = rules
                                .prompts
                                .iter()
                                .find(|(id, _, _)| *id == hero)
                                .ok_or_else(|| unsupported("respawn prompt missing"))?;
                            let location=selected.as_ref().is_some_and(|v|(matches!(v,Datum::Object(_))&&v.truthy(&self.symbols))||matches!(v,Datum::Text(id) if self.symbols.name(*id)=="RESPAWN"));
                            request = Some(NativeRequest {
                                controlled: None,
                                id: RequestToken::Generated(0),
                                owner: hero,
                                prompt: if location { *hex } else { *choice },
                                movement_range: None,
                                pre_action_range: None,
                                choices: RequestChoices::Respawn {
                                    hexes: candidates,
                                    location,
                                },
                                can_skip: false,
                                discard: false,
                            });
                        }
                        FlowStep::Advance => {
                            self.advance_turn(false)?;
                            self.schedule_continuations()?;
                        }
                        FlowStep::EndPhase => self.schedule_flow_names(&[
                            "minion_battle",
                            "check_lane_push",
                            "end_cleanup",
                        ])?,
                        FlowStep::Cleanup => {
                            let rules = self
                                .round_rules
                                .as_ref()
                                .ok_or_else(|| unsupported("round rules missing"))?
                                .clone();
                            let (leveled, events) = rules
                                .cleanup(&mut self.world, &mut self.round)
                                .map_err(card_error)?;
                            result
                                .events
                                .extend(events.into_iter().map(NativeEvent::Round));
                            if leveled {
                                self.planning.phase = Phase::LevelUp;
                            }
                            if !self.round.pending.is_empty() {
                                self.schedule_flow_names(&["upgrades"])?;
                            } else {
                                self.round
                                    .reset(&mut self.world, false)
                                    .map_err(card_error)?;
                                self.boundary.snapshot(&self.world);
                                self.planning.phase = Phase::Planning;
                            }
                        }
                        FlowStep::Upgrades => {
                            let rules = self
                                .round_rules
                                .as_ref()
                                .ok_or_else(|| unsupported("round rules missing"))?
                                .clone();
                            if let Some(value) = &selected
                                && let Some(choice) = rules
                                    .validate(&self.world, &self.round, value)
                                    .map_err(card_error)?
                            {
                                for card in choice.cards {
                                    rules
                                        .apply(&mut self.world, &mut self.round, choice.hero, card)
                                        .map_err(card_error)?;
                                }
                            }
                            frame.pending = None;
                            if self.round.pending.is_empty() {
                                self.round.reveal.clear();
                                self.schedule_flow_names(&["round_reset"])?;
                                continue;
                            }
                            request = Some(NativeRequest {
                                controlled: None,
                                id: RequestToken::Generated(0),
                                owner: rules.simultaneous,
                                prompt: rules.prompt,
                                movement_range: None,
                                pre_action_range: None,
                                choices: RequestChoices::Upgrades(
                                    rules.players(&self.world, &self.round),
                                ),
                                can_skip: false,
                                discard: false,
                            });
                        }
                        FlowStep::Reset => {
                            self.round
                                .reset(&mut self.world, true)
                                .map_err(card_error)?;
                            self.boundary.snapshot(&self.world);
                            self.planning.phase = Phase::Planning;
                        }
                    }
                }
                Instruction::Battle(step) => {
                    let rules = self
                        .battle_rules
                        .as_ref()
                        .ok_or_else(|| unsupported("battle rules missing"))?
                        .clone();
                    let favored = self
                        .symbols
                        .lookup(if self.team == Team::Red {
                            "RED"
                        } else {
                            "BLUE"
                        })
                        .ok_or_else(|| unsupported("battle team missing"))?;
                    let resolution = rules
                        .resolve(
                            &mut self.world,
                            &self.symbols,
                            step,
                            selected.as_ref(),
                            favored,
                        )
                        .map_err(|e| match e {
                            crate::battle::BattleError::Rule(s) => EngineError::Rule(s),
                            crate::battle::BattleError::Unsupported(s) => unsupported(s),
                        })?;
                    match resolution {
                        crate::battle::Resolution::Done(ops) => self.schedule_battle(ops)?,
                        crate::battle::Resolution::Prompt(p) => {
                            if p.clear_pending {
                                frame.pending = None;
                            }
                            request = Some(NativeRequest {
                                controlled: None,
                                id: RequestToken::Generated(0),
                                owner: p.owner,
                                prompt: p.prompt,
                                movement_range: None,
                                pre_action_range: None,
                                choices: match p.choices {
                                    crate::battle::Choices::Units(values) => {
                                        RequestChoices::Units {
                                            tokens: false,
                                            values: values
                                                .into_iter()
                                                .map(|id| (id, None))
                                                .collect(),
                                        }
                                    }
                                    crate::battle::Choices::Hexes(values) => {
                                        RequestChoices::ZoneHexes {
                                            values,
                                            unit: p.unit,
                                        }
                                    }
                                },
                                can_skip: false,
                                discard: false,
                            });
                        }
                    }
                }
                Instruction::ReturnMinions {
                    processed,
                    movement,
                    placement,
                } => {
                    let favored = self
                        .symbols
                        .lookup(if self.team == Team::Red {
                            "RED"
                        } else {
                            "BLUE"
                        })
                        .ok_or_else(|| unsupported("minion return team missing"))?;
                    let Some(plan) = self
                        .return_rules
                        .next(
                            &self.world,
                            &self.symbols,
                            &processed,
                            favored,
                            self.planning.actor,
                            &self.context,
                        )
                        .map_err(unsupported)?
                    else {
                        continue;
                    };
                    let destination = selected
                        .as_ref()
                        .and_then(|v| v.hex(self.coordinates, &self.symbols).ok())
                        .filter(|h| plan.candidates.contains(h))
                        .or_else(|| (plan.candidates.len() == 1).then(|| plan.candidates[0]));
                    if plan.candidates.is_empty() || destination.is_some() {
                        if plan.more {
                            let mut next = self.generated_frame(frame.instruction, None, None)?;
                            let mut processed = processed;
                            processed.push(plan.unit);
                            next.zone_return =
                                Some(crate::minion_return::ReturnBinding::Processed(processed));
                            self.stack.push(next);
                        }
                        if let Some(destination) = destination {
                            let mut next = self.generated_frame(
                                if plan.placement { placement } else { movement },
                                None,
                                None,
                            )?;
                            next.unit_override = None;
                            next.zone_return =
                                Some(crate::minion_return::ReturnBinding::Relocate {
                                    unit: plan.unit,
                                    destination,
                                    range: (!plan.placement).then_some(plan.distance),
                                });
                            self.stack.push(next);
                        }
                        continue;
                    }
                    frame.pending = None;
                    request = Some(NativeRequest {
                        controlled: None,
                        id: RequestToken::Generated(0),
                        owner: plan.owner,
                        prompt: plan.prompt,
                        movement_range: None,
                        pre_action_range: None,
                        choices: RequestChoices::Hexes(plan.candidates),
                        can_skip: false,
                        discard: false,
                    });
                }
                Instruction::ComputeHex {
                    origin,
                    target,
                    scale,
                    output,
                } => {
                    let actor = if let Some(key) = origin {
                        self.symbol_from_context(key)?
                    } else {
                        self.planning.actor
                    };
                    let target = self.symbol_from_context(target)?;
                    if let Some((origin, target)) = actor
                        .and_then(|id| self.world.position(id))
                        .zip(target.and_then(|id| self.world.position(id)))
                        && let Some(result) = crate::displacement::behind(origin, target, scale)
                            .map_err(EngineError::Rule)?
                    {
                        self.context
                            .set(output, native_hex(result, self.coordinates));
                    }
                }
                Instruction::Push { spec, passive } => {
                    let target = if spec.target.is_some() {
                        spec.target
                    } else if let Some(key) = spec.key {
                        self.symbol_from_context(key)?
                    } else {
                        None
                    };
                    let Some(target) = target else {
                        continue;
                    };
                    let distance = if let Some(value) = spec
                        .distance_key
                        .and_then(|k| self.context.get(k))
                        .filter(|v| !matches!(v, Datum::Null))
                    {
                        value
                            .to_integer(&self.symbols)
                            .map_err(unsupported)?
                            .ok_or_else(|| EngineError::Rule("invalid push distance".into()))?
                    } else {
                        spec.distance
                    };
                    let Some(from) = self.world.position(target) else {
                        continue;
                    };
                    let source = if spec.source.is_some() {
                        spec.source
                    } else if let Some(key) = spec.source_key {
                        self.symbol_from_context(key)?
                            .and_then(|id| self.world.position(id))
                    } else {
                        None
                    };
                    let source =
                        source.or(self.planning.actor.and_then(|id| self.world.position(id)));
                    let Some(source) = source else {
                        continue;
                    };
                    if source == from {
                        continue;
                    }
                    if self.world.entity(target).is_none() {
                        if mandatory {
                            self.abort_action();
                        }
                        continue;
                    }
                    if !source.straight(from) {
                        continue;
                    }
                    if distance > 0 {
                        self.world.ensure_move_closure(false).map_err(unsupported)?;
                    }
                    let Some(pushed) =
                        crate::displacement::push(&self.world, source, from, distance, spec.ignore)
                            .map_err(EngineError::Rule)?
                    else {
                        continue;
                    };
                    if pushed.distance > 0 {
                        self.world
                            .move_entity(target, pushed.to)
                            .map_err(EngineError::Rule)?;
                    }
                    if let Some(key) = spec.collision {
                        self.context.set(key, Datum::Bool(pushed.collision));
                    }
                    self.context.set(spec.victim_output, Datum::Text(target));
                    let token = self
                        .world
                        .entity(target)
                        .is_some_and(|e| e.kind == EntityKind::Token);
                    if !token {
                        let child = self.generated_frame(passive, None, None)?;
                        self.stack.push(child);
                    }
                    result.events.push(NativeEvent::Pushed {
                        actor: self.planning.actor,
                        target,
                        token,
                        from,
                        to: pushed.to,
                        distance: pushed.distance,
                        collision: pushed.collision,
                    });
                }
                Instruction::Multi {
                    output,
                    filters,
                    tokens,
                    prompt,
                    min,
                    max,
                } => {
                    let list = |values: &[Symbol]| {
                        Datum::List(values.iter().copied().map(Datum::Text).collect())
                    };
                    if self
                        .planning
                        .actor
                        .is_none_or(|id| id == self.world.definitions.effect_empty)
                    {
                        self.context.set(output, list(&frame.multi));
                        continue;
                    }
                    let done = selected.as_ref().is_some_and(|v| matches!(v,Datum::Text(id) if self.symbols.name(*id)=="DONE" || *id==self.skip));
                    if done && (frame.multi.len() as i64) >= min {
                        self.context.set(output, list(&frame.multi));
                        continue;
                    }
                    if !done && let Some(value) = &selected {
                        let value = match value {
                            Datum::Text(id) => self.symbols.name(*id).to_owned(),
                            Datum::Int(n) => n.to_string(),
                            Datum::Bool(b) => if *b { "True" } else { "False" }.into(),
                            Datum::Null => "None".into(),
                            _ => String::new(),
                        };
                        if let Some(id) = self
                            .multi_choices(tokens, &filters, &frame)?
                            .into_iter()
                            .find(|id| self.symbols.name(*id) == value)
                        {
                            frame.multi.push(id);
                            self.context.set(output, list(&frame.multi));
                            if (frame.multi.len() as i64) >= max {
                                continue;
                            }
                        }
                    }
                    frame.pending = None;
                    let values = self.multi_choices(tokens, &filters, &frame)?;
                    if values.is_empty() {
                        self.context.set(output, list(&frame.multi));
                        if mandatory && (frame.multi.len() as i64) < min {
                            self.abort_action();
                        }
                        continue;
                    }
                    request = Some(NativeRequest {
                        controlled: None,
                        id: RequestToken::Generated(0),
                        owner: self.normalized_owner(self.planning.actor.unwrap()),
                        prompt,
                        movement_range: None,
                        pre_action_range: None,
                        choices: RequestChoices::Multiple {
                            values,
                            count: frame.multi.len(),
                            max,
                        },
                        can_skip: (frame.multi.len() as i64) >= min,
                        discard: false,
                    });
                }
                Instruction::RemoveToken { literal, key } => {
                    let token = if let Some(key) = key {
                        self.symbol_from_context(key)?
                    } else {
                        literal
                    };
                    let Some(token) = token else {
                        continue;
                    };
                    if !self
                        .world
                        .entity(token)
                        .is_some_and(|e| e.kind == EntityKind::Token)
                    {
                        continue;
                    }
                    if let Some(from) =
                        self.world.remove_entity(token).map_err(EngineError::Rule)?
                    {
                        let effects_removed = self.world.remove_token_effects(token);
                        result.events.push(NativeEvent::TokenRemoved {
                            actor: self.planning.actor,
                            token,
                            from,
                            effects_removed,
                        });
                    }
                }
                Instruction::ResolveCard { hero, program } => {
                    match self
                        .action_rules
                        .resolve(&self.world, hero, selected.as_ref(), &mut self.context)
                        .map_err(unsupported)?
                    {
                        ActionResolution::Prompt(menu) => {
                            request = Some(NativeRequest {
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
                        }
                        ActionResolution::Finished(children) => {
                            // Reject missing instruction families before queueing any part of the action.
                            let children: Vec<_> = children
                                .into_iter()
                                .flat_map(|child| {
                                    if matches!(child, ActionStep::Clear) {
                                        vec![("clear_select", None), ("clear_each", None)]
                                    } else {
                                        vec![(child.template(), Some(child))]
                                    }
                                })
                                .collect();
                            let ids = children
                                .iter()
                                .map(|(key, _)| {
                                    program
                                        .iter()
                                        .find(|(name, _)| name == key)
                                        .map(|(_, id)| *id)
                                        .ok_or_else(|| {
                                            unsupported(format!("action child not compiled: {key}"))
                                        })
                                })
                                .collect::<Result<Vec<_>>>()?;
                            let mut frames = Vec::with_capacity(children.len());
                            for ((_, child), id) in children.into_iter().zip(ids) {
                                let mut next = self.generated_frame(id, None, None)?;
                                next.action = child;
                                frames.push(next);
                            }
                            self.stack.extend(frames.into_iter().rev());
                        }
                    }
                }

                Instruction::CardText { hero, program } => {
                    let Some(h) = self.world.hero(hero) else {
                        continue;
                    };
                    let Some(id) = h.current else {
                        continue;
                    };
                    let c = self
                        .world
                        .card(id)
                        .ok_or_else(|| unsupported("current card missing"))?;
                    let d = &self.world.definitions.cards[c.definition];
                    let key = self
                        .symbols
                        .lookup("current_card_id")
                        .ok_or_else(|| unsupported("current card register missing"))?;
                    self.context.set(key, Datum::Text(d.printed_id));
                    if c.facedown || d.effect_absent {
                        continue;
                    }
                    let mut values = PrimaryValues {
                        hero: h.id,
                        card: d.printed_id,
                        name: d.name,
                        ranged: d.ranged,
                        stats: CardStats {
                            primary_value: 0,
                            range: 0,
                            radius: None,
                        },
                    };
                    let children = if let Some((_, children)) = program
                        .bodies
                        .iter()
                        .find(|(effect, _)| *effect == d.effect)
                    {
                        values.stats = self.world.card_stats(h.id, id).map_err(unsupported)?;
                        children.clone()
                    } else if self.reaction_rules.registered_effect(d.effect) {
                        return Err(unsupported(format!(
                            "primary card effect not compiled: {}",
                            self.symbols.name(d.effect)
                        )));
                    } else {
                        values.hero = hero;
                        match d.primary {
                            Some(crate::world::ActionKind::Movement) => {
                                values.stats.primary_value = self
                                    .world
                                    .computed_stat_in_context(
                                        hero,
                                        Stat::Movement,
                                        d.primary_value.unwrap_or(0),
                                        &self.context,
                                    )
                                    .map_err(unsupported)?;
                                vec![program.movement]
                            }
                            Some(crate::world::ActionKind::Attack) => {
                                values.stats.primary_value = self
                                    .world
                                    .computed_stat_in_context(
                                        hero,
                                        Stat::Attack,
                                        d.primary_value.unwrap_or(0),
                                        &self.context,
                                    )
                                    .map_err(unsupported)?;
                                values.stats.range = self
                                    .world
                                    .computed_stat_in_context(
                                        hero,
                                        Stat::Range,
                                        d.range.filter(|n| *n != 0).unwrap_or(1),
                                        &self.context,
                                    )
                                    .map_err(unsupported)?;
                                vec![program.attack]
                            }
                            Some(crate::world::ActionKind::Defense) => vec![program.defense],
                            Some(crate::world::ActionKind::Skill) => vec![program.skill],
                            _ => Vec::new(),
                        }
                    };
                    for id in children.into_iter().rev() {
                        let mut child = self.generated_frame(id, None, None)?;
                        child.unit_override = None;
                        child.effect = Some(EffectBinding {
                            card: values.card,
                            context: None,
                            range: None,
                            primary: Some(values.clone()),
                        });
                        self.stack.push(child);
                    }
                }
                Instruction::CheckAdjacency {
                    a,
                    b,
                    a_key,
                    b_key,
                    output,
                } => {
                    enum Identity {
                        Absent,
                        Unit(Symbol),
                        Unknown,
                        Unhashable,
                    }
                    let identity = |literal: Option<Symbol>, key: Option<Symbol>| {
                        if let Some(id) = literal.filter(|id| !self.symbols.name(*id).is_empty()) {
                            return Identity::Unit(id);
                        }
                        match key
                            .and_then(|k| self.context.get(k))
                            .filter(|v| v.truthy(&self.symbols))
                        {
                            Some(Datum::Text(id)) => Identity::Unit(*id),
                            Some(Datum::List(_) | Datum::Object(_) | Datum::Instance(_)) => {
                                Identity::Unhashable
                            }
                            Some(_) => Identity::Unknown,
                            None => Identity::Absent,
                        }
                    };
                    let (a, b) = (identity(a, a_key), identity(b, b_key));
                    let adjacent = if matches!(a, Identity::Absent) || matches!(b, Identity::Absent)
                    {
                        false
                    } else {
                        let position = |id| match id {
                            Identity::Unit(id) => Ok(self.world.position(id)),
                            Identity::Unhashable => {
                                Err(unsupported("unhashable adjacency identity"))
                            }
                            _ => Ok(None),
                        };
                        if let Some((from, to)) = position(a)?.zip(position(b)?) {
                            from.distance(to) == 1
                                && self
                                    .world
                                    .effect_connected(from, to, &[])
                                    .map_err(unsupported)?
                        } else {
                            false
                        }
                    };
                    self.context.set(output, Datum::Bool(adjacent));
                }
                Instruction::RecordHex { unit, key, output } => {
                    let unit = if unit.is_some() {
                        unit
                    } else if let Some(key) = key {
                        self.symbol_from_context(key)?
                    } else {
                        None
                    };
                    if let Some(hex) = unit.and_then(|id| self.world.position(id)) {
                        self.context.set(output, native_hex(hex, self.coordinates));
                    }
                }
                Instruction::Place {
                    unit,
                    key,
                    destination,
                    destination_key,
                } => {
                    let unit = if unit.is_some() {
                        unit
                    } else if let Some(key) = key {
                        self.symbol_from_context(key)?
                    } else {
                        None
                    };
                    let unit = unit.or(self.planning.actor);
                    let value = destination
                        .map(|h| native_hex(h, self.coordinates))
                        .or_else(|| self.context.get(destination_key).cloned())
                        .filter(|v| v.truthy(&self.symbols));
                    let (Some(unit), Some(value)) = (unit, value) else {
                        continue;
                    };
                    if value == Datum::Text(self.skip) {
                        continue;
                    }
                    let to = datum_hex(&value, &self.symbols, self.coordinates)?;
                    if self.world.occupant(to).is_some_and(|other| other != unit) {
                        continue;
                    }
                    if !self
                        .world
                        .can_displace(
                            unit,
                            self.planning
                                .actor
                                .filter(|id| !self.symbols.name(*id).is_empty())
                                .or(Some(unit)),
                            crate::validation::DisplacementKind::Place,
                            Some(to),
                        )
                        .map_err(unsupported)?
                        .allowed
                    {
                        if mandatory {
                            self.abort_action();
                        }
                        continue;
                    }
                    let token = self
                        .world
                        .entity(unit)
                        .is_some_and(|e| e.kind == EntityKind::Token);
                    let from = self.world.position(unit);
                    self.world
                        .place_entity(unit, to)
                        .map_err(EngineError::Rule)?;
                    result.events.push(if token {
                        NativeEvent::TokenMoved {
                            actor: self
                                .planning
                                .actor
                                .filter(|id| !self.symbols.name(*id).is_empty()),
                            token: unit,
                            from,
                            to,
                        }
                    } else {
                        NativeEvent::UnitPlaced { unit, from, to }
                    });
                }
                Instruction::Swap { a, b, a_key, b_key } => {
                    let a = if a.is_some() {
                        a
                    } else if let Some(key) = a_key {
                        self.symbol_from_context(key)?
                    } else {
                        None
                    };
                    let b = if b.is_some() {
                        b
                    } else if let Some(key) = b_key {
                        self.symbol_from_context(key)?
                    } else {
                        None
                    };
                    let (Some(a), Some(b)) = (a, b) else {
                        continue;
                    };
                    if a == self.skip || b == self.skip {
                        continue;
                    }
                    let (Some(from), Some(to)) = (self.world.position(a), self.world.position(b))
                    else {
                        continue;
                    };
                    if !self
                        .world
                        .can_displace(
                            a,
                            self.planning
                                .actor
                                .filter(|id| !self.symbols.name(*id).is_empty())
                                .or_else(|| self.symbols.lookup("system")),
                            crate::validation::DisplacementKind::Swap,
                            None,
                        )
                        .map_err(unsupported)?
                        .allowed
                        || !self
                            .world
                            .can_displace(
                                b,
                                self.planning
                                    .actor
                                    .filter(|id| !self.symbols.name(*id).is_empty())
                                    .or_else(|| self.symbols.lookup("system")),
                                crate::validation::DisplacementKind::Swap,
                                None,
                            )
                            .map_err(unsupported)?
                            .allowed
                    {
                        if mandatory {
                            self.abort_action();
                        }
                        continue;
                    }
                    self.world.remove_entity(a).map_err(EngineError::Rule)?;
                    self.world.remove_entity(b).map_err(EngineError::Rule)?;
                    self.world.place_entity(a, to).map_err(EngineError::Rule)?;
                    self.world
                        .place_entity(b, from)
                        .map_err(EngineError::Rule)?;
                    result.events.push(NativeEvent::Swapped { a, b, from, to });
                }
                Instruction::Log => {}
                Instruction::CardSelect(spec) => {
                    let owner = spec
                        .owner(
                            self.planning.actor,
                            &self.context,
                            &self.symbols,
                            frame.forced.as_ref(),
                        )
                        .map_err(unsupported)?;
                    let choices = spec
                        .choices(
                            &self.world,
                            &self.context,
                            &self.symbols,
                            owner,
                            frame.forced.as_ref(),
                        )
                        .map_err(unsupported)?;
                    if choices.is_empty() {
                        if mandatory {
                            self.abort_action();
                        } else {
                            self.context.remove(spec.output);
                        }
                        continue;
                    }
                    if spec.auto && choices.len() == 1 && mandatory {
                        let c = self.world.card(choices[0]).unwrap();
                        let printed = self.world.definitions.cards[c.definition].printed_id;
                        spec.store(&self.world, &mut self.context, &choices, printed)
                            .map_err(unsupported)?;
                        continue;
                    }
                    if selected == Some(Datum::Text(self.skip)) && !mandatory {
                        self.context.remove(spec.output);
                        continue;
                    }
                    if let Some(Datum::Text(id)) = selected.as_ref()
                        && choices.iter().any(|c| {
                            self.world.card(*c).is_some_and(|c| {
                                self.world.definitions.cards[c.definition].printed_id == *id
                            })
                        })
                    {
                        spec.store(&self.world, &mut self.context, &choices, *id)
                            .map_err(unsupported)?;
                        continue;
                    }
                    request = Some(NativeRequest {
                        controlled: None,
                        id: RequestToken::Generated(0),
                        owner: self.normalized_owner(owner.unwrap_or(self.no_actor)),
                        prompt: spec.prompt,
                        movement_range: None,
                        pre_action_range: None,
                        choices: RequestChoices::CardIds {
                            values: choices
                                .iter()
                                .map(|id| {
                                    self.world.definitions.cards
                                        [self.world.card(*id).unwrap().definition]
                                        .printed_id
                                })
                                .collect(),
                            forced_victim: frame.forced.as_ref().map(|f| f.victim),
                            defeat_victim: frame.defeat_prompt_victim,
                        },
                        can_skip: !mandatory,
                        discard: spec.discard,
                    });
                }
                Instruction::ForceDiscard {
                    victim,
                    key,
                    basic,
                    immunity,
                    select,
                    discard,
                } => {
                    let victim = if victim.is_some() {
                        victim
                    } else if let Some(key) = key {
                        self.symbol_from_context(key)?
                    } else {
                        None
                    };
                    let Some(victim) = victim else {
                        continue;
                    };
                    let Some(hero) = self.world.hero(victim) else {
                        continue;
                    };
                    if let Some(source) = immunity
                        && self
                            .world
                            .actor_immune(hero.id, source)
                            .map_err(unsupported)?
                    {
                        continue;
                    }
                    let eligible = hero
                        .hand
                        .iter()
                        .copied()
                        .filter(|id| {
                            self.world.card(*id).is_some_and(|c| {
                                basic.is_none_or(|b| {
                                    self.world.definitions.cards[c.definition].basic == b
                                })
                            })
                        })
                        .collect::<Vec<_>>();
                    if eligible.is_empty() {
                        continue;
                    }
                    let shields = self.world.active_shield_cards(hero.id);
                    let allowed = if !shields.is_empty() && basic.is_some() {
                        Some(Arc::from(
                            eligible
                                .iter()
                                .chain(shields.iter())
                                .map(|id| {
                                    self.world.definitions.cards
                                        [self.world.card(*id).unwrap().definition]
                                        .printed_id
                                })
                                .collect::<Vec<_>>(),
                        ))
                    } else {
                        None
                    };
                    let binding = ForceBinding {
                        victim,
                        basic,
                        shields: !shields.is_empty(),
                        allowed,
                    };
                    for id in [discard, select] {
                        let mut child = self.generated_frame(id, None, None)?;
                        child.forced = Some(binding.clone());
                        self.stack.push(child);
                    }
                }

                Instruction::DiscardOrDefeat {
                    key,
                    killer,
                    killer_key,
                    immunity,
                    select,
                    discard,
                    defeat,
                } => {
                    let Some(victim) = self.symbol_from_context(key)? else {
                        continue;
                    };
                    let Some(hero) = self.world.hero(victim) else {
                        continue;
                    };
                    if let Some(source) = immunity
                        && self
                            .world
                            .actor_immune(hero.id, source)
                            .map_err(unsupported)?
                    {
                        continue;
                    }
                    if self.world.position(victim).is_none()
                        && !(hero.id == victim && self.world.has_board_presence(victim))
                    {
                        continue;
                    }
                    if hero.hand.is_empty() {
                        let killer = if killer.is_some() {
                            killer
                        } else if let Some(key) = killer_key {
                            self.symbol_from_context(key)?.or(self.planning.actor)
                        } else {
                            self.planning.actor
                        };
                        let mut child = self.generated_frame(defeat, None, None)?;
                        child.defeat_override = Some(DefeatBinding { victim, killer });
                        self.stack.push(child);
                    } else {
                        let child = self.generated_frame(discard, None, None)?;
                        self.stack.push(child);
                        let mut child = self.generated_frame(select, None, None)?;
                        child.defeat_prompt_victim = Some(victim);
                        self.stack.push(child);
                    }
                }

                Instruction::Attack {
                    spec,
                    select,
                    children,
                } => {
                    let Some(expansion) = self
                        .attack_rules
                        .prepare(
                            &self.world,
                            &mut self.context,
                            &self.symbols,
                            self.planning.actor,
                            &spec,
                            frame.attack_followup,
                            self.track_attack_source,
                        )
                        .map_err(unsupported)?
                    else {
                        if mandatory {
                            self.abort_action();
                        }
                        continue;
                    };
                    if expansion.select && !mandatory {
                        let mut followup = frame.clone();
                        followup.attack_followup = Some(expansion.binding.key);
                        self.stack.push(followup);
                    } else {
                        for child in children.into_iter().rev() {
                            let mut child = self.generated_frame(child, None, None)?;
                            child.attack_binding = Some(expansion.binding);
                            self.stack.push(child);
                        }
                    }
                    if expansion.select {
                        let mut child = self.generated_frame(select, None, None)?;
                        child.attack_binding = Some(expansion.binding);
                        self.stack.push(child);
                    }
                }
                Instruction::CheckDefeated {
                    target,
                    output,
                    by_actor,
                } => {
                    let value = self
                        .attack_rules
                        .defeated(
                            &self.world,
                            &self.context,
                            &self.symbols,
                            self.planning.actor,
                            target,
                            by_actor,
                        )
                        .map_err(unsupported)?;
                    self.context.set(output, Datum::Bool(value));
                }

                Instruction::Reaction {
                    target,
                    discard,
                    passive,
                } => {
                    match self
                        .reaction_rules
                        .window(
                            &self.world,
                            &mut self.context,
                            &self.symbols,
                            frame.attack_binding.map_or(target, |b| b.key),
                            self.planning.actor,
                            selected.as_ref(),
                        )
                        .map_err(unsupported)?
                    {
                        Window::Done => {}
                        Window::Selected { owner, card } => {
                            let child = self.generated_frame(passive, Some(owner), None)?;
                            self.stack.push(child);
                            let mut child = self.generated_frame(discard, None, None)?;
                            child.discard_override = Some((owner, card));
                            self.stack.push(child);
                        }
                        Window::Wait(menu) => {
                            if selected.as_ref().is_some_and(|v| v.truthy(&self.symbols)) {
                                frame.pending = None;
                            }
                            request = Some(NativeRequest {
                                controlled: None,
                                id: RequestToken::Generated(0),
                                owner: menu.owner,
                                prompt: target,
                                movement_range: None,
                                pre_action_range: None,
                                choices: RequestChoices::Defense(menu),
                                can_skip: false,
                                discard: false,
                            });
                        }
                    }
                }
                Instruction::Discard {
                    hero,
                    hero_key,
                    card,
                    card_key,
                    played,
                    passive,
                } => {
                    let binding = if let Some(binding) = frame.discard_override {
                        Some(binding)
                    } else {
                        let hero = if let Some(forced) = frame.forced.as_ref() {
                            Some(forced.victim)
                        } else if hero.is_some() {
                            hero
                        } else if let Some(key) = hero_key {
                            self.symbol_from_context(key)?
                        } else {
                            None
                        };
                        let card = if card.is_some() {
                            card
                        } else if let Some(key) = card_key {
                            self.symbol_from_context(key)?
                        } else {
                            None
                        };
                        hero.zip(card)
                    };
                    if let Some((hero, card)) = binding
                        && self
                            .reaction_rules
                            .discard(&mut self.world, &mut self.context, hero, card, played)
                            .map_err(unsupported)?
                    {
                        let child = self.generated_frame(passive, Some(hero), None)?;
                        self.stack.push(child);
                    }
                }
                Instruction::Passive { hero, trigger } => {
                    let hero = frame.unit_override.flatten().or(hero);
                    let offers = self
                        .passive_rules
                        .scan(&self.world, self.planning.actor, hero, trigger)
                        .map_err(unsupported)?;
                    for binding in offers.into_iter().rev() {
                        let mut child = self.generated_frame(
                            self.passive_offer.ok_or_else(|| {
                                unsupported("newly eligible passive requires recompilation")
                            })?,
                            None,
                            None,
                        )?;
                        child.offer_override = Some(binding);
                        self.stack.push(child);
                    }
                }
                Instruction::OfferPassive { spec, mark } => {
                    let spec = frame.offer_override.unwrap_or(spec);
                    let Some(owner) = spec
                        .hero
                        .or(self.planning.actor)
                        .and_then(|h| self.world.hero(h).map(|h| h.id))
                    else {
                        continue;
                    };
                    let Some((card, effect)) = self.passive_rules.resolve(&self.world, owner, spec)
                    else {
                        continue;
                    };
                    let c = self.world.card(card).unwrap();
                    let d = &self.world.definitions.cards[c.definition];
                    if !self.reaction_rules.registered_effect(effect) {
                        continue;
                    }
                    if spec.optional && selected.is_none() {
                        request = Some(NativeRequest {
                            controlled: None,
                            id: RequestToken::Generated(0),
                            owner,
                            prompt: spec.prompt,
                            movement_range: None,
                            pre_action_range: None,
                            choices: RequestChoices::Passive {
                                card: spec.card,
                                name: d.name,
                            },
                            can_skip: false,
                            discard: false,
                        });
                    } else if (!spec.optional || selected == Some(Datum::Text(self.yes)))
                        && self
                            .passive_rules
                            .body(
                                &self.world,
                                &self.context,
                                &self.symbols,
                                owner,
                                effect,
                                spec.trigger,
                            )
                            .map_err(unsupported)?
                    {
                        let templates = self
                            .passive_bodies
                            .iter()
                            .find(|body| body.effect == effect && body.owner == owner)
                            .ok_or_else(|| {
                                unsupported("eligible passive body requires compilation")
                            })?
                            .templates
                            .clone();
                        let mut child = self.generated_frame(mark, None, None)?;
                        child.offer_override = Some(spec);
                        self.stack.push(child);
                        for id in templates.into_iter().rev() {
                            let mut child = self.generated_frame(id, None, None)?;
                            // The recipe already carries the passive owner's literal
                            // unit/origins; absence of an override must preserve them.
                            child.unit_override = None;
                            self.stack.push(child);
                        }
                    }
                }
                Instruction::MarkPassive { card, provider } => {
                    let card = frame.offer_override.map_or(card, |o| o.card);
                    let provider = frame.offer_override.map_or(provider, |o| o.provider);
                    crate::passives::PassiveRules::mark(
                        &mut self.world,
                        self.planning.actor,
                        card,
                        provider,
                    )
                    .map_err(unsupported)?;
                }
                Instruction::DefenseText { on_block, program } => {
                    if let Some(plan) = self
                        .reaction_rules
                        .defense_text(&self.world, &self.context, &self.symbols, on_block)
                        .map_err(unsupported)?
                        && !plan.operations.is_empty()
                    {
                        let child = self.generated_frame(program.exit, None, None)?;
                        self.stack.push(child);
                        for op in plan.operations.into_iter().rev() {
                            let (instruction, unit, context, range) = match op {
                                DefenseOp::Flag { key, value } => {
                                    (program.flag, None, Some((key, value)), None)
                                }
                                DefenseOp::PlaceBounty { target } => {
                                    (program.place, Some(target), None, None)
                                }
                                DefenseOp::RemoveBounty => (program.remove, None, None, None),
                                DefenseOp::ProjectileTarget { deflect, range } => (
                                    if deflect {
                                        program.deflect_select.ok_or_else(|| {
                                            unsupported("deflect targeting body missing")
                                        })?
                                    } else {
                                        program.reflect_select.ok_or_else(|| {
                                            unsupported("reflect targeting body missing")
                                        })?
                                    },
                                    None,
                                    None,
                                    Some(range),
                                ),
                                DefenseOp::ProjectileDiscard { deflect } => (
                                    if deflect {
                                        program.deflect_discard.ok_or_else(|| {
                                            unsupported("deflect discard body missing")
                                        })?
                                    } else {
                                        program.reflect_discard.ok_or_else(|| {
                                            unsupported("reflect discard body missing")
                                        })?
                                    },
                                    None,
                                    None,
                                    None,
                                ),
                            };
                            let mut child = self.generated_frame(instruction, unit, None)?;
                            child.effect = Some(EffectBinding {
                                card: plan.card,
                                context,
                                range,
                                primary: None,
                            });
                            self.stack.push(child);
                        }
                        let child = self.generated_frame(program.enter, None, None)?;
                        self.stack.push(child);
                    }
                }
                Instruction::PlaceMarker {
                    kind,
                    target,
                    key,
                    value,
                    passive,
                } => {
                    let target = if let Some(unit) = frame.unit_override {
                        unit
                    } else if target.is_some() {
                        target
                    } else if let Some(key) = key {
                        self.symbol_from_context(key)?
                    } else {
                        None
                    };
                    if let Some(target) = target
                        && let Some(event) = self.marker_rules.place(
                            &mut self.world,
                            &mut self.context,
                            &self.symbols,
                            self.planning.actor,
                            MarkerPlacement {
                                kind,
                                target,
                                value,
                            },
                        )
                    {
                        result.events.push(NativeEvent::Marker(event));
                        let child = self.generated_frame(passive, None, None)?;
                        self.stack.push(child);
                    }
                }
                Instruction::RemoveMarker(kind) => {
                    if let Some(event) = self.marker_rules.remove(&mut self.world, kind) {
                        result.events.push(NativeEvent::Marker(event));
                    }
                }
                Instruction::FinishedEffect => {}
                Instruction::RestoreContext { .. } => {
                    return Err(unsupported(
                        "action-context restoration requires live object bindings",
                    ));
                }
                Instruction::RestoreAction => self
                    .reaction_rules
                    .restore(&mut self.context)
                    .map_err(unsupported)?,
                Instruction::PreMovement {
                    hero,
                    key,
                    children,
                } => {
                    let hero = if let Some(hero) =
                        hero.filter(|id| *id != self.world.definitions.effect_empty)
                    {
                        Some(hero)
                    } else {
                        key.map(|key| self.symbol_from_context(key))
                            .transpose()?
                            .flatten()
                    };
                    let Some(hero) = hero.filter(|id| *id != self.world.definitions.effect_empty)
                    else {
                        continue;
                    };
                    let Some(range) = self.world.pre_action_movement_range(hero) else {
                        continue;
                    };
                    let range = i32::try_from(range)
                        .map_err(|_| unsupported("pre-action movement range outside i32"))?;
                    for id in children.into_iter().rev() {
                        let mut child = self.generated_frame(id, None, None)?;
                        child.unit_override = None;
                        child.pre_move = Some((hero, range));
                        self.stack.push(child);
                    }
                }
                Instruction::PreDiscard {
                    hero,
                    key,
                    abort,
                    force,
                    defeat,
                } => {
                    let hero = if let Some(hero) =
                        hero.filter(|id| *id != self.world.definitions.effect_empty)
                    {
                        Some(hero)
                    } else {
                        key.map(|key| self.symbol_from_context(key))
                            .transpose()?
                            .flatten()
                    };
                    let Some(hero) = hero.filter(|id| *id != self.world.definitions.effect_empty)
                    else {
                        continue;
                    };
                    let Some(owner) = self.world.hero(hero) else {
                        continue;
                    };
                    if !self.world.has_board_presence(hero) {
                        if abort {
                            self.abort_action();
                        }
                        continue;
                    }
                    let has_hand = !owner.hand.is_empty();
                    let mut next = None;
                    for effect in &self.world.effects.instances {
                        if effect.definition.behavior.kind != crate::effect_rules::Kind::PreDiscard
                            || frame.pre_processed.contains(&effect.id)
                            || !self
                                .world
                                .pre_action_discard_applies(effect, hero)
                                .map_err(unsupported)?
                        {
                            continue;
                        }
                        frame.pre_processed.push(effect.id);
                        if has_hand || effect.definition.behavior.discard_or_defeat {
                            next = Some((effect.id, effect.source));
                            break;
                        }
                    }
                    if let Some((effect, source)) = next {
                        self.stack.push(frame.clone());
                        if has_hand {
                            self.world.remove_effects(&[effect]);
                            let key = self
                                .symbols
                                .lookup("pre_action_discard_victim")
                                .ok_or_else(|| unsupported("pre-discard key missing"))?;
                            self.context.set(key, Datum::Text(hero));
                            let child = self.generated_frame(force, None, None)?;
                            self.stack.push(child);
                        } else {
                            let mut child = self.generated_frame(defeat, None, None)?;
                            child.defeat_override = Some(DefeatBinding {
                                victim: hero,
                                killer: Some(source),
                            });
                            self.stack.push(child);
                        }
                    }
                }

                Instruction::Remove { unit, key } => {
                    let id = if frame.unit_override.is_some() {
                        frame.unit_override.flatten()
                    } else if let Some(key) = key {
                        self.symbol_from_context(key)?
                    } else {
                        unit
                    };
                    let Some(id) = id else {
                        continue;
                    };
                    let from = self.world.remove_entity(id).map_err(EngineError::Rule)?;
                    result
                        .events
                        .push(NativeEvent::Combat(CombatEvent::Removed {
                            target: id,
                            from,
                        }));
                }
                Instruction::GameOver(spec) => {
                    let spec = frame.outcome_override.unwrap_or(spec);
                    if spec.team.is_some() == spec.hero.is_some() {
                        return Err(EngineError::Rule("game over requires one winner".into()));
                    }
                    self.world.combat.outcome = Some(spec);
                    self.planning.phase = Phase::GameOver;
                    self.stack.clear();
                    result
                        .events
                        .push(NativeEvent::Combat(CombatEvent::GameOver(spec)));
                }
                Instruction::Combat {
                    damage,
                    target,
                    defeat,
                } => {
                    let (event, binding) = self
                        .combat_rules
                        .resolve(
                            &self.world,
                            &mut self.context,
                            &self.symbols,
                            self.planning.actor,
                            frame.attack_binding.map_or(target, |b| b.key),
                            frame.attack_binding.map_or(damage, |b| b.damage),
                        )
                        .map_err(combat_error)?;
                    if let Some(event) = event {
                        result.events.push(NativeEvent::Combat(event));
                    }
                    if let Some(binding) = binding {
                        let mut child = self.generated_frame(defeat, None, None)?;
                        child.defeat_override = Some(binding);
                        self.stack.push(child);
                    }
                }
                Instruction::Defeat {
                    protection,
                    victim,
                    key,
                    killer,
                    assist,
                    remove,
                    over,
                } => {
                    let binding = if let Some(binding) = frame.defeat_override {
                        Some(binding)
                    } else {
                        let victim = if victim.is_some() {
                            victim
                        } else if let Some(key) = key {
                            self.symbol_from_context(key)?
                        } else {
                            None
                        };
                        victim.map(|victim| DefeatBinding { victim, killer })
                    };
                    let Some(binding) = binding else {
                        continue;
                    };
                    let outcome = self
                        .combat_rules
                        .defeat(
                            &mut self.world,
                            &mut self.context,
                            binding.victim,
                            binding.killer,
                            assist,
                        )
                        .map_err(combat_error)?;
                    if outcome.protection {
                        let mut child = self.generated_frame(
                            protection.ok_or_else(|| {
                                unsupported("new minion protection requires recompilation")
                            })?,
                            None,
                            None,
                        )?;
                        child.protection = Some(crate::protection::ProtectionBinding {
                            minion: binding.victim,
                            killer: binding.killer,
                            tried: Arc::from([]),
                        });
                        self.stack.push(child);
                        continue;
                    }
                    result
                        .events
                        .extend(outcome.events.into_iter().map(NativeEvent::Combat));
                    if let Some(spec) = outcome.outcome {
                        let mut child = self.generated_frame(over, None, None)?;
                        child.outcome_override = Some(spec);
                        self.stack.push(child);
                    }
                    for id in outcome.removals.into_iter().rev() {
                        let child = self.generated_frame(remove, Some(id), None)?;
                        self.stack.push(child);
                    }
                }
                Instruction::Set { key, value } => {
                    let (key, value) = frame
                        .effect
                        .as_ref()
                        .and_then(|e| e.context.clone())
                        .unwrap_or((key, value));
                    self.context.set(key, value);
                }
                Instruction::Add { key, amount } => {
                    let value = match self.context.get(key) {
                        Some(Datum::Float(v)) => Datum::Float(v + amount as f64),
                        Some(Datum::Int(v)) => Datum::Int(
                            v.checked_add(amount)
                                .ok_or_else(|| unsupported("integer addition outside i64"))?,
                        ),
                        Some(Datum::Bool(v)) => Datum::Int(
                            i64::from(*v)
                                .checked_add(amount)
                                .ok_or_else(|| unsupported("integer addition outside i64"))?,
                        ),
                        _ => Datum::Int(amount),
                    };
                    self.context.set(key, value);
                }
                Instruction::SetActor { literal, key, save } => {
                    self.context.set(
                        save,
                        self.planning
                            .actor
                            .filter(|id| !self.symbols.name(*id).is_empty())
                            .map_or(Datum::Null, Datum::Text),
                    );
                    let next = scope_actor(literal, key, &self.symbols, |key| {
                        self.symbol_from_context(key)
                    })?;
                    if let Some(actor) = next.filter(|id| !self.symbols.name(*id).is_empty()) {
                        self.planning.actor = Some(actor);
                    }
                }
                Instruction::Record { input, output } => {
                    if let Some(value) = self
                        .context
                        .get(input)
                        .filter(|v| v.truthy(&self.symbols))
                        .cloned()
                    {
                        if matches!(value, Datum::List(_) | Datum::Object(_)) {
                            return Err(unsupported(
                                "mutable aggregate target aliases require the context arena",
                            ));
                        }
                        match self.context.get(output).cloned() {
                            None => self.context.set(output, Datum::List(vec![value])),
                            Some(Datum::List(mut items)) => {
                                items.push(value);
                                self.context.set(output, Datum::List(items));
                            }
                            _ => {}
                        }
                    }
                }
                Instruction::Retrieve {
                    card,
                    hero_key,
                    all,
                } => {
                    let actor = if let Some(key) = hero_key {
                        self.symbol_from_context(key)?
                    } else {
                        self.planning.actor
                    };
                    let Some(actor) = actor.filter(|id| !self.symbols.name(*id).is_empty()) else {
                        continue;
                    };
                    let printed = if all {
                        None
                    } else {
                        match self.context.get(card) {
                            Some(Datum::Text(id)) if !self.symbols.name(*id).is_empty() => {
                                Some(*id)
                            }
                            _ => None,
                        }
                    };
                    for returned in self
                        .world
                        .retrieve_card(actor, printed, all)
                        .map_err(card_error)?
                    {
                        result.events.push(NativeEvent::CardRetrieved {
                            hero: actor,
                            card: returned.card,
                            name: returned.name,
                            kept: None,
                        });
                    }
                }
                Instruction::ParkedPhase(_)
                | Instruction::LiveReturnMinions
                | Instruction::LivePlace
                | Instruction::LiveSwap
                | Instruction::LiveRepeat { .. }
                | Instruction::LiveTravel { .. }
                | Instruction::LivePerformPrimary
                | Instruction::LivePerformCard
                | Instruction::LiveCardAction
                | Instruction::LiveChoosePiece
                | Instruction::LivePush
                | Instruction::LiveCoordinates(_)
                | Instruction::LivePrimaryText
                | Instruction::LiveResource
                | Instruction::LiveCreateEffect
                | Instruction::LiveDefenseBody { .. }
                | Instruction::LiveMarker { .. }
                | Instruction::LiveMove
                | Instruction::LiveTokenMove
                | Instruction::LiveDrag
                | Instruction::LiveMoveToward
                | Instruction::LiveSetInitiative
                | Instruction::LiveActionControl
                | Instruction::LiveJourney
                | Instruction::LiveMineChoice
                | Instruction::LiveTriggerMines
                | Instruction::LivePassive
                | Instruction::LiveOfferPassive
                | Instruction::LiveMoveSequence
                | Instruction::LiveForceDefenseMovement
                | Instruction::LiveHexSelect
                | Instruction::LiveUnitSelect
                | Instruction::LiveMultiSelect
                | Instruction::LiveDefeat
                | Instruction::LivePreMovement
                | Instruction::LivePreDiscard
                | Instruction::LiveReaction
                | Instruction::LiveAttackSequence
                | Instruction::LiveCombatResolve
                | Instruction::LiveAdjacentSnapshot
                | Instruction::LiveTargetDefeated
                | Instruction::LiveRemoveUnit
                | Instruction::LiveGameOver
                | Instruction::LiveProtection
                | Instruction::LiveQuery { .. }
                | Instruction::LiveContextCondition
                | Instruction::LiveSpell(_)
                | Instruction::LiveCheckUnitFilters
                | Instruction::LiveDistanceCheck
                | Instruction::LiveUnitTypeCheck
                | Instruction::LiveUnitOnBoard
                | Instruction::LiveBooleanCombine
                | Instruction::LiveConvertCardItem
                | Instruction::LiveAfterAttackText
                | Instruction::LiveMinionRespawn
                | Instruction::LiveAdjacentEnemies
                | Instruction::LiveZoneChange
                | Instruction::LiveSoloWin
                | Instruction::LiveCardCount
                | Instruction::LiveRoundDefeat
                | Instruction::LiveRuneChoice
                | Instruction::LiveMirrorPush
                | Instruction::LiveRemovePiece
                | Instruction::LiveSpawnPiece
                | Instruction::LiveSetPiece
                | Instruction::LiveLaugh
                | Instruction::LiveRevealHandCard
                | Instruction::LiveCardGuess { .. }
                | Instruction::LiveChooseCardColor
                | Instruction::LiveDirectionalMove
                | Instruction::LiveRuneInscription
                | Instruction::LiveMarkPassive
                | Instruction::LiveDiscard
                | Instruction::LiveRetrieve
                | Instruction::LiveTokenPlace
                | Instruction::LiveTokenPath { .. }
                | Instruction::LiveTurret { .. }
                | Instruction::LiveRockUltimate
                | Instruction::LiveTokenBatch
                | Instruction::LiveTokenRemove
                | Instruction::LiveForceDiscard
                | Instruction::LiveColorDiscard
                | Instruction::LiveResolvedCardSwap
                | Instruction::LiveCardSwap
                | Instruction::LiveDeckCardSwap
                | Instruction::LiveItemSwap
                | Instruction::LiveDiscardOrDefeat
                | Instruction::LiveCardSelect => {
                    return Err(unsupported(
                        "parked arena phase step cannot run in the ordinary interpreter",
                    ));
                }
                Instruction::RetrieveUnresolved { hero, prompt } => {
                    let Some(index) = self.world.hero_index(hero) else {
                        continue;
                    };
                    let h = &self.world.heroes[index];
                    let (Some(first), Some(second)) = (h.current, h.extra) else {
                        continue;
                    };
                    let first_card = self
                        .world
                        .card(first)
                        .ok_or_else(|| unsupported("missing unresolved card"))?;
                    let first_def = &self.world.definitions.cards[first_card.definition];
                    let second_card = self
                        .world
                        .card(second)
                        .ok_or_else(|| unsupported("missing extra card"))?;
                    let second_def = &self.world.definitions.cards[second_card.definition];
                    if let Some(Datum::Text(chosen)) = selected
                        && (chosen == first_def.printed_id || chosen == second_def.printed_id)
                    {
                        let (retrieved, kept) = if chosen == first_def.printed_id {
                            (first, second)
                        } else {
                            (second, first)
                        };
                        let (printed, name) = if retrieved == first {
                            (first_def.printed_id, first_def.name)
                        } else {
                            (second_def.printed_id, second_def.name)
                        };
                        let kept_printed = if kept == first {
                            first_def.printed_id
                        } else {
                            second_def.printed_id
                        };
                        self.world.heroes[index].extra = None;
                        self.world
                            .return_card_to_hand(hero, retrieved)
                            .map_err(|e| unsupported(e.to_string()))?;
                        self.world.heroes[index].current = Some(kept);
                        result.events.push(NativeEvent::CardRetrieved {
                            hero,
                            card: printed,
                            name,
                            kept: Some(kept_printed),
                        });
                        continue;
                    }
                    // The Python step clears malformed pending input before reissuing.
                    frame.pending = None;
                    request = Some(NativeRequest {
                        controlled: None,
                        id: RequestToken::Generated(0),
                        owner: hero,
                        prompt,
                        movement_range: None,
                        pre_action_range: None,
                        choices: RequestChoices::Cards(vec![
                            (
                                first_def.printed_id,
                                first_def.name,
                                first_card
                                    .initiative_override
                                    .unwrap_or(first_def.initiative),
                            ),
                            (
                                second_def.printed_id,
                                second_def.name,
                                second_card
                                    .initiative_override
                                    .unwrap_or(second_def.initiative),
                            ),
                        ]),
                        can_skip: false,
                        discard: false,
                    });
                }
                Instruction::FlipCoin => {
                    self.team = if self.team == Team::Red {
                        Team::Blue
                    } else {
                        Team::Red
                    };
                    result.events.push(NativeEvent::TieBreakerFlipped {
                        blue: self.team == Team::Blue,
                    });
                }
                Instruction::ChoosePiece { hero, prompt } => {
                    let Some(h) = self.world.hero(hero) else {
                        continue;
                    };
                    if !self.world.entity(h.id).is_some_and(|e| e.multi_piece) {
                        continue;
                    }
                    let name = self.world.entity(h.id).unwrap().name;
                    let pieces = self.world.piece_ids(hero).collect::<Vec<_>>();
                    if pieces.is_empty() {
                        continue;
                    }
                    if pieces.len() == 1 {
                        self.world.acting_piece = Some(pieces[0]);
                        continue;
                    }
                    if let Some(Datum::Text(id)) = selected
                        && pieces.contains(&id)
                    {
                        self.world.acting_piece = Some(id);
                        continue;
                    }
                    frame.pending = None;
                    request = Some(NativeRequest {
                        controlled: None,
                        id: RequestToken::Generated(0),
                        owner: hero,
                        prompt,
                        movement_range: None,
                        pre_action_range: None,
                        choices: RequestChoices::Units {
                            tokens: false,
                            values: pieces
                                .into_iter()
                                .map(|id| (id, self.world.position(id).map(|hex| (name, hex))))
                                .collect(),
                        },
                        can_skip: false,
                        discard: false,
                    });
                }
                Instruction::SetPiece { hero, piece, key } => {
                    let id = if piece.is_some() {
                        piece
                    } else if let Some(key) = key {
                        self.symbol_from_context(key)?
                    } else {
                        None
                    };
                    let Some(id) = id.filter(|id| !self.symbols.name(*id).is_empty()) else {
                        continue;
                    };
                    if self.world.piece_ids(hero).any(|p| p == id) {
                        self.world.acting_piece = Some(id);
                    } else if mandatory {
                        self.abort_action();
                    }
                }
                Instruction::Unit {
                    output,
                    filters,
                    tokens,
                    prompt,
                    auto,
                    owner,
                    owner_key,
                    discard,
                } => {
                    let (output, filters) = if let Some(binding) = frame.attack_binding {
                        let mut filters = filters;
                        let Some(UnitFilter::Range(HexFilter::Range { max, .. })) =
                            filters.first_mut()
                        else {
                            return Err(unsupported("attack selection lacks its range predicate"));
                        };
                        *max = Some(binding.range);
                        (binding.key, filters)
                    } else {
                        (output, filters)
                    };
                    let mut filters = filters;
                    if let Some(range) = frame.effect.as_ref().and_then(|e| e.range) {
                        for filter in &mut filters {
                            if let UnitFilter::Range(HexFilter::Range { max, .. }) = filter {
                                *max = Some(range);
                            }
                        }
                    }
                    let target_actor = if owner.is_some() {
                        owner
                    } else if let Some(key) = owner_key {
                        self.symbol_from_context(key)?.or(self.planning.actor)
                    } else {
                        self.planning.actor
                    };
                    let spatial = SpatialContext {
                        count_observer: Some(&self.count_observer),
                        world: &self.world,
                        registers: &self.context,
                        symbols: &self.symbols,
                        actor: self.planning.actor,
                        frame: frame.unit_override,
                        coordinates: self.coordinates,
                        range_override: None,
                        through_override: None,
                    };
                    let choices = self
                        .world
                        .select_units(
                            if tokens {
                                UnitCandidates::UnitsAndTokens
                            } else {
                                UnitCandidates::UnitsWithIllusions
                            },
                            target_actor,
                            &filters,
                            &spatial,
                        )
                        .map_err(unsupported)?;
                    self.cleanup_count_origin();
                    if choices.is_empty() {
                        if mandatory {
                            self.abort_action();
                        } else {
                            self.context.remove(output);
                        }
                        continue;
                    }
                    if auto && choices.len() == 1 && mandatory {
                        self.context.set(output, Datum::Text(choices[0]));
                        continue;
                    }
                    if selected == Some(Datum::Text(self.skip)) && !mandatory {
                        self.context.remove(output);
                        continue;
                    }
                    if let Some(Datum::Text(id)) = selected
                        && choices.contains(&id)
                    {
                        self.context.set(output, Datum::Text(id));
                        continue;
                    }
                    request = Some(NativeRequest {
                        controlled: None,
                        id: RequestToken::Generated(0),
                        owner: self.normalized_owner(target_actor.unwrap_or(self.no_actor)),
                        prompt,
                        movement_range: None,
                        pre_action_range: None,
                        choices: RequestChoices::Units {
                            tokens,
                            values: choices.into_iter().map(|id| (id, None)).collect(),
                        },
                        can_skip: !mandatory,
                        discard,
                    });
                }
                Instruction::Number {
                    output,
                    options,
                    labels,
                    prompt,
                    auto,
                    owner,
                    owner_key,
                    discard,
                } => {
                    if options.is_empty() {
                        if mandatory {
                            if self
                                .context
                                .get(self.action_stack_key)
                                .is_some_and(|v| v.truthy(&self.symbols))
                            {
                                return Err(unsupported(
                                    "action-context abort restoration not compiled",
                                ));
                            }
                            self.abort_action();
                        } else {
                            self.context.remove(output);
                        }
                        continue;
                    }
                    if auto && options.len() == 1 && mandatory {
                        self.context.set(output, Datum::Int(options[0]));
                        continue;
                    }
                    if selected == Some(Datum::Text(self.skip)) && !mandatory {
                        self.context.remove(output);
                        continue;
                    }
                    if let Some(value) = selected.as_ref().filter(|v| !matches!(v, Datum::Null))
                        && let Some(value) = value.to_integer(&self.symbols).map_err(unsupported)?
                        && options.contains(&value)
                    {
                        self.context.set(output, Datum::Int(value));
                        continue;
                    }
                    let owner = self.prompt_owner(owner, owner_key)?;
                    request = Some(NativeRequest {
                        controlled: None,
                        id: RequestToken::Generated(0),
                        owner,
                        prompt,
                        movement_range: None,
                        pre_action_range: None,
                        choices: RequestChoices::Numbers {
                            values: options,
                            labels,
                        },
                        can_skip: !mandatory,
                        discard,
                    });
                }
                Instruction::Hex {
                    output,
                    filters,
                    prompt,
                    auto,
                    owner,
                    owner_key,
                    discard,
                } => {
                    let candidates = self.world.tile_hexes();
                    let mut accepted = vec![true; candidates.len()];
                    let context = SpatialContext {
                        count_observer: Some(&self.count_observer),
                        world: &self.world,
                        registers: &self.context,
                        symbols: &self.symbols,
                        actor: self.planning.actor,
                        frame: frame.unit_override,
                        coordinates: self.coordinates,
                        range_override: frame.range_override,
                        through_override: frame.through_override,
                    };
                    for filter in filters {
                        filter
                            .apply_scan(&context, candidates, &mut accepted)
                            .map_err(unsupported)?;
                    }
                    let choices = candidates
                        .iter()
                        .zip(accepted)
                        .filter_map(|(hex, accepted)| accepted.then_some(*hex))
                        .collect::<Vec<_>>();
                    self.cleanup_count_origin();
                    if choices.is_empty() {
                        if mandatory {
                            if self
                                .context
                                .get(self.action_stack_key)
                                .is_some_and(|v| v.truthy(&self.symbols))
                            {
                                return Err(unsupported(
                                    "action-context abort restoration not compiled",
                                ));
                            }
                            self.abort_action();
                        } else {
                            self.context.remove(output);
                        }
                        continue;
                    }
                    if auto && choices.len() == 1 && mandatory {
                        self.context
                            .set(output, native_hex(choices[0], self.coordinates));
                        continue;
                    }
                    if selected == Some(Datum::Text(self.skip)) && !mandatory {
                        self.context.remove(output);
                        continue;
                    }
                    if let Some(hex) = selected
                        .as_ref()
                        .and_then(|v| v.hex(self.coordinates, &self.symbols).ok())
                        && choices.contains(&hex)
                    {
                        self.context.set(output, native_hex(hex, self.coordinates));
                        continue;
                    }
                    request = Some(NativeRequest {
                        controlled: None,
                        id: RequestToken::Generated(0),
                        owner: self.prompt_owner(owner, owner_key)?,
                        prompt,
                        movement_range: frame.range_override,
                        pre_action_range: frame.pre_move.map(|(_, range)| range),
                        choices: RequestChoices::Hexes(choices),
                        can_skip: !mandatory,
                        discard,
                    });
                }
                Instruction::TravelSequence {
                    unit,
                    destination,
                    select,
                    travel,
                } => {
                    let Some(unit) = unit
                        .or(self.planning.actor)
                        .map(|id| self.world.resolved_actor(id))
                    else {
                        continue;
                    };
                    self.world
                        .ensure_fast_travel_closure()
                        .map_err(unsupported)?;
                    let source_card = match &frame.action {
                        Some(ActionStep::FastTravel { card, .. }) => Some(*card),
                        _ => None,
                    };
                    let binding = source_card.map(|card| EffectBinding {
                        card,
                        context: None,
                        range: None,
                        primary: None,
                    });
                    let mut travel = self.generated_frame(travel, Some(unit), None)?;
                    travel.effect = binding.clone();
                    self.stack.push(travel);
                    if !self
                        .context
                        .get(destination)
                        .is_some_and(|v| v.truthy(&self.symbols))
                    {
                        let mut selection = self.generated_frame(select, Some(unit), None)?;
                        selection.effect = binding;
                        self.stack.push(selection);
                    }
                }
                Instruction::FastTravel {
                    unit,
                    destination,
                    destination_key,
                    change,
                } => {
                    let unit = frame
                        .unit_override
                        .unwrap_or(unit)
                        .or(self.planning.actor)
                        .map(|id| self.world.resolved_actor(id));
                    let value = destination
                        .map(|hex| native_hex(hex, self.coordinates))
                        .or_else(|| self.context.get(destination_key).cloned())
                        .filter(|v| v.truthy(&self.symbols));
                    let (Some(unit), Some(value)) = (unit, value) else {
                        continue;
                    };
                    if value == Datum::Text(self.skip) {
                        continue;
                    }
                    let to = if matches!(value, Datum::Object(_)) {
                        Some(datum_hex(&value, &self.symbols, self.coordinates)?)
                    } else {
                        None
                    };
                    let valid = if let Some(to) = to {
                        self.world
                            .fast_travel_destinations(Some(unit), change, &[to])
                            .map_err(unsupported)?[0]
                    } else {
                        false
                    };
                    if !valid {
                        if mandatory {
                            self.abort_action();
                        }
                        continue;
                    }
                    let to = to.unwrap();
                    let from = self.world.position(unit);
                    self.world
                        .move_entity(unit, to)
                        .map_err(EngineError::Rule)?;
                    result
                        .events
                        .push(NativeEvent::UnitPlaced { unit, from, to });
                }
                Instruction::MovementSequence {
                    unit,
                    destination,
                    select,
                    movement,
                    range,
                    stat,
                } => {
                    self.world.ensure_move_closure(true).map_err(unsupported)?;
                    let unit = unit
                        .or(self.planning.actor)
                        .map(|id| self.world.resolved_actor(id));
                    let range = if let (Some(unit), Some(stat)) = (unit, stat) {
                        i32::try_from(
                            self.world
                                .computed_stat_in_context(
                                    unit,
                                    stat,
                                    i64::from(range),
                                    &self.context,
                                )
                                .map_err(unsupported)?,
                        )
                        .map_err(|_| unsupported("computed movement range outside i32"))?
                    } else {
                        range
                    };
                    let range = if let Some(unit) = unit {
                        i32::try_from(
                            self.world
                                .movement_action_range(unit, i64::from(range))
                                .map_err(unsupported)?,
                        )
                        .map_err(|_| unsupported("capped movement range outside i32"))?
                    } else {
                        range
                    };
                    let through = if let Some(unit) = unit {
                        (self.world.movement_aura_passes(unit)
                            || self.world.movement_zone_passes(unit).map_err(unsupported)?)
                        .then_some(true)
                    } else {
                        None
                    };
                    let mut movement = self.generated_frame(movement, unit, Some(range))?;
                    movement.through_override = through;
                    self.stack.push(movement);
                    if !self
                        .context
                        .get(destination)
                        .is_some_and(|v| v.truthy(&self.symbols))
                    {
                        let mut selection = self.generated_frame(select, unit, Some(range))?;
                        selection.through_override = through;
                        self.stack.push(selection);
                    }
                    continue;
                }
                Instruction::Confirm {
                    output,
                    prompt,
                    owner,
                } => {
                    let Some(owner) = owner.or(self.planning.actor) else {
                        continue;
                    };
                    if frame
                        .pending
                        .as_ref()
                        .is_some_and(|v| v.truthy(&self.symbols))
                    {
                        self.context
                            .set(output, Datum::Bool(selected == Some(Datum::Text(self.yes))));
                        continue;
                    }
                    request = Some(NativeRequest {
                        controlled: None,
                        id: RequestToken::Generated(0),
                        owner: self.normalized_owner(owner),
                        prompt,
                        movement_range: None,
                        pre_action_range: None,
                        choices: RequestChoices::YesNo,
                        can_skip: false,
                        discard: false,
                    });
                }
                Instruction::Move {
                    mine_choice,
                    mine_trigger,
                    prechosen,
                    unit,
                    unit_key,
                    destination,
                    destination_key,
                    range,
                    through,
                    straight,
                    crossed,
                    success,
                    is_action,
                } => {
                    self.world
                        .ensure_move_closure(is_action)
                        .map_err(unsupported)?;
                    if let Some(key) = success {
                        self.context.remove(key);
                    }
                    let range = frame.range_override.unwrap_or(range);
                    let through = frame.through_override.unwrap_or(through);
                    let unit = frame.unit_override.unwrap_or(unit);
                    let target = unit
                        .or(if let Some(key) = unit_key {
                            self.symbol_from_context(key)?
                        } else {
                            None
                        })
                        .or(self.planning.actor);
                    let Some(unit) = target else {
                        continue;
                    };
                    let destination = if let Some(hex) = destination {
                        Some(hex)
                    } else {
                        self.context
                            .get(destination_key)
                            .filter(|v| v.truthy(&self.symbols))
                            .map(|v| datum_hex(v, &self.symbols, self.coordinates))
                            .transpose()?
                    };
                    let Some(to) = destination else {
                        continue;
                    };
                    let Some(from) = self.world.position(unit) else {
                        continue;
                    };
                    if self.world.entity(unit).is_none() {
                        if mandatory {
                            self.abort_action();
                        }
                        continue;
                    }
                    let actor = self
                        .planning
                        .actor
                        .filter(|id| *id != self.world.definitions.effect_empty)
                        .unwrap_or(unit);
                    if !self
                        .world
                        .can_displace(
                            unit,
                            Some(actor),
                            crate::validation::DisplacementKind::Move,
                            None,
                        )
                        .map_err(unsupported)?
                        .allowed
                    {
                        if mandatory {
                            self.abort_action();
                        }
                        continue;
                    }
                    let distance = if self
                        .world
                        .effect_connected(from, to, &[unit])
                        .map_err(unsupported)?
                    {
                        i64::from(from.distance(to))
                    } else {
                        0
                    };
                    if !self
                        .world
                        .movement_legality(unit, distance, is_action, None, &self.context)
                        .map_err(unsupported)?
                        .allowed
                    {
                        if mandatory {
                            self.abort_action();
                        }
                        continue;
                    }
                    let valid = if from == to {
                        range >= 0
                    } else {
                        self.world
                            .movement_paths(
                                &crate::geometry::MovementQuery {
                                    start: from,
                                    ends: vec![to],
                                    max_steps: range,
                                    ignore_obstacles: false,
                                    pass_through_obstacles: through,
                                },
                                self.planning.actor,
                                Some(&[unit]),
                                &[],
                            )
                            .map_err(unsupported)?[0]
                    };
                    if !valid {
                        continue;
                    }
                    let line = if straight {
                        let Some(line) = from.line_to(to) else {
                            continue;
                        };
                        if !self
                            .world
                            .effect_connected(from, to, &[])
                            .map_err(unsupported)?
                        {
                            continue;
                        }
                        let mut blocked = false;
                        for hex in line.iter().take(line.len().saturating_sub(1)) {
                            if !self.world.on_map(*hex)
                                || (!through
                                    && self
                                        .world
                                        .obstacle_for_actor(*hex, Some(unit))
                                        .map_err(unsupported)?
                                    && !self.world.passable_token(*hex, &[unit]))
                            {
                                blocked = true;
                                break;
                            }
                        }
                        if blocked {
                            continue;
                        }
                        line
                    } else {
                        Vec::new()
                    };
                    let ids_key = self.symbols.lookup("triggered_mine_ids");
                    let mut triggered = Vec::new();
                    if prechosen || frame.mine_prechosen {
                        if let Some(key) = ids_key {
                            triggered = match self.context.get(key) {
                                None => Vec::new(),
                                Some(Datum::List(ids)) => ids
                                    .iter()
                                    .map(|v| {
                                        if let Datum::Text(id) = v {
                                            Ok(*id)
                                        } else {
                                            Err(unsupported("non-string prechosen mine identity"))
                                        }
                                    })
                                    .collect::<Result<Vec<_>>>()?,
                                _ => {
                                    return Err(unsupported(
                                        "prechosen mine queue must be a native list",
                                    ));
                                }
                            };
                        }
                    } else if let Some(team) = self
                        .world
                        .entity(unit)
                        .filter(|e| matches!(e.kind, EntityKind::Hero | EntityKind::Piece))
                        .and_then(|e| e.team)
                        .filter(|team| from != to && self.world.has_enemy_mine_supply(*team))
                    {
                        if straight {
                            for hex in line.iter().take(line.len().saturating_sub(1)) {
                                if !self.world.passable_token(*hex, &[]) {
                                    continue;
                                }
                                if let Some(entity) = self
                                    .world
                                    .occupant(*hex)
                                    .and_then(|id| self.world.entity(id))
                                    .filter(|e| e.kind == EntityKind::Token)
                                    && let Some(owner) =
                                        entity.owner.and_then(|id| self.world.hero(id))
                                    && owner.team != team
                                {
                                    triggered.push(entity.id);
                                }
                            }
                        } else {
                            let paths = self
                                .world
                                .mine_paths(from, range, self.planning.actor, Some(team), &[unit])
                                .map_err(unsupported)?;
                            let options = self
                                .world
                                .tile_index(to)
                                .map(|i| paths[i].as_slice())
                                .unwrap_or(&[]);
                            if options.len() > 1 {
                                let choice = mine_choice.ok_or_else(|| {
                                    unsupported(
                                        "newly deployed mines require movement recompilation",
                                    )
                                })?;
                                frame.mine_prechosen = true;
                                // The Python model copy is a new runtime frame. Imported
                                // fixture labels remain explicit; generated copies receive
                                // their own ephemeral identity, preserved across restore.
                                if frame.generated.is_some()
                                    || self.steps[frame.instruction].wire["step_id"]
                                        .as_str()
                                        .is_some_and(|id| id.starts_with("native-step-"))
                                {
                                    self.next_step =
                                        self.next_step.checked_add(1).ok_or_else(|| {
                                            unsupported("step generation counter exhausted")
                                        })?;
                                    frame.generated = Some(self.next_step);
                                }
                                self.stack.push(frame.clone());
                                let choice =
                                    self.generated_frame(choice, Some(unit), Some(range))?;
                                self.stack.push(choice);
                                continue;
                            }
                            if let Some(path) = options.first() {
                                triggered = path.mines.to_vec();
                            }
                        }
                    }
                    // Resolve all unsupported closure checks before changing location.
                    let trigger = if triggered.is_empty() {
                        None
                    } else {
                        let trigger = mine_trigger.ok_or_else(|| {
                            unsupported("newly deployed mines require movement recompilation")
                        })?;
                        if let Instruction::TriggerMines { blast, discard, .. } =
                            &self.steps[trigger].instruction
                            && triggered.iter().any(|id| {
                                self.world
                                    .entity(*id)
                                    .is_some_and(|e| e.token_type == Some(*blast))
                            })
                            && discard.is_none()
                        {
                            return Err(unsupported(
                                "blast mine discard recipe requires recompilation",
                            ));
                        }
                        Some(trigger)
                    };
                    if let Some(key) = crossed {
                        let mut hexes = Vec::new();
                        for hex in line.iter().take(line.len().saturating_sub(1)) {
                            if self
                                .world
                                .obstacle_for_actor(*hex, Some(actor))
                                .map_err(unsupported)?
                            {
                                hexes.push(native_hex(*hex, self.coordinates));
                            }
                        }
                        self.context.set(key, Datum::List(hexes));
                    }
                    self.world
                        .move_entity(unit, to)
                        .map_err(EngineError::Rule)?;
                    if let Some(key) = success {
                        self.context.set(key, Datum::Bool(true));
                    }
                    result.events.push(NativeEvent::UnitMoved {
                        unit,
                        from,
                        to,
                        range,
                    });
                    if let Some(trigger) = trigger {
                        let ids_key =
                            ids_key.ok_or_else(|| unsupported("mine context key not compiled"))?;
                        let victim_key = self
                            .symbols
                            .lookup("mine_victim_id")
                            .ok_or_else(|| unsupported("mine victim key not compiled"))?;
                        self.context.set(
                            ids_key,
                            Datum::List(triggered.iter().copied().map(Datum::Text).collect()),
                        );
                        self.context.set(victim_key, Datum::Text(unit));
                        let mut child = self.generated_frame(trigger, None, None)?;
                        child.mine_snapshot = Some((triggered, unit));
                        self.stack.push(child);
                    }
                }
                Instruction::Query {
                    spec,
                    collect,
                    output,
                } => {
                    let spatial = SpatialContext {
                        count_observer: Some(&self.count_observer),
                        world: &self.world,
                        registers: &self.context,
                        symbols: &self.symbols,
                        actor: self.planning.actor,
                        frame: frame.unit_override,
                        coordinates: self.coordinates,
                        range_override: None,
                        through_override: None,
                    };
                    let value = spec.evaluate(&spatial, collect).map_err(unsupported)?;
                    self.cleanup_count_origin();
                    self.context.set(output, value);
                }
                Instruction::ContextCondition {
                    input,
                    comparison,
                    threshold,
                    output,
                } => {
                    let value = comparison
                        .evaluate(self.context.get(input), threshold)
                        .map_err(unsupported)?;
                    self.context.set(
                        output,
                        if value {
                            Datum::Bool(true)
                        } else {
                            Datum::Null
                        },
                    );
                }
                Instruction::ValidateRepeat {
                    actor,
                    and_key,
                    output,
                } => {
                    let allowed = self
                        .world
                        .can_repeat_action(actor.or(self.planning.actor))
                        .map_err(unsupported)?;
                    let value = if allowed {
                        if let Some(key) = and_key {
                            let value =
                                self.context.get(key).cloned().unwrap_or(Datum::Bool(false));
                            if matches!(value, Datum::List(_) | Datum::Object(_)) {
                                return Err(unsupported(
                                    "repeat validation aggregate alias requires the context arena",
                                ));
                            }
                            value
                        } else {
                            Datum::Bool(true)
                        }
                    } else {
                        Datum::Bool(false)
                    };
                    self.context.set(output, value);
                }
                Instruction::Repeat {
                    max,
                    prompt,
                    templates,
                    check,
                } => {
                    if frame.index >= max {
                        continue;
                    }
                    let Some(actor) = self.planning.actor else {
                        continue;
                    };
                    if !self.world.battle_zones.is_empty() && !frame.repeat_checked {
                        frame.repeat_checked = true;
                        self.stack.push(frame);
                        let child = self.generated_frame(check, None, None)?;
                        self.stack.push(child);
                        continue;
                    }
                    if !self
                        .world
                        .can_repeat_action(Some(actor))
                        .map_err(unsupported)?
                    {
                        continue;
                    }
                    if let Some(selection) = selected {
                        frame.pending = None;
                        if selection != Datum::Text(self.yes) {
                            continue;
                        }
                        frame.index = frame
                            .index
                            .checked_add(1)
                            .ok_or_else(|| unsupported("repeat count overflow"))?;
                        let effect = frame.effect.clone();
                        if frame.index < max {
                            frame.repeat_checked = false;
                            self.stack.push(frame);
                        }
                        self.push_template_copies(&templates, effect)?;
                        continue;
                    }
                    request = Some(NativeRequest {
                        controlled: None,
                        id: RequestToken::Generated(0),
                        owner: actor,
                        prompt,
                        movement_range: None,
                        pre_action_range: None,
                        choices: RequestChoices::Repeat {
                            done: frame.index,
                            max,
                        },
                        can_skip: false,
                        discard: false,
                    });
                }
                Instruction::ForEach {
                    list,
                    item,
                    templates,
                } => {
                    let values = match self.context.get(list) {
                        None => &[][..],
                        Some(Datum::List(items)) => items.as_slice(),
                        _ => return Err(unsupported("ForEach source requires a native list")),
                    };
                    if frame.index >= values.len() as i64 {
                        continue;
                    }
                    let index = if frame.index < 0 {
                        values.len() as i64 + frame.index
                    } else {
                        frame.index
                    };
                    let value =
                        values
                            .get(usize::try_from(index).map_err(|_| {
                                EngineError::Rule("ForEach index outside list".into())
                            })?)
                            .ok_or_else(|| EngineError::Rule("ForEach index outside list".into()))?
                            .clone();
                    let keep = frame.index + 1 < values.len() as i64;
                    let effect = frame.effect.clone();
                    if matches!(value, Datum::List(_) | Datum::Object(_)) {
                        return Err(unsupported(
                            "nested aggregate ForEach aliases require the context arena",
                        ));
                    }
                    self.context.set(item, value);
                    frame.index += 1;
                    if keep {
                        self.stack.push(frame);
                    }
                    self.push_template_copies(&templates, effect)?;
                    continue;
                }
            }
            if let Some(request) = request {
                let request = self.issue(&mut frame, request)?;
                self.stack.push(frame);
                self.last_request = Some(request.clone());
                result.request = Some(request);
                return Ok(result);
            }
        }
        Ok(result)
    }
    fn finish_planning_command(
        &mut self,
        result: std::result::Result<(), crate::cards::CardError>,
    ) -> Result<()> {
        result.map_err(|e| match e {
            crate::cards::CardError::Illegal(s) => EngineError::InvalidInput(s.into()),
            crate::cards::CardError::Unsupported(s) => unsupported(s),
        })?;
        if self.planning.take_end_turn() {
            self.advance_turn(true)?;
        }
        self.schedule_continuations()
    }
    fn ensure_planning_effect_closure(&self) -> Result<()> {
        if self.objects.is_some() {
            return Err(unsupported(
                "shared-object planning continuation not yet bound",
            ));
        }
        if self.world.has_effects {
            return Err(unsupported("planning active-effect closure not compiled"));
        }
        if self.planning.phase == Phase::Planning
            && self.flow_program.as_ref().is_none_or(|program| {
                !program
                    .entries
                    .iter()
                    .any(|(name, _)| *name == "find_actor")
            })
        {
            return Err(unsupported(
                "planning continuation closure not compiled; restore the saved boundary",
            ));
        }
        Ok(())
    }
    pub fn commit_card(&mut self, hero: Symbol, card: crate::cards::CardId) -> Result<()> {
        if self.objects.is_some() {
            return self.with_live_objects(|session, objects| {
                session.commit_card_bound(objects, hero, card)
            });
        }
        self.ensure_planning_effect_closure()?;
        let result = self.planning.commit(&mut self.world, hero, card);
        self.finish_planning_command(result)
    }
    pub fn uncommit_card(&mut self, hero: Symbol) -> Result<()> {
        if self.objects.is_some() {
            return self
                .with_live_objects(|session, objects| session.uncommit_card_bound(objects, hero));
        }
        self.ensure_planning_effect_closure()?;
        let result = self.planning.uncommit(&mut self.world, hero);
        self.finish_planning_command(result)
    }
    pub fn pass_planning(&mut self, hero: Symbol) -> Result<()> {
        if self.objects.is_some() {
            return self
                .with_live_objects(|session, objects| session.pass_planning_bound(objects, hero));
        }
        self.ensure_planning_effect_closure()?;
        let result = self.planning.pass(&mut self.world, hero);
        self.finish_planning_command(result)
    }
    pub fn finish_planning(&mut self, hero: Symbol) -> Result<()> {
        if self.objects.is_some() {
            return self.with_live_objects(|session, objects| {
                session.finish_planning_bound(objects, hero)
            });
        }
        self.ensure_planning_effect_closure()?;
        let result = self.planning.finish(&mut self.world, hero);
        self.finish_planning_command(result)
    }
    pub fn submit(&mut self, request: RequestToken, selection: Datum) -> Result<()> {
        if self.objects.is_some() {
            return self.submit_objects(request, selection);
        }
        let frame = self
            .stack
            .last_mut()
            .ok_or_else(|| EngineError::InvalidInput("No pending step to receive input".into()))?;
        if frame.request != Some(request) {
            return Err(EngineError::InvalidInput(
                "stale or unissued input request".into(),
            ));
        }
        if matches!(
            self.steps[frame.instruction].instruction,
            Instruction::Flow(crate::turn::FlowStep::Upgrades)
        ) {
            let rules = self
                .round_rules
                .as_ref()
                .ok_or_else(|| unsupported("round rules missing"))?;
            if rules
                .validate(&self.world, &self.round, &selection)
                .map_err(card_error)?
                .is_none()
            {
                return Err(EngineError::InvalidInput(
                    "Choose one legal upgrade or all remaining upgrades for your hero".into(),
                ));
            }
        }
        frame.pending = Some(Datum::Object(vec![(self.selection_key, selection)]));
        frame.request = None;
        Ok(())
    }
    pub fn selection_json(&self, selection: &Datum) -> Value {
        selection.to_json(&self.symbols)
    }

    pub fn import_selection(&mut self, value: &Value) -> Result<Datum> {
        let value =
            Datum::from_json(value, Arc::make_mut(&mut self.symbols)).map_err(unsupported)?;
        if let Some(objects) = &mut self.objects {
            objects.arena.symbols = self.symbols.clone();
        }
        Ok(value)
    }
    pub fn request_token(&self, token: RequestToken) -> String {
        match token {
            RequestToken::Imported(id) => self.symbols.name(id).into(),
            RequestToken::Generated(id) => format!("native-request-{id}"),
        }
    }
    pub fn import_request_token(&mut self, value: &str) -> RequestToken {
        if let Some(token) = self.stack.last().and_then(|frame| frame.request)
            && self.request_token(token) == value
        {
            return token;
        }
        if let Some(id) = value
            .strip_prefix("native-request-")
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|id| value == format!("native-request-{id}"))
        {
            RequestToken::Generated(id)
        } else {
            let id = Arc::make_mut(&mut self.symbols).intern(value);
            if let Some(objects) = &mut self.objects {
                objects.arena.symbols = self.symbols.clone();
            }
            RequestToken::Imported(id)
        }
    }
    /// Trace-only model identities; never part of rule state or persisted IDs.
    pub fn generated_step_objects(
        &self,
        bindings: &[crate::codec::ObjectBinding],
    ) -> Result<Vec<(usize, usize)>> {
        self.objects
            .as_ref()
            .ok_or_else(|| unsupported("trace object arena missing"))?
            .arena
            .generated_step_objects(bindings)
            .map_err(unsupported)
    }
    /// Compact outer boundary preserves physical instance aliases on restore.
    pub fn export_envelope(&self) -> Result<StateEnvelope> {
        let mut source = if let Some(objects) = &self.objects {
            objects.arena.boundary(&self.source).map_err(unsupported)?
        } else {
            (*self.source).clone()
        };
        source.native_resume = Some(crate::codec::NativeResume {
            version: crate::versions::NATIVE_RESUME,
            request_high_water: self.next_request,
            step_high_water: self.next_step,
        });
        if self.objects.is_some() {
            return Ok(source);
        }
        self.planning
            .export_fields(&mut source, &self.symbols)
            .map_err(unsupported)?;
        let state = source.state.as_object_mut().unwrap();
        state.insert(
            "execution_context".into(),
            self.context.to_json(&self.symbols),
        );
        state.insert(
            "tie_breaker_team".into(),
            json!(if self.team == Team::Red {
                "RED"
            } else {
                "BLUE"
            }),
        );
        state.insert(
            "execution_stack".into(),
            Value::Array(
                self.stack
                    .iter()
                    .map(|frame| {
                        let mut wire = self.steps[frame.instruction].wire.clone();
                        if let Some(values) = frame.effect.as_ref().and_then(|e| e.primary.as_ref())
                        {
                            values.patch_wire(&mut wire, &self.symbols, &self.primary_operands);
                            if matches!(&self.steps[frame.instruction].instruction,Instruction::Attack{spec,..} if spec.card_ranged){wire["is_ranged"]=json!(values.ranged);}
                        }
                        if let Some(cont)=&frame.continuation {patch_continuation(&mut wire,cont,&self.symbols);}
                        if let Some(battle)=&frame.battle {battle.patch_wire(&mut wire,&self.symbols);}
                        if let Some(binding) = &frame.protection {
                            wire["minion_id"] = json!(self.symbols.name(binding.minion));
                            wire["killer_id"] = binding.killer.map_or(Value::Null, |id| json!(self.symbols.name(id)));
                            wire["tried_effect_ids"] = json!(binding.tried.iter().map(|id| self.symbols.name(*id)).collect::<Vec<_>>());
                        }
                        if frame.mine_prechosen { wire["mine_path_prechosen"] = json!(true); }
                        if let Some((ids, victim)) = &frame.mine_snapshot {
                            wire["mine_ids"] = json!(ids.iter().map(|id| self.symbols.name(*id)).collect::<Vec<_>>());
                            wire["victim_id"] = json!(self.symbols.name(*victim));
                        }
                        if let Some((victim,key,source))=frame.mine_trigger {
                wire["victim_id"]=victim.map(|id|self.symbols.name(id)).map_or(Value::Null,|id|json!(id));
                wire["victim_key"]=json!(self.symbols.name(key));
                wire["immunity_source_id"]=source.map(|id|self.symbols.name(id)).map_or(Value::Null,|id|json!(id));
            }
            if let Some(binding) = &frame.zone_return {
                            match binding {
                                crate::minion_return::ReturnBinding::Processed(ids) => {
                                    wire["processed_minion_ids"] = json!(ids.iter().map(|id| self.symbols.name(*id)).collect::<Vec<_>>());
                                }
                                crate::minion_return::ReturnBinding::Relocate { unit, destination, range } => {
                                    wire["unit_id"] = json!(self.symbols.name(*unit));
                                    wire["target_hex_arg"] = json!(destination);
                                    if let Some(range) = range { wire["range_val"] = json!(range); }
                                }
                            }
                        }
                        if let Some(action) = &frame.action {
                            action.patch_wire(&mut wire, &self.symbols);
                        }
                        if let Some(id) = frame.generated {
                            wire["step_id"] = json!(format!("native-step-{id}"));
                            if let Some(templates) =
                                wire.get_mut("steps_template").and_then(Value::as_array_mut)
                            {
                                for (index, template) in templates.iter_mut().enumerate() {
                                    template["step_id"] =
                                        json!(format!("native-step-{id}-template-{index}"));
                                }
                            }
                        }
                        if let Some(unit) = frame.unit_override {
                            let value = unit.map_or(Value::Null, |id| json!(self.symbols.name(id)));
                            if wire["type"] == "move_unit"
                                || wire["type"] == "mine_path_choice"
                                || wire["type"] == "fast_travel_unit"
                                || wire["type"] == "remove_unit"
                            {
                                wire["unit_id"] = value;
                            } else if wire["type"] == "select_step"
                                && let Some(filters) = wire["filters"].as_array_mut()
                            {
                                for filter in filters {
                                    let key = match filter["type"].as_str() {
                                        Some("occupied_filter") => Some("exclude_id"),
                                        Some(
                                            "movement_path_filter"
                                            | "fast_travel_destination_filter",
                                        ) => Some("unit_id"),
                                        Some(
                                            "in_straight_line_filter" | "straight_line_path_filter",
                                        ) => Some("origin_id"),
                                        _ => None,
                                    };
                                    if let Some(key) = key {
                                        filter[key] = value.clone();
                                    }
                                }
                            }
                        }
                        if let Some(offer) = frame.offer_override {
                            wire["active_effect_id"] = offer.provider.map_or(Value::Null, |id| json!(self.symbols.name(id)));
                            wire["card_id"] = json!(self.symbols.name(offer.card));
                            if wire["type"] == "offer_passive" {
                                wire["trigger"] = json!(self.symbols.name(offer.trigger));
                                wire["is_optional"] = json!(offer.optional);
                                wire["prompt"] = json!(self.symbols.name(offer.prompt));
                                wire["hero_id"] = offer.hero.map_or(Value::Null, |h| json!(self.symbols.name(h)));
                            }
                        }
                        if let Some(prompt) = frame.defeat_prompt_victim {
                            wire["prompt"] = json!(format!("{}, select a card to discard (or be Defeated).", self.symbols.name(prompt)));
                        }
                        if let Some(range) = frame.range_override {
                            if wire["type"] == "move_unit" || wire["type"] == "mine_path_choice" {
                                wire["range_val"] = json!(range);
                            } else if wire["type"] == "select_step" {
                                wire["prompt"] =
                                    json!(format!("Select Movement Destination (Range {range})"));
                                if let Some(filters) = wire["filters"].as_array_mut() {
                                    for filter in filters {
                                        if filter["type"] == "movement_path_filter" {
                                            filter["range_val"] = json!(range);
                                        }
                                        if filter["type"] == "range_filter" {
                                            filter["min_range"] = json!(range);
                                            filter["max_range"] = json!(range);
                                        }
                                    }
                                }
                            }
                        }
                        if let Some((hero,range)) = frame.pre_move {
                            if wire["type"] == "move_unit" {
                                wire["unit_id"] = json!(self.symbols.name(hero)); wire["range_val"] = json!(range);
                            } else if wire["type"] == "select_step" {
                                wire["prompt"] = json!(format!("Pre-action movement (up to {range} space{})", if range > 1 { "s" } else { "" }));
                                for filter in wire["filters"].as_array_mut().expect("compiled pre-action movement filters") {
                                    if filter["type"] == "occupied_filter" { filter["exclude_id"] = json!(self.symbols.name(hero)); }
                                    if filter["type"] == "movement_path_filter" { filter["unit_id"] = json!(self.symbols.name(hero)); filter["range_val"] = json!(range); }
                                }
                            }
                        }
                        if wire["type"] == "resolve_pre_action_discard" {
                            wire["processed_effect_ids"] = json!(frame.pre_processed.iter().map(|id| self.symbols.name(*id)).collect::<Vec<_>>());
                        }
                        if let Some(forced) = frame.forced.as_ref() {
                            if wire["type"] == "select_step" {
                                wire["context_hero_id"] = json!(self.symbols.name(forced.victim));
                                wire["override_player_id"] =
                                    json!(self.symbols.name(forced.victim));
                                wire["card_is_basic"] = json!(forced.basic);
                                if forced.shields {
                                    wire["card_containers"] = json!(["HAND", "PLAYED"]);
                                    wire["restrict_played_to_shields"] = json!(true);
                                    wire["card_is_basic"] = Value::Null;
                                    wire["allowed_card_ids"] = forced.allowed.as_ref().map_or(Value::Null, |ids| json!(ids.iter().map(|id| self.symbols.name(*id)).collect::<Vec<_>>()));
                                }
                                wire["prompt"] = json!(format!(
                                    "{}, select a card to discard.",
                                    self.symbols.name(forced.victim)
                                ));
                            } else if wire["type"] == "discard_card" {
                                wire["hero_id"] = json!(self.symbols.name(forced.victim));
                            }
                        }
                        if let Some(through) = frame.through_override {
                            if wire["type"] == "move_unit" {wire["pass_through_obstacles"] = json!(through);}
                            if let Some(filters) = wire.get_mut("filters").and_then(Value::as_array_mut) {
                                for filter in filters {
                                    if filter.get("pass_through_obstacles").is_some() {filter["pass_through_obstacles"] = json!(through);}
                                }
                            }
                        }
                        if let Some(effect) = &frame.effect {
                            if effect.primary.is_none() && wire.get("source_card_id").is_some() {
                                wire["source_card_id"] = json!(self.symbols.name(effect.card));
                            }
                            if let Some(filters) = wire
                                .get_mut("filters")
                                .and_then(Value::as_array_mut)
                                .filter(|_| effect.primary.is_none())
                            {
                                for filter in filters {
                                    if filter.get("source_card_id").is_some() {
                                        filter["source_card_id"] =
                                            json!(self.symbols.name(effect.card));
                                    }
                                }
                            }
                            if let Some(range) = effect.range
                                && let Some(filters) = wire["filters"].as_array_mut()
                            {
                                for filter in filters {
                                    if filter["type"] == "range_filter" {
                                        filter["max_range"] = json!(range);
                                    }
                                }
                            }
                            if let Some((key, value)) = &effect.context {
                                wire["key"] = json!(self.symbols.name(*key));
                                wire["value"] = value.to_json(&self.symbols);
                            }
                        }
                        if wire["type"] == "place_marker"
                            && let Some(Some(target)) = frame.unit_override
                        {
                            wire["target_id"] = json!(self.symbols.name(target));
                        }
                        if let Some(key) = frame.attack_followup {
                            wire["target_id_key"] = json!(self.symbols.name(key));
                            wire["target_output_key"] = Value::Null;
                            wire["active_if_key"] = json!(self.symbols.name(key));
                        }
                        if let Some(binding) = frame.attack_binding {
                            match wire["type"].as_str() {
                                Some("select_step") => {
                                    wire["output_key"] = json!(self.symbols.name(binding.key));
                                    wire["filters"][0]["max_range"] = json!(binding.range);
                                }
                                Some("reaction_window") => {
                                    wire["target_player_key"] =
                                        json!(self.symbols.name(binding.key))
                                }
                                Some("resolve_combat") => {
                                    wire["target_key"] = json!(self.symbols.name(binding.key));
                                    wire["damage"] = json!(binding.damage);
                                }
                                _ => {}
                            }
                        }
                        if let Some((hero, card)) = frame.discard_override {
                            wire["hero_id"] = json!(self.symbols.name(hero));
                            wire["card_id"] = json!(self.symbols.name(card));
                        }
                        if wire["type"] == "check_passive_abilities"
                            && let Some(Some(hero)) = frame.unit_override
                        {
                            wire["hero_id"] = json!(self.symbols.name(hero));
                        }
                        if let Some(binding) = frame.defeat_override {
                            wire["victim_id"] = json!(self.symbols.name(binding.victim));
                            wire["killer_id"] =
                                json!(binding.killer.map(|id| self.symbols.name(id)));
                        }
                        if let Some(outcome) = frame.outcome_override {
                            wire["winner"] = json!(outcome.team.map(|id| self.symbols.name(id)));
                            wire["individual_winner_id"] =
                                json!(outcome.hero.map(|id| self.symbols.name(id)));
                            wire["condition"] = json!(self.symbols.name(outcome.condition));
                        }
                        wire["pending_input"] = frame
                            .pending
                            .as_ref()
                            .map_or(Value::Null, |value| value.to_json(&self.symbols));
                        wire["pending_request_id"] = frame
                            .request
                            .map_or(Value::Null, |id| Value::from(self.request_token(id)));
                        if matches!(
                            self.steps[frame.instruction].instruction,
                            Instruction::Multi { .. }
                        ) {
                            wire["selections"] = json!(
                                frame
                                    .multi
                                    .iter()
                                    .map(|id| self.symbols.name(*id))
                                    .collect::<Vec<_>>()
                            );
                        }
                        if matches!(self.steps[frame.instruction].instruction, Instruction::Repeat { .. }) {
                            wire["repeats_done"] = json!(frame.index);
                            wire["lane_push_checked"] = json!(frame.repeat_checked);
                        }
                        if matches!(
                            self.steps[frame.instruction].instruction,
                            Instruction::ForEach { .. }
                        ) {
                            wire["current_index"] = json!(frame.index);
                        }
                        wire
                    })
                    .collect(),
            ),
        );
        self.boundary.export(&mut source.state, &self.symbols);
        self.round.export(&mut source.state, &self.symbols);
        self.world.export_combat(&mut source.state, &self.symbols);
        self.world.export_spatial(&mut source.state, &self.symbols);
        self.world.export_cards(&mut source, &self.symbols);
        Ok(source)
    }
    /// Root-relative rollback diagnostic for execution state and arena payloads.
    pub fn execution_fingerprint(&self) -> u128 {
        use std::hash::{DefaultHasher, Hash, Hasher};

        let lane = |salt| {
            let mut hash = DefaultHasher::new();
            (
                "execution",
                salt,
                &self.stack,
                &self.context,
                &self.last_request,
                self.next_request,
                self.next_step,
                (&self.planning, &self.boundary, &self.round),
                self.team,
                self.sampled_guess_steps,
            )
                .hash(&mut hash);
            self.symbols.hash_local(&mut hash);
            self.objects.is_some().hash(&mut hash);
            if let Some(objects) = &self.objects {
                objects.hash_execution(&mut hash);
            }
            hash.finish()
        };
        u128::from(lane(0u64)) | (u128::from(lane(1u64)) << 64)
    }

    /// Rollback diagnostic for spatial state, hero and card rows, effects, and world counters.
    pub fn world_state_fingerprint(&mut self) -> u128 {
        self.world.world_state_fingerprint()
    }

    pub fn semantic_state(&self) -> Result<Value> {
        self.export_envelope()?.materialize().map_err(unsupported)
    }
    pub fn request_json(&self, request: &NativeRequest) -> Value {
        let mut wire = self.request_json_base(request);
        if let Some(controlled) = request.controlled {
            wire["context"]["controlled_hero_id"] = json!(self.symbols.name(controlled));
            wire["player_id"] = json!(self.symbols.name(request.owner));
        }
        wire
    }
    fn request_json_base(&self, request: &NativeRequest) -> Value {
        let owner = request.controlled.unwrap_or(request.owner);
        if let RequestChoices::Minions(values) = &request.choices {
            return json!({"id":self.request_token(request.id),"request_type":"SELECT_OPTION","player_id":self.symbols.name(owner),"prompt":self.symbols.name(request.prompt),"options":values.iter().map(|(id,kind)|json!({"id":self.symbols.name(*id),"text":format!("{} Minion",match kind {crate::world::MinionKind::Melee=>"MELEE",crate::world::MinionKind::Ranged=>"RANGED",crate::world::MinionKind::Heavy=>"HEAVY"}),"metadata":{}})).collect::<Vec<_>>(),"can_skip":false,"can_rollback":false,"context":{},"decision_context":null});
        }
        if let RequestChoices::SpellCards {
            values,
            cast,
            removed,
            max,
        } = &request.choices
        {
            return json!({"id":self.request_token(request.id),"request_type":"SELECT_CARD","player_id":self.symbols.name(owner),
                "prompt":if *cast {"Choose a prepared spell to cast".to_owned()}else{format!("Find Familiar: remove up to three other prepared spells ({removed}/{max})")},
                "options":values.iter().map(|id|{let d=&self.world.definitions.cards[self.world.card(*id).unwrap().definition];json!({"id":self.symbols.name(d.printed_id),"text":self.symbols.name(d.name),"metadata":{}})}).collect::<Vec<_>>(),
                "can_skip":!*cast,"can_rollback":false,"context":{},"decision_context":if *cast {json!({"purpose":"cast_spell","actor_id":self.symbols.name(owner),"source_card_id":null,"choice_facts":{}})}else{Value::Null}});
        }
        if let RequestChoices::Protection(cards) = &request.choices {
            return json!({"id":self.request_token(request.id),"request_type":"SELECT_CARD","player_id":self.symbols.name(owner),
                "prompt":format!("Discard a card to protect {}?",self.symbols.name(request.prompt)),
                "options":cards.iter().map(|id| {let d=&self.world.definitions.cards[self.world.card(*id).unwrap().definition]; json!({"id":self.symbols.name(d.printed_id),"text":self.symbols.name(d.name),"metadata":{"card_id":self.symbols.name(d.printed_id),"color":d.color.map(|id| self.symbols.name(id))}})}).collect::<Vec<_>>(),
                "can_skip":true,"can_rollback":false,"context":{},"decision_context":{"purpose":"discard","actor_id":self.symbols.name(owner),"source_card_id":null,"choice_facts":{}}});
        }
        if let RequestChoices::RuneInscription {
            hero,
            placed,
            remaining,
        } = &request.choices
        {
            let slot = placed.len() + 1;
            let count = remaining.len();
            let mut counts = vec![1, count - 1, count];
            counts.dedup();
            return json!({"id":self.request_token(request.id),"request_type":"SELECT_OPTION","player_id":self.symbols.name(owner),"prompt":format!("Place a rune below turn slot {slot}"),
                "options":remaining.iter().map(|id| {let name=self.symbols.name(*id);let mut label=name.to_owned();label[..1].make_ascii_uppercase();json!({"id":name,"text":label,"metadata":{}})}).collect::<Vec<_>>(),
                "can_skip":false,"can_rollback":false,"context":{"supports_batch":true,"next_slot":slot,"placed_runes":self.runes_json(placed),"max_selections":count,"selection_counts":counts},"decision_context":{"purpose":"rune_inscription","actor_id":self.symbols.name(*hero),"source_card_id":null,"choice_facts":{}}});
        }
        if let RequestChoices::CardColors(values) = &request.choices {
            let options = values
                .iter()
                .map(|id| {
                    let name = self.symbols.name(*id);
                    json!({"id":name,"text":name,"metadata":{}})
                })
                .collect::<Vec<_>>();
            return json!({"id":self.request_token(request.id),"request_type":"SELECT_OPTION",
                "player_id":self.symbols.name(owner),"prompt":self.symbols.name(request.prompt),
                "options":options,"can_skip":false,"can_rollback":false,"context":{},"decision_context":null});
        }
        if let RequestChoices::Runes(values) = &request.choices {
            return json!({"id":self.request_token(request.id),"request_type":"SELECT_OPTION","player_id":self.symbols.name(owner),"prompt":self.symbols.name(request.prompt),"options":values.iter().map(|id| {let name=self.symbols.name(*id);let label=match name {"axe"=>"Axe","bird"=>"Bird","anvil"=>"Anvil","horn"=>"Horn",_=>name};json!({"id":name,"text":label,"metadata":{}})}).collect::<Vec<_>>(),"can_skip":false,"can_rollback":false,"context":{},"decision_context":{"purpose":"rune_selection","actor_id":self.symbols.name(owner),"source_card_id":null,"choice_facts":{}}});
        }
        if let RequestChoices::Mines(options) = &request.choices {
            return json!({"id":self.request_token(request.id),"request_type":"SELECT_OPTION","player_id":self.symbols.name(owner),"prompt":"Choose which mines to move through","options":options.iter().enumerate().map(|(i,path)|json!({"id":i.to_string(),"text":format!("Path through {} mine(s)",path.mines.len()),"metadata":{"metadata":{"mine_count":path.mines.len(),"mine_hexes":path.mines.iter().filter_map(|id|self.world.position(*id)).collect::<Vec<_>>(),"path":path.path}}})).collect::<Vec<_>>(),"can_skip":false,"can_rollback":false,"context":{},"decision_context":null});
        }
        if let RequestChoices::PrimaryReplacement {
            actor,
            card,
            primary,
            replacement,
        } = &request.choices
        {
            let mut facts = serde_json::Map::new();
            let options = [
                (*primary, "Primary", "primary"),
                (*replacement, "Replacement", "replacement"),
            ]
            .into_iter()
            .map(|(kind, label, role)| {
                let name = kind.name();
                facts.insert(
                    name.into(),
                    json!({"action_role":role,"defense_stat":null,"defense_kind":null}),
                );
                json!({"id":name,"text":format!("{label}: {name}"),"metadata":{"type":name}})
            })
            .collect::<Vec<_>>();
            return json!({"id":self.request_token(request.id),"request_type":"CHOOSE_ACTION","player_id":self.symbols.name(owner),"prompt":self.symbols.name(request.prompt),"options":options,"can_skip":false,"can_rollback":false,"context":{},"decision_context":{"purpose":"action","actor_id":self.symbols.name(*actor),"source_card_id":self.symbols.name(*card),"choice_facts":facts}});
        }
        if let RequestChoices::Action(menu) = &request.choices {
            return menu.to_json(self.request_token(request.id), &self.symbols);
        }
        if let RequestChoices::Defense(menu) = &request.choices {
            return menu.to_json(self.request_token(request.id), &self.symbols);
        }
        if let RequestChoices::Actors { team, values } = &request.choices {
            let names = values
                .iter()
                .map(|h| format!("'{}'", self.symbols.name(*h)))
                .collect::<Vec<_>>()
                .join(", ");
            return json!({"id":self.request_token(request.id),"request_type":"CHOOSE_ACTOR","player_id":self.symbols.name(owner),
                "prompt":format!("Team {}, choose who acts first between [{names}].",self.symbols.name(*team)),
                "options":values.iter().map(|h|json!({"id":self.symbols.name(*h),"text":self.symbols.name(*h),"metadata":{}})).collect::<Vec<_>>(),
                "can_skip":false,"can_rollback":false,"context":{"team":self.symbols.name(*team)},"decision_context":null});
        }
        if let RequestChoices::Respawn { hexes, location } = &request.choices {
            let options = if *location {
                hexes.iter().map(|h|{let(q,r,s)=h.coordinates();json!({"id":format!("hex_{q}_{r}_{s}"),"text":format!("({q},{r},{s})"),"metadata":{"hex":h,"raw":h}})}).collect::<Vec<_>>()
            } else {
                ["RESPAWN", "PASS"]
                    .iter()
                    .map(|id| json!({"id":id,"text":id,"metadata":{}}))
                    .collect()
            };
            return json!({"id":self.request_token(request.id),"request_type":if *location{"CHOOSE_RESPAWN_HEX"}else{"CHOOSE_RESPAWN"},"player_id":self.symbols.name(owner),
                "prompt":self.symbols.name(request.prompt),"options":options,"can_skip":false,"can_rollback":false,"context":{"valid_hexes":hexes},"decision_context":null});
        }
        if let RequestChoices::Upgrades(players) = &request.choices {
            let mut envelope = (*self.source).clone();
            self.world.export_cards(&mut envelope, &self.symbols);
            let players=Value::Object(players.iter().map(|player|{
                let options=player.options.iter().map(|option|json!({"color":self.symbols.name(option.color),"tier":self.symbols.name(option.tier),
                    "pair":option.pair.iter().map(|id|self.symbols.name(self.world.definitions.cards[self.world.card(*id).unwrap().definition].printed_id)).collect::<Vec<_>>(),
                    "card_details":option.pair.iter().map(|id|envelope.expand(&crate::cards::wire(Some(*id))).expect("compiled card record")).collect::<Vec<_>>() })).collect::<Vec<_>>();
                let mut counts=vec![1,player.remaining];counts.sort();counts.dedup();
                (self.symbols.name(player.hero).into(),json!({"remaining":player.remaining,"options":options,"selection_counts":counts}))
            }).collect());
            return json!({"id":self.request_token(request.id),"request_type":"UPGRADE_PHASE","player_id":"simultaneous","prompt":"Mandatory Upgrade Phase",
                "options":[],"can_skip":false,"can_rollback":false,"context":{"players":players,"supports_batch":true,"supports_preview":true},"decision_context":null});
        }
        let (kind,options)=match &request.choices {
            RequestChoices::Minions(_) | RequestChoices::SpellCards { .. } | RequestChoices::PrimaryReplacement { .. } | RequestChoices::Runes(_) | RequestChoices::CardColors(_) | RequestChoices::RuneInscription { .. } | RequestChoices::Protection(_) | RequestChoices::Mines(_) | RequestChoices::Defense(_) | RequestChoices::Action(_)|RequestChoices::Actors{..}|RequestChoices::Respawn{..}|RequestChoices::Upgrades(_)=>unreachable!(),
            RequestChoices::ConfirmAction=>("CHOOSE_ACTION",vec![json!({"id":"CONFIRM","text":"Confirm","metadata":{}})]),
            RequestChoices::Multiple{values,..}=>("SELECT_UNIT",values.iter().map(|id|json!({"id":self.symbols.name(*id),"text":self.symbols.name(*id),"metadata":{}})).collect()),
            RequestChoices::Units {tokens,values}=>(if *tokens {"SELECT_UNIT_OR_TOKEN"}else{"SELECT_UNIT"},values.iter().map(|(id,label)|{
                let text=label.map_or_else(||if *id == self.skip { "Keep all pieces".to_owned() } else { self.symbols.name(*id).to_owned() },|(name,hex)|{let(q,r,s)=hex.coordinates();format!("{} at ({q}, {r}, {s})",self.symbols.name(name))});
                json!({"id":self.symbols.name(*id),"text":text,"metadata":{}})
            }).collect()),
            RequestChoices::Hexes(values)|RequestChoices::ZoneHexes{values,..}=>("SELECT_HEX",values.iter().map(|hex|{
                let (q,r,s)=hex.coordinates();json!({"id":format!("hex_{q}_{r}_{s}"),"text":format!("({q},{r},{s})"),"metadata":{"hex":hex,"raw":hex}})
            }).collect()),
            RequestChoices::CardIds{values,..}=>("SELECT_CARD",values.iter().map(|id|json!({"id":self.symbols.name(*id),"text":self.symbols.name(*id),"metadata":{}})).collect()),
            RequestChoices::Cards(values)=>("SELECT_CARD",values.iter().map(|(id,name,initiative)|json!({"id":self.symbols.name(*id),"text":format!("{} (initiative {initiative})",self.symbols.name(*name)),"metadata":{}})).collect()),
            RequestChoices::Passive{..} | RequestChoices::Laugh=>("CONFIRM_PASSIVE",vec![json!({"id":"YES","text":"Yes","metadata":{}}),json!({"id":"NO","text":"No","metadata":{}})]),
            RequestChoices::Repeat{..} | RequestChoices::YesNo=>("SELECT_OPTION",vec![json!({"id":"YES","text":"Yes","metadata":{}}),json!({"id":"NO","text":"No","metadata":{}})]),
            RequestChoices::MirrorDistances(values)=>("SELECT_NUMBER", values.iter().map(|(distance, actual)|
                json!({"id":distance.to_string(),"text":distance.to_string(),"metadata":{"actual_moved":actual}})
            ).collect()),
            RequestChoices::Numbers {values,labels}=>("SELECT_NUMBER",values.iter().map(|n|{
                let label=labels.iter().find(|(number,_)|number==n).map(|(_,id)|self.symbols.name(*id).to_owned()).unwrap_or_else(||n.to_string());
                json!({"id":n.to_string(),"text":label,"metadata":if labels.is_empty(){json!({})}else{json!({"raw":n})}})
            }).collect()),
        };
        let prompt = if let Some(range) = request.pre_action_range {
            format!(
                "Pre-action movement (up to {range} space{})",
                if range > 1 { "s" } else { "" }
            )
        } else if let RequestChoices::Repeat { done, max } = &request.choices {
            format!("{} ({done}/{max} done)", self.symbols.name(request.prompt))
        } else if let RequestChoices::Multiple { count, max, .. } = &request.choices {
            format!("{} ({count}/{max})", self.symbols.name(request.prompt))
        } else if let RequestChoices::CardIds {
            defeat_victim: Some(v),
            ..
        } = &request.choices
        {
            format!(
                "{}, select a card to discard (or be Defeated).",
                self.symbols.name(*v)
            )
        } else if let RequestChoices::CardIds {
            forced_victim: Some(v),
            ..
        } = &request.choices
        {
            format!("{}, select a card to discard.", self.symbols.name(*v))
        } else {
            request.movement_range.map_or_else(
                || self.symbols.name(request.prompt).to_owned(),
                |range| format!("Select Movement Destination (Range {range})"),
            )
        };
        json!({"id":self.request_token(request.id),"request_type":kind,"player_id":self.symbols.name(owner),
            "prompt":prompt,"options":options,"can_skip":request.can_skip,"can_rollback":false,"context":match &request.choices {RequestChoices::Passive{card,name}=>json!({"card_id":self.symbols.name(*card),"card_name":self.symbols.name(*name)}),RequestChoices::ZoneHexes{unit:Some(id),..}=>json!({"context_unit_id":self.symbols.name(*id)}),_=>json!({})},
            "decision_context":if request.discard {json!({"purpose":"discard","actor_id":self.symbols.name(owner),"source_card_id":null,"choice_facts":{}})}else{Value::Null}})
    }
    fn runes_json(&self, placed: &[(Symbol, Symbol)]) -> Value {
        Value::Object(
            placed
                .iter()
                .map(|(slot, rune)| {
                    (
                        self.symbols.name(*slot).into(),
                        json!(self.symbols.name(*rune)),
                    )
                })
                .collect(),
        )
    }
    pub fn event_json(&self, event: &NativeEvent) -> Value {
        match event {
            NativeEvent::RunesPlaced { hero, placed } => {
                json!({"event_type":"RUNES_PLACED","actor_id":self.symbols.name(*hero),"target_id":null,"from_hex":null,"to_hex":null,"metadata":{"rune_slots":self.runes_json(placed)}})
            }
            NativeEvent::JourneyScheduled {
                actor,
                target,
                kind,
            } => {
                json!({"event_type":"EFFECT_CREATED","actor_id":self.symbols.name(*actor),"target_id":self.symbols.name(*target),"from_hex":null,"to_hex":null,"metadata":{"effect_type":self.symbols.name(*kind)}})
            }
            NativeEvent::ActionControlScheduled {
                actor,
                target,
                card,
            } => {
                json!({"event_type":"EFFECT_CREATED","actor_id":self.symbols.name(*actor),"target_id":self.symbols.name(*target),"from_hex":null,"to_hex":null,"metadata":{"effect":"action_control","controller_id":self.symbols.name(*actor),"card_id":self.symbols.name(*card)}})
            }
            NativeEvent::InitiativeOverride {
                actor,
                target,
                card,
                value,
            } => {
                json!({"event_type":"EFFECT_CREATED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*target),"from_hex":null,"to_hex":null,"metadata":{"effect":"initiative_override","card_id":self.symbols.name(*card),"value":value}})
            }
            NativeEvent::EffectCreated(event) => {
                let mut metadata = serde_json::Map::new();
                metadata.insert("effect_type".into(), json!(self.symbols.name(event.kind)));
                metadata.insert("duration".into(), json!(self.symbols.name(event.duration)));
                if let Some(axis) = event.split {
                    metadata.insert("split_axis".into(), json!(self.symbols.name(axis)));
                    metadata.insert("split_value".into(), json!(event.split_value));
                }
                if let Some(color) = event.color {
                    metadata.insert("named_color".into(), json!(self.symbols.name(color)));
                }
                json!({"event_type":"EFFECT_CREATED","actor_id":self.symbols.name(event.actor),"target_id":null,"from_hex":null,"to_hex":null,"metadata":metadata})
            }
            NativeEvent::Round(event) => event.to_json(&self.symbols),
            NativeEvent::TurnEnded(hero) => {
                json!({"event_type":"TURN_ENDED","actor_id":self.symbols.name(*hero),"target_id":null,"from_hex":null,"to_hex":null,"metadata":{}})
            }
            NativeEvent::RespawnCleared { hero, token, from } => {
                json!({"event_type":"TOKEN_REMOVED","actor_id":self.symbols.name(*hero),"target_id":self.symbols.name(*token),"from_hex":from,"to_hex":null,"metadata":{"reason":"respawn"}})
            }
            NativeEvent::MineTriggered {
                actor,
                target,
                from,
                kind,
                blast,
            } => {
                json!({"event_type":"MINE_TRIGGERED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*target),"from_hex":from,"to_hex":null,"metadata":{"token_type":kind.map(|id|self.symbols.name(id)),"is_blast":blast}})
            }
            NativeEvent::TokenRemoved {
                actor,
                token,
                from,
                effects_removed,
            } => {
                json!({"event_type":"TOKEN_REMOVED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*token),"from_hex":from,"to_hex":null,"metadata":{"effects_removed":effects_removed}})
            }
            NativeEvent::BoardEntityPlaced {
                actor,
                entity,
                from,
                to,
                obstacle,
            } => {
                json!({"event_type":"BOARD_ENTITY_PLACED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*entity),"from_hex":from,"to_hex":to,"metadata":{"entity_kind":"turret","is_obstacle":obstacle}})
            }
            NativeEvent::BoardEntityRemoved {
                actor,
                entity,
                from,
            } => {
                json!({"event_type":"BOARD_ENTITY_REMOVED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*entity),"from_hex":from,"to_hex":null,"metadata":{"entity_kind":"turret"}})
            }
            NativeEvent::TokenPlaced {
                actor,
                token,
                to,
                kind,
            } => {
                json!({"event_type":"TOKEN_PLACED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*token),"from_hex":null,"to_hex":to,"metadata":{"token_type":self.symbols.name(*kind)}})
            }
            NativeEvent::TokenReplaced {
                actor,
                token,
                from,
                to,
                kind,
            } => {
                json!({"event_type":"TOKEN_MOVED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*token),"from_hex":from,"to_hex":to,"metadata":{"token_type":self.symbols.name(*kind),"replaced":true}})
            }
            NativeEvent::TokenWalked {
                actor,
                token,
                from,
                to,
                range,
            } => {
                json!({"event_type":"TOKEN_MOVED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*token),"from_hex":from,"to_hex":to,"metadata":{"range":range}})
            }
            NativeEvent::TokenMoved {
                actor,
                token,
                from,
                to,
            } => {
                json!({"event_type":"TOKEN_MOVED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*token),"from_hex":from,"to_hex":to,"metadata":{}})
            }
            NativeEvent::Swapped { a, b, from, to } => {
                json!({"event_type":"UNITS_SWAPPED","actor_id":self.symbols.name(*a),"target_id":self.symbols.name(*b),"from_hex":from,"to_hex":to,"metadata":{}})
            }
            NativeEvent::Pushed {
                actor,
                target,
                token,
                from,
                to,
                distance,
                collision,
            } => {
                json!({"event_type":if *token{"TOKEN_PUSHED"}else{"UNIT_PUSHED"},"actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*target),"from_hex":from,"to_hex":to,"metadata":{"distance":distance,"collision":collision}})
            }
            NativeEvent::ItemGranted {
                owner,
                stat,
                amount,
            } => {
                json!({"event_type":"ITEM_GAINED","actor_id":self.symbols.name(*owner),"target_id":null,
                    "from_hex":null,"to_hex":null,"metadata":{"stat_type":stat.name(),"amount":amount}})
            }
            NativeEvent::GoldEffect {
                actor,
                target,
                owner,
                amount,
                lost,
            } => {
                let mut metadata =
                    json!({"amount":amount.to_json(&self.symbols),"reason":"effect"});
                if *lost {
                    metadata["owner_id"] = json!(self.symbols.name(*owner));
                }
                json!({"event_type":if *lost {"GOLD_LOST"} else {"GOLD_GAINED"},"actor_id":actor.map(|id|self.symbols.name(id)),"target_id":target.map(|id|self.symbols.name(id)),"from_hex":null,"to_hex":null,"metadata":metadata})
            }
            NativeEvent::Spell(event) => event.to_json(&self.symbols),
            NativeEvent::CoinsStolen {
                actor,
                victim,
                amount,
            } => {
                json!({"event_type":"GOLD_GAINED","actor_id":self.symbols.name(*actor),"target_id":self.symbols.name(*victim),"from_hex":null,"to_hex":null,"metadata":{"amount":amount.to_json(&self.symbols),"reason":"steal"}})
            }
            NativeEvent::AdditionalLifeSpent {
                team,
                target,
                total,
                change,
            } => {
                json!({"event_type":"LIFE_COUNTER_CHANGED","actor_id":null,"target_id":self.symbols.name(*target),
                    "from_hex":null,"to_hex":null,"metadata":{"team":self.symbols.name(*team),"change":change,
                    "remaining":total,"reason":"additional_life_counter"}})
            }
            NativeEvent::LifePenaltyAnnihilation { winner } => {
                json!({"event_type":"GAME_OVER","actor_id":null,"target_id":null,"from_hex":null,"to_hex":null,
                    "metadata":{"reason":"annihilation","winning_team":self.symbols.name(*winner)}})
            }
            NativeEvent::LifeRestored {
                hero,
                team,
                change,
                remaining,
            } => {
                json!({"event_type":"LIFE_COUNTER_CHANGED","actor_id":self.symbols.name(*hero),"target_id":null,"from_hex":null,"to_hex":null,"metadata":{"team":self.symbols.name(*team),"change":change,"remaining":remaining,"reason":"life_counter_restored"}})
            }
            NativeEvent::Marker(event) => event.to_json(&self.symbols),
            NativeEvent::Combat(event) => event.to_json(&self.symbols),
            NativeEvent::UnitPlaced { unit, from, to } => {
                json!({"event_type":"UNIT_PLACED","actor_id":self.symbols.name(*unit),"target_id":null,"from_hex":from,"to_hex":to,"metadata":{}})
            }
            NativeEvent::CardRetrieved {
                hero,
                card,
                name,
                kept,
            } => {
                let mut metadata = json!({"card_id":self.symbols.name(*card),"card_name":self.symbols.name(*name)});
                if let Some(kept) = kept {
                    metadata["kept_card_id"] = json!(self.symbols.name(*kept));
                    metadata["source"] = json!("alternative_timelines");
                }
                json!({"event_type":"CARD_RETRIEVED","actor_id":self.symbols.name(*hero),"target_id":null,"from_hex":null,"to_hex":null,"metadata":metadata})
            }
            NativeEvent::GuessSelected {
                actor,
                victim,
                attempt,
            } => {
                json!({"event_type":"CARD_SELECTED_FOR_GUESS","actor_id":actor.map(|id|self.symbols.name(id)),
                    "target_id":self.symbols.name(*victim),"from_hex":null,"to_hex":null,"metadata":{"attempt":attempt}})
            }
            NativeEvent::ResolvedCardsSwapped {
                actor,
                target,
                a,
                b,
            } => json!({
                "event_type":"RESOLVED_CARDS_SWAPPED",
                "actor_id":actor.map(|id|self.symbols.name(id)),
                "target_id":self.symbols.name(*target), "from_hex":null, "to_hex":null,
                "metadata":{"card_a_id":self.symbols.name(*a),"card_b_id":self.symbols.name(*b)}
            }),
            NativeEvent::HeroPieceRemoved { owner, piece, from } => json!({
                "event_type":"UNIT_REMOVED", "actor_id":self.symbols.name(*owner),
                "target_id":self.symbols.name(*piece), "from_hex":from, "to_hex":null,
                "metadata":{"owner_hero_id":self.symbols.name(*owner),"voluntary":true}
            }),
            NativeEvent::HeroPieceSpawned { owner, piece, to } => json!({
                "event_type":"UNIT_PLACED", "actor_id":self.symbols.name(*owner),
                "target_id":self.symbols.name(*piece), "from_hex":null, "to_hex":to,
                "metadata":{"owner_hero_id":self.symbols.name(*owner)}
            }),
            NativeEvent::HeroLaughed(actor) => json!({
                "event_type":"HERO_LAUGHED", "actor_id":self.symbols.name(*actor),
                "target_id":null, "from_hex":null, "to_hex":null, "metadata":{}
            }),
            NativeEvent::HandCardRevealed {
                actor,
                target,
                owner,
                card,
                name,
                color,
                tier,
                tier_value,
            } => json!({"event_type":"CARD_REVEALED",
                    "actor_id":actor.map(|id|self.symbols.name(id)),
                    "target_id":self.symbols.name(*target),"from_hex":null,"to_hex":null,
                    "metadata":{"owner_id":self.symbols.name(*owner),
                        "card_id":self.symbols.name(*card),"card_name":self.symbols.name(*name),
                        "card_color":color.map(|id|self.symbols.name(id)),
                        "card_tier":self.symbols.name(*tier),"tier_value":tier_value}}),
            NativeEvent::GuessRevealed {
                actor,
                victim,
                attempt,
                card,
                name,
                actual,
                guessed,
                correct,
            } => {
                json!({"event_type":"GUESSED_CARD_REVEALED","actor_id":actor.map(|id|self.symbols.name(id)),
                    "target_id":self.symbols.name(*victim),"from_hex":null,"to_hex":null,
                    "metadata":{"attempt":attempt,"card_id":self.symbols.name(*card),"card_name":self.symbols.name(*name),
                        "card_color":actual.map(|id|self.symbols.name(id)),"guessed_color":self.symbols.name(*guessed),"guess_correct":correct}})
            }
            NativeEvent::DeckCardSwapped {
                actor,
                outgoing,
                incoming,
                state,
                facedown,
            } => {
                json!({"event_type":"DECK_CARD_SWAPPED","actor_id":self.symbols.name(*actor),"target_id":null,"from_hex":null,"to_hex":null,"metadata":{"outgoing_card_id":self.symbols.name(*outgoing),"incoming_card_id":self.symbols.name(*incoming),"incoming_card_state":state,"incoming_is_facedown":facedown}})
            }
            NativeEvent::ItemGained { actor, stat, card } => {
                json!({"event_type":"ITEM_GAINED","actor_id":self.symbols.name(*actor),"target_id":null,"from_hex":null,"to_hex":null,"metadata":{"stat_type":stat.name(),"amount":1,"source_card_id":self.symbols.name(*card)}})
            }
            NativeEvent::TieBreakerFlipped { blue } => {
                json!({"event_type":"TIE_BREAKER_FLIPPED","actor_id":null,"target_id":null,"from_hex":null,"to_hex":null,
                "metadata":{"tie_breaker_team":if *blue {"BLUE"}else{"RED"},"coin_face":if *blue {"BLUE"}else{"ORANGE"}}})
            }
            NativeEvent::DirectionalMove {
                unit,
                from,
                to,
                direction,
            } => {
                json!({"event_type":"UNIT_MOVED","actor_id":self.symbols.name(*unit),"target_id":null,
                    "from_hex":from,"to_hex":to,"metadata":{"direction":direction,"simultaneous":true}})
            }
            NativeEvent::UnitMoved {
                unit,
                from,
                to,
                range,
            } => {
                json!({"event_type":"UNIT_MOVED","actor_id":self.symbols.name(*unit),"target_id":null,"from_hex":from,"to_hex":to,"metadata":{"range":range}})
            }
        }
    }
}
fn valid_rune_batch(selection: &Datum, remaining: &[Symbol]) -> Option<Vec<Symbol>> {
    let values = match selection {
        Datum::List(values) => values.as_slice(),
        value => std::slice::from_ref(value),
    };
    if values.is_empty()
        || ![1, remaining.len().saturating_sub(1), remaining.len()].contains(&values.len())
    {
        return None;
    }
    let mut chosen = Vec::with_capacity(values.len());
    for value in values {
        let Datum::Text(rune) = value else {
            return None;
        };
        if !remaining.contains(rune) || chosen.contains(rune) {
            return None;
        }
        chosen.push(*rune);
    }
    Some(chosen)
}

fn datum_hex(value: &Datum, symbols: &Symbols, keys: [Symbol; 3]) -> Result<Hex> {
    value.hex(keys, symbols).map_err(EngineError::Rule)
}
fn native_hex(hex: Hex, keys: [Symbol; 3]) -> Datum {
    let (q, r, s) = hex.coordinates();
    Datum::Object(
        keys.into_iter()
            .zip([q, r, s])
            .map(|(key, value)| (key, Datum::Int(i64::from(value))))
            .collect(),
    )
}
fn required_symbol(raw: &Value, key: &str, symbols: &mut Symbols) -> Result<Symbol> {
    Ok(symbols.intern(
        raw[key]
            .as_str()
            .ok_or_else(|| unsupported(format!("missing string field: {key}")))?,
    ))
}
fn optional_symbol(raw: &Value, key: &str, symbols: &mut Symbols) -> Result<Option<Symbol>> {
    match raw.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok((!value.is_empty()).then(|| symbols.intern(value))),
        _ => Err(unsupported(format!("invalid string field: {key}"))),
    }
}
fn integer(raw: &Value, key: &str, default: i64) -> Result<i64> {
    raw.get(key).map_or(Ok(default), |v| {
        v.as_i64()
            .ok_or_else(|| unsupported(format!("invalid integer field: {key}")))
    })
}
fn flag(raw: &Value, key: &str, default: bool) -> Result<bool> {
    raw.get(key).map_or(Ok(default), |v| {
        v.as_bool()
            .ok_or_else(|| unsupported(format!("invalid boolean field: {key}")))
    })
}
fn compile_movement_sequence(
    raw: &Value,
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
) -> Result<Instruction> {
    let stat = raw
        .get("range_stat_type")
        .filter(|v| !v.is_null())
        .map(|v| {
            Stat::parse(
                v.as_str()
                    .ok_or_else(|| unsupported("invalid movement stat"))?,
            )
            .map_err(unsupported)
        })
        .transpose()?;
    let templates = crate::templates::instructions().map_err(unsupported)?;
    let range = i32::try_from(integer(raw, "range_val", 1)?)
        .map_err(|_| unsupported("movement range outside i32"))?;
    let mandatory = flag(raw, "is_mandatory", true)?;
    let through = flag(raw, "pass_through_obstacles", false)?;
    let straight = flag(raw, "force_straight_line", false)?;
    let destination = raw["destination_key"].as_str().unwrap_or("target_hex");
    let mut filters = vec![
        templates["templates"]["obstacle"].clone(),
        templates["templates"]["movement_path"].clone(),
    ];
    filters[1]["range_val"] = json!(range);
    filters[1]["pass_through_obstacles"] = json!(through);
    if straight {
        filters.push(templates["templates"]["straight"].clone());
        let mut path = templates["templates"]["straight_path"].clone();
        path["pass_through_obstacles"] = json!(through);
        filters.push(path);
    }
    if flag(raw, "force_full_distance", false)? {
        let mut filter = templates["templates"]["range"].clone();
        filter["min_range"] = json!(range);
        filter["max_range"] = json!(range);
        filters.push(filter);
    }
    let mut selection = templates["templates"]["select_hex"].clone();
    selection["prompt"] = json!(format!("Select Movement Destination (Range {range})"));
    selection["output_key"] = json!(destination);
    selection["is_mandatory"] = json!(mandatory);
    selection["filters"] = Value::Array(filters);
    let select = compile_step(&selection, steps, symbols)?;
    if let Instruction::Hex { filters, .. } = &mut steps[select].instruction {
        for filter in filters {
            filter.bind_frame();
        }
    }
    let mut movement = templates["templates"]["move_unit"].clone();
    movement["range_val"] = json!(range);
    movement["destination_key"] = json!(destination);
    movement["is_mandatory"] = json!(mandatory);
    movement["is_movement_action"] = json!(true);
    movement["pass_through_obstacles"] = json!(through);
    movement["force_straight_line"] = json!(straight);
    let movement = compile_step(&movement, steps, symbols)?;
    Ok(Instruction::MovementSequence {
        unit: optional_symbol(raw, "unit_id", symbols)?,
        destination: symbols.intern(destination),
        select,
        movement,
        range,
        stat,
    })
}
fn compile_travel_sequence(
    raw: &Value,
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
) -> Result<Instruction> {
    let frozen = crate::templates::instructions().map_err(unsupported)?;
    let destination = raw["destination_key"].as_str().unwrap_or("target_hex");
    let change = flag(raw, "require_zone_change", false)?;
    let source_card = raw.get("source_card_id").cloned().unwrap_or(Value::Null);
    let mut filter = frozen["templates"]["travel_filter"].clone();
    filter["unit_id"] = Value::Null;
    filter["require_zone_change"] = json!(change);
    filter["source_card_id"] = source_card.clone();
    let mut selection = frozen["templates"]["select_hex"].clone();
    selection["prompt"] = json!("Select Fast Travel Destination");
    selection["output_key"] = json!(destination);
    selection["is_mandatory"] = json!(false);
    selection["filters"] = json!([filter]);
    let select = compile_step(&selection, steps, symbols)?;
    if let Instruction::Hex { filters, .. } = &mut steps[select].instruction
        && let HexFilter::FastTravel { origin, .. } = &mut filters[0]
    {
        *origin = Origin {
            frame: true,
            ..Default::default()
        };
    }
    let mut travel = frozen["templates"]["fast_travel_unit"].clone();
    travel["destination_key"] = json!(destination);
    travel["require_zone_change"] = json!(change);
    travel["source_card_id"] = source_card;
    travel["is_mandatory"] = json!(false);
    let travel = compile_step(&travel, steps, symbols)?;
    Ok(Instruction::TravelSequence {
        unit: optional_symbol(raw, "unit_id", symbols)?,
        destination: symbols.intern(destination),
        select,
        travel,
    })
}
fn combat_error(error: CombatError) -> EngineError {
    match error {
        CombatError::Rule(s) => EngineError::Rule(s),
        CombatError::Unsupported(s) => unsupported(s),
    }
}

fn compile_primary_program(
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
) -> Result<PrimaryProgram> {
    let source = crate::primary::catalog().map_err(unsupported)?;
    let mut bodies = Vec::new();
    for (name, wires) in source["recipes"]
        .as_object()
        .ok_or_else(|| unsupported("primary recipes missing"))?
    {
        let body = wires
            .as_array()
            .ok_or_else(|| unsupported("primary recipe not a list"))?
            .iter()
            .enumerate()
            .map(|(index, wire)| {
                let first = steps.len();
                let id = compile_step(wire, steps, symbols)?;
                for step in &mut steps[first..] {
                    step.primary_template = true;
                }
                if source["ranged_steps"][name]
                    .as_array()
                    .is_some_and(|v| v.contains(&json!(index)))
                {
                    let Instruction::Attack { spec, .. } = &mut steps[id].instruction else {
                        return Err(unsupported("ranged operand on non-attack recipe"));
                    };
                    spec.card_ranged = true;
                }
                Ok(id)
            })
            .collect::<Result<Vec<_>>>()?;
        bodies.push((symbols.intern(name), body));
    }
    let mut movement = combat_template("movement_sequence")?;
    movement["unit_id"] = json!(crate::primary::HERO);
    movement["range_val"] = json!(crate::primary::PRIMARY);
    movement["range_stat_type"] = Value::Null;
    let movement = compile_step(&movement, steps, symbols)?;
    let mut attack = combat_template("attack_sequence")?;
    attack["damage"] = json!(crate::primary::PRIMARY);
    attack["range_val"] = json!(crate::primary::RANGE);
    attack["damage_stat_type"] = Value::Null;
    attack["range_stat_type"] = Value::Null;
    let attack = compile_step(&attack, steps, symbols)?;
    let mut log = combat_template("log")?;
    log["message"] = json!(crate::primary::DEFENSE_LOG);
    let defense = compile_step(&log, steps, symbols)?;
    log["message"] = json!(crate::primary::SKILL_LOG);
    let skill = compile_step(&log, steps, symbols)?;
    Ok(PrimaryProgram {
        bodies,
        movement,
        attack,
        defense,
        skill,
    })
}
fn bind_primary(
    instruction: &mut Instruction,
    v: &PrimaryValues,
    slots: &PrimaryOperands,
) -> Result<()> {
    let number = |n: i64| slots.number(n, v).map_err(unsupported);
    match instruction {
        Instruction::Attack { spec, .. } => {
            if spec.card_ranged {
                spec.ranged = v.ranged;
            }
            spec.damage = number(spec.damage)?;
            spec.range = number(spec.range)?;
        }
        Instruction::Query { spec, .. } => spec.bind(slots, v).map_err(unsupported)?,
        Instruction::ContextCondition { threshold, .. } => *threshold = number(*threshold)?,
        Instruction::Hex { filters, .. } => {
            for f in filters {
                slots.hex_filter(f, v).map_err(unsupported)?;
            }
        }
        Instruction::Unit { filters, .. } | Instruction::Multi { filters, .. } => {
            for f in filters {
                slots.unit_filter(f, v).map_err(unsupported)?;
            }
        }
        Instruction::MovementSequence { unit, range, .. }
        | Instruction::Move { unit, range, .. } => {
            slots.optional_symbol(unit, v);
            *range = i32::try_from(number(i64::from(*range))?)
                .map_err(|_| unsupported("primary movement range outside i32"))?;
        }
        Instruction::Push { spec, .. } => {
            slots.optional_symbol(&mut spec.target, v);
            spec.distance = number(spec.distance)?;
        }
        Instruction::Place { unit, .. }
        | Instruction::RecordHex { unit, .. }
        | Instruction::FastTravel { unit, .. }
        | Instruction::TravelSequence { unit, .. } => slots.optional_symbol(unit, v),
        Instruction::Swap { a, b, .. } | Instruction::CheckAdjacency { a, b, .. } => {
            slots.optional_symbol(a, v);
            slots.optional_symbol(b, v);
        }
        Instruction::PlaceMarker { target, value, .. } => {
            slots.optional_symbol(target, v);
            *value = number(*value)?;
        }
        Instruction::ForceDiscard { victim, .. } => slots.optional_symbol(victim, v),
        Instruction::CreateEffect(spec) => Arc::make_mut(spec)
            .bind_primary(|value| bind_primary_datum(value, v, slots).map_err(|e| e.to_string()))
            .map_err(unsupported)?,
        Instruction::Set { value, .. } => bind_primary_datum(value, v, slots)?,
        _ => {}
    }
    Ok(())
}
fn bind_primary_datum(value: &mut Datum, v: &PrimaryValues, slots: &PrimaryOperands) -> Result<()> {
    match value {
        Datum::Text(id) => *id = slots.symbol(*id, v),
        Datum::Int(n) => *n = slots.number(*n, v).map_err(unsupported)?,
        Datum::List(values) => {
            for value in values {
                bind_primary_datum(value, v, slots)?;
            }
        }
        Datum::Object(fields) => {
            for (_, value) in fields {
                bind_primary_datum(value, v, slots)?;
            }
        }
        _ => {}
    }
    Ok(())
}
/// Bind small typed recipe operands without constructing or parsing step JSON.
fn bind_action(
    instruction: &mut Instruction,
    action: &ActionStep,
    symbols: &Symbols,
) -> Result<()> {
    match (action, instruction) {
        (ActionStep::ChoosePiece(h), Instruction::ChoosePiece { hero, .. }) => *hero = *h,
        (ActionStep::Passive(p), Instruction::Passive { trigger, .. }) => {
            *trigger = symbols
                .lookup(p.name())
                .ok_or_else(|| unsupported("passive trigger missing"))?
        }
        (ActionStep::PreMovement(owner), Instruction::PreMovement { hero, .. })
        | (ActionStep::PreDiscard(owner), Instruction::PreDiscard { hero, .. }) => {
            *hero = Some(*owner)
        }
        (ActionStep::CardText { hero: h, .. }, Instruction::CardText { hero, .. }) => *hero = *h,
        (
            ActionStep::Movement { hero, base },
            Instruction::MovementSequence {
                unit, range, stat, ..
            },
        ) => {
            *unit = Some(*hero);
            *range = i32::try_from(*base).map_err(|_| unsupported("movement base outside i32"))?;
            *stat = Some(Stat::Movement);
        }
        (ActionStep::FastTravel { hero, .. }, Instruction::TravelSequence { unit, .. }) => {
            *unit = Some(*hero)
        }
        (ActionStep::Attack { damage, range }, Instruction::Attack { spec, .. }) => {
            spec.damage = *damage;
            spec.range = *range;
            spec.damage_stat = Some(Stat::Attack);
            spec.range_stat = Some(Stat::Range);
        }
        (ActionStep::Hold(_) | ActionStep::Defend(_), Instruction::Log) => {}
        (ActionStep::Context(k, v), Instruction::Set { key, value }) => {
            *key = *k;
            *value = v.clone();
        }
        _ => return Err(unsupported("invalid action instruction binding")),
    }
    Ok(())
}

fn combat_template(name: &str) -> Result<Value> {
    let frozen = crate::templates::instructions().map_err(unsupported)?;
    let wire = frozen["templates"][name].clone();
    if wire.is_null() {
        return Err(unsupported("missing combat template"));
    }
    Ok(wire)
}
fn nullable_symbol(raw: &Value, key: &str, symbols: &mut Symbols) -> Result<Option<Symbol>> {
    match raw.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(symbols.intern(s))),
        _ => Err(unsupported(format!("invalid nullable symbol {key}"))),
    }
}
fn compile_attack(
    raw: &Value,
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
) -> Result<Instruction> {
    let stat = |name: &str| -> Result<Option<Stat>> {
        raw.get(name)
            .filter(|v| !v.is_null())
            .map(|v| {
                Stat::parse(
                    v.as_str()
                        .ok_or_else(|| unsupported("invalid attack stat"))?,
                )
                .map_err(unsupported)
            })
            .transpose()
    };
    let spec = AttackSpec {
        damage: integer(raw, "damage", 0)?,
        range: integer(raw, "range_val", 1)?,
        ranged: flag(raw, "is_ranged", false)?,
        card_ranged: false,
        initiative: flag(raw, "defense_uses_initiative", false)?,
        target: optional_symbol(raw, "target_id_key", symbols)?,
        output: optional_symbol(raw, "target_output_key", symbols)?,
        damage_bonus: optional_symbol(raw, "damage_bonus_key", symbols)?,
        range_bonus: optional_symbol(raw, "range_bonus_key", symbols)?,
        damage_stat: stat("damage_stat_type")?,
        range_stat: stat("range_stat_type")?,
    };
    let mut select = combat_template("select_unit")?;
    select["prompt"] = json!("Select Attack Target");
    select["output_key"] = json!("victim_id");
    select["is_mandatory"] = json!(flag(raw, "is_mandatory", true)?);
    let mut range = combat_template("range")?;
    range["max_range"] = json!(spec.range);
    let mut filters = vec![range, json!({"type":"team_filter","relation":"ENEMY"})];
    if let Some(extra) = raw.get("target_filters") {
        filters.extend(
            extra
                .as_array()
                .ok_or_else(|| unsupported("invalid attack filters"))?
                .iter()
                .cloned(),
        );
    }
    select["filters"] = json!(filters);
    let select = compile_step(&select, steps, symbols)?;
    let mut recipes = Vec::new();
    recipes.push(combat_template("reaction")?);
    let mut actor = combat_template("set_actor")?;
    actor["actor_key"] = json!("defender_id");
    actor["save_key"] = json!("_pre_pam_actor");
    recipes.push(actor);
    recipes.push(combat_template("passive")?);
    let mut movement = combat_template("pre_movement")?;
    movement["hero_id"] = Value::Null;
    movement["hero_key"] = json!("defender_id");
    movement["active_if_key"] = json!("is_primary_defense");
    recipes.push(movement);
    let mut discard = combat_template("pre_discard")?;
    discard["hero_id"] = Value::Null;
    discard["hero_key"] = json!("defender_id");
    discard["active_if_key"] = json!("is_primary_defense");
    discard["abort_on_defeat"] = json!(false);
    recipes.push(discard);
    let mut actor = combat_template("set_actor")?;
    actor["actor_key"] = json!("_pre_pam_actor");
    actor["save_key"] = json!("_discard_pam");
    recipes.push(actor);
    recipes.push(combat_template("defense_text")?);
    recipes.push(combat_template("resolve_combat")?);
    recipes.push(combat_template("on_block")?);
    recipes.push(combat_template("restore_action")?);
    let children = recipes
        .iter()
        .map(|wire| compile_step(wire, steps, symbols))
        .collect::<Result<Vec<_>>>()?;
    Ok(Instruction::Attack {
        spec,
        select,
        children,
    })
}
fn bind_pre_movement(instruction: &mut Instruction, hero: Symbol, range: i32) -> Result<()> {
    match instruction {
        Instruction::Hex { filters, .. } => {
            for filter in filters {
                match filter {
                    HexFilter::Obstacle { exclude, .. } => {
                        *exclude = Origin {
                            literal: Some(hero),
                            key: None,
                            frame: false,
                        }
                    }
                    HexFilter::Movement {
                        origin,
                        range: distance,
                        ..
                    } => {
                        *origin = Origin {
                            literal: Some(hero),
                            key: None,
                            frame: false,
                        };
                        *distance = range;
                    }
                    _ => return Err(unsupported("unknown pre-action movement predicate")),
                }
            }
        }
        Instruction::Move {
            unit,
            range: distance,
            ..
        } => {
            *unit = Some(hero);
            *distance = range;
        }
        _ => return Err(unsupported("unknown pre-action movement child")),
    }
    Ok(())
}
fn compile_projectile(
    deflect: bool,
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
) -> Result<(usize, usize)> {
    let key = if deflect {
        "deflect_victim"
    } else {
        "reflect_victim"
    };
    let mut select = combat_template("select_unit")?;
    select["prompt"] = json!(if deflect {
        "Select enemy hero in range to discard (not attacker)"
    } else {
        "Select enemy hero in range to discard"
    });
    select["output_key"] = json!(key);
    select["is_mandatory"] = json!(false);
    let mut range = combat_template("range")?;
    range["max_range"] = json!(3);
    let mut filters = vec![
        json!({"type":"unit_type_filter","unit_type":"HERO"}),
        json!({"type":"team_filter","relation":"ENEMY"}),
        range,
    ];
    if deflect {
        filters.push(json!({"type":"exclude_identity_filter","exclude_self":true,"exclude_keys":["attacker_id"]}));
    }
    select["filters"] = json!(filters);
    let mut force = combat_template("force_discard")?;
    force["victim_key"] = json!(key);
    Ok((
        compile_step(&select, steps, symbols)?,
        compile_step(&force, steps, symbols)?,
    ))
}
fn compile_step(
    raw: &Value,
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
) -> Result<usize> {
    let conditions = Conditions {
        active: optional_symbol(raw, "active_if_key", symbols)?,
        skip: optional_symbol(raw, "skip_if_key", symbols)?,
        mandatory: flag(raw, "is_mandatory", true)?,
    };
    let instruction = match raw["type"]
        .as_str()
        .ok_or_else(|| unsupported("step lacks type"))?
    {
        "cast_spell" => Instruction::LiveSpell(crate::spells::SpellOp::Cast),
        "wish_sequence" => Instruction::LiveSpell(crate::spells::SpellOp::Wish),
        "prepare_spellbook" => Instruction::LiveSpell(crate::spells::SpellOp::Prepare),
        "remove_prepared_spells" => Instruction::LiveSpell(crate::spells::SpellOp::Remove),
        "check_unit_filters" => Instruction::LiveCheckUnitFilters,
        "check_distance" => Instruction::LiveDistanceCheck,
        "check_unit_on_board" => Instruction::LiveUnitOnBoard,
        "check_unit_type" => Instruction::LiveUnitTypeCheck,
        "combine_boolean_context" => Instruction::LiveBooleanCombine,
        "convert_card_to_item" => Instruction::LiveConvertCardItem,
        "apply_after_attack_card_text" => Instruction::LiveAfterAttackText,
        "place_token" => Instruction::LiveTokenPlace,
        "respawn_minion_at_hex" => Instruction::LiveMinionRespawn,
        "count_adjacent_enemies" => Instruction::LiveAdjacentEnemies,
        "check_zone_change" => Instruction::LiveZoneChange,
        "check_solo_win" => Instruction::LiveSoloWin,
        "count_cards" => Instruction::LiveCardCount,
        "check_hero_defeated_this_round" => Instruction::LiveRoundDefeat,
        "gain_coins" | "lose_coins" | "restore_spent_life_counter" | "steal_coins" => {
            Instruction::LiveResource
        }
        "perform_card_action" => Instruction::LivePerformCard,
        "find_next_actor" => Instruction::Flow(crate::turn::FlowStep::Next),
        "finalize_hero_turn" => Instruction::Flow(crate::turn::FlowStep::Finalize(
            required_symbol(raw, "hero_id", symbols)?,
        )),
        "confirm_resolution" => Instruction::Flow(crate::turn::FlowStep::Confirm(required_symbol(
            raw, "hero_id", symbols,
        )?)),
        "respawn_hero" => Instruction::Flow(crate::turn::FlowStep::Respawn(required_symbol(
            raw, "hero_id", symbols,
        )?)),
        "resolve_tie_breaker" => Instruction::Flow(crate::turn::FlowStep::Tie(
            raw["tied_hero_ids"]
                .as_array()
                .ok_or_else(|| unsupported("tie candidates missing"))?
                .iter()
                .map(|id| {
                    id.as_str()
                        .map(|id| symbols.intern(id))
                        .ok_or_else(|| unsupported("invalid tied hero"))
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        "advance_turn" => Instruction::Flow(crate::turn::FlowStep::Advance),
        "end_phase" => Instruction::Flow(crate::turn::FlowStep::EndPhase),
        "end_phase_cleanup" => Instruction::Flow(crate::turn::FlowStep::Cleanup),
        "resolve_upgrades" => Instruction::Flow(crate::turn::FlowStep::Upgrades),
        "round_reset" => Instruction::Flow(crate::turn::FlowStep::Reset),
        "attack_sequence" => compile_attack(raw, steps, symbols)?,
        "check_target_defeated" => Instruction::CheckDefeated {
            target: required_symbol(raw, "target_key", symbols)?,
            output: required_symbol(raw, "output_key", symbols)?,
            by_actor: flag(raw, "by_actor", false)?,
        },
        "reaction_window" => {
            let discard = compile_step(&combat_template("discard_card")?, steps, symbols)?;
            let mut passive = combat_template("passive")?;
            passive["trigger"] = json!("before_defense");
            let passive = compile_step(&passive, steps, symbols)?;
            Instruction::Reaction {
                target: symbols.intern(raw["target_player_key"].as_str().unwrap_or("target_id")),
                discard,
                passive,
            }
        }
        "multi_select" => {
            if raw["target_type"] != "UNIT" && raw["target_type"] != "UNIT_OR_TOKEN" {
                return Err(unsupported("multi-selection target closure not compiled"));
            }
            let mut single = raw.clone();
            single["type"] = json!("select_step");
            let id = compile_step(&single, steps, symbols)?;
            let Instruction::Unit {
                output,
                filters,
                tokens,
                prompt,
                ..
            } = steps[id].instruction.clone()
            else {
                return Err(unsupported("invalid multi-selection template"));
            };
            Instruction::Multi {
                output,
                filters,
                tokens,
                prompt,
                min: integer(raw, "min_selections", 0)?,
                max: integer(raw, "max_selections", 0)?,
            }
        }
        "remove_token" => Instruction::RemoveToken {
            literal: optional_symbol(raw, "token_id", symbols)?,
            key: optional_symbol(raw, "token_key", symbols)?,
        },
        "resolve_card" => {
            let mut program = Vec::new();
            for key in [
                "action_piece",
                "passive",
                "pre_movement",
                "pre_discard",
                "card_text",
                "movement_sequence",
                "fast_travel",
                "attack_sequence",
                "log",
                "context",
                "clear_select",
                "clear_each",
            ] {
                let mut wire = combat_template(key)?;
                if key == "clear_each" {
                    wire["steps_template"][0]["step_id"] =
                        json!("native-step-clear-remove-template");
                }
                program.push((key, compile_step(&wire, steps, symbols)?));
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
            Instruction::ResolveCard {
                hero: required_symbol(raw, "hero_id", symbols)?,
                program: Arc::new(program),
            }
        }
        "resolve_card_text" => Instruction::CardText {
            hero: required_symbol(raw, "hero_id", symbols)?,
            program: Arc::new(compile_primary_program(steps, symbols)?),
        },
        "compute_hex" => Instruction::ComputeHex {
            origin: optional_symbol(raw, "origin_key", symbols)?,
            target: symbols.intern(raw["target_key"].as_str().unwrap_or("")),
            scale: integer(raw, "scale", 1)?,
            output: symbols.intern(raw["output_key"].as_str().unwrap_or("computed_hex")),
        },
        "push_unit" => {
            let mut child = combat_template("passive")?;
            child["trigger"] = json!("after_push");
            Instruction::Push {
                spec: crate::displacement::PushSpec {
                    target: optional_symbol(raw, "target_id", symbols)?,
                    key: optional_symbol(raw, "target_key", symbols)?,
                    source: raw
                        .get("source_hex")
                        .filter(|v| !v.is_null())
                        .map(|v| {
                            serde_json::from_value(v.clone())
                                .map_err(|e| unsupported(e.to_string()))
                        })
                        .transpose()?,
                    source_key: optional_symbol(raw, "source_key", symbols)?,
                    distance: integer(raw, "distance", 1)?,
                    distance_key: optional_symbol(raw, "distance_key", symbols)?,
                    collision: optional_symbol(raw, "collision_output_key", symbols)?,
                    ignore: flag(raw, "ignore_obstacles", false)?,
                    victim_output: symbols.intern("push_victim_id"),
                },
                passive: compile_step(&child, steps, symbols)?,
            }
        }
        "check_lane_push" | "minion_battle" => {
            let lane = raw["lane_id"].as_str().map(|s| symbols.intern(s));
            Instruction::Battle(if raw["type"] == "check_lane_push" {
                crate::battle::BattleStep::Check(lane)
            } else {
                crate::battle::BattleStep::Minions(lane)
            })
        }
        "lane_push" => {
            Instruction::Battle(crate::battle::BattleStep::Push(crate::battle::PushSpec {
                lane: required_symbol(raw, "lane_id", symbols)?,
                loser: required_symbol(raw, "losing_team", symbols)?,
                target: raw["target_zone_id"].as_str().map(|s| symbols.intern(s)),
                skip_wave: flag(raw, "skip_wave_counter", false)?,
            }))
        }
        "choose_minion_removal" => Instruction::Battle(crate::battle::BattleStep::Casualties(
            crate::battle::RemovalSpec {
                loser: required_symbol(raw, "losing_team", symbols)?,
                zone: required_symbol(raw, "zone_id", symbols)?,
                remaining: integer(raw, "remaining_to_remove", 0)?,
            },
        )),
        "resolve_displacement" => {
            let items = raw["displacements"]
                .as_array()
                .ok_or_else(|| unsupported("displacement queue missing"))?
                .iter()
                .map(|v| {
                    let pair = v
                        .as_array()
                        .filter(|v| v.len() == 2)
                        .ok_or_else(|| unsupported("invalid displacement entry"))?;
                    let id = symbols.intern(
                        pair[0]
                            .as_str()
                            .ok_or_else(|| unsupported("invalid displaced unit"))?,
                    );
                    let hex = serde_json::from_value(pair[1].clone())
                        .map_err(|e| unsupported(format!("displacement hex: {e}")))?;
                    Ok((id, hex))
                })
                .collect::<Result<Vec<_>>>()?;
            Instruction::Battle(crate::battle::BattleStep::Displace(items))
        }
        "return_minion_to_zone" => {
            let processed = raw["processed_minion_ids"]
                .as_array()
                .ok_or_else(|| unsupported("minion return processed list missing"))?
                .iter()
                .map(|id| {
                    id.as_str()
                        .map(|id| symbols.intern(id))
                        .ok_or_else(|| unsupported("invalid processed minion"))
                })
                .collect::<Result<Vec<_>>>()?;
            let mut movement = combat_template("move_unit")?;
            movement["is_mandatory"] = json!(false);
            movement["is_movement_action"] = json!(false);
            let mut placement = combat_template("place_unit")?;
            placement["is_mandatory"] = json!(false);
            Instruction::ReturnMinions {
                processed,
                movement: compile_step(&movement, steps, symbols)?,
                placement: compile_step(&placement, steps, symbols)?,
            }
        }
        "check_adjacency" => Instruction::CheckAdjacency {
            a: optional_symbol(raw, "unit_a_id", symbols)?,
            b: optional_symbol(raw, "unit_b_id", symbols)?,
            a_key: optional_symbol(raw, "unit_a_key", symbols)?,
            b_key: optional_symbol(raw, "unit_b_key", symbols)?,
            output: required_symbol(raw, "output_key", symbols)?,
        },
        "record_hex" => Instruction::RecordHex {
            unit: optional_symbol(raw, "unit_id", symbols)?,
            key: optional_symbol(raw, "unit_key", symbols)?,
            output: required_symbol(raw, "output_key", symbols)?,
        },
        "place_unit" => Instruction::Place {
            unit: optional_symbol(raw, "unit_id", symbols)?,
            key: optional_symbol(raw, "unit_key", symbols)?,
            destination: raw
                .get("target_hex_arg")
                .filter(|v| !v.is_null())
                .map(|v| serde_json::from_value(v.clone()).map_err(|e| unsupported(e.to_string())))
                .transpose()?,
            destination_key: symbols
                .intern(raw["destination_key"].as_str().unwrap_or("target_hex")),
        },
        "swap_units" => Instruction::Swap {
            a: optional_symbol(raw, "unit_a_id", symbols)?,
            b: optional_symbol(raw, "unit_b_id", symbols)?,
            a_key: optional_symbol(raw, "unit_a_key", symbols)?,
            b_key: optional_symbol(raw, "unit_b_key", symbols)?,
        },
        "create_effect" => Instruction::CreateEffect(Arc::new(
            crate::effect_steps::EffectStepSpec::compile(raw, symbols).map_err(unsupported)?,
        )),
        "log_message" => {
            let message = raw["message"]
                .as_str()
                .ok_or_else(|| unsupported("log message missing"))?;
            if message.contains(['{', '}']) {
                return Err(unsupported("formatted debug log not compiled"));
            }
            Instruction::Log
        }
        "force_discard_or_defeat" => {
            let key = required_symbol(raw, "victim_key", symbols)?;
            let mut select = combat_template("select_card")?;
            select["prompt"] = json!("template");
            select["output_key"] = json!("card_to_discard");
            select["decision_purpose"] = json!("discard");
            select["context_hero_id_key"] = raw["victim_key"].clone();
            select["override_player_id_key"] = raw["victim_key"].clone();
            let mut discard = combat_template("discard_card")?;
            discard["card_key"] = json!("card_to_discard");
            discard["hero_key"] = raw["victim_key"].clone();
            Instruction::DiscardOrDefeat {
                key,
                killer: optional_symbol(raw, "killer_id", symbols)?,
                killer_key: optional_symbol(raw, "killer_key", symbols)?,
                immunity: optional_symbol(raw, "immunity_source_id", symbols)?,
                select: compile_step(&select, steps, symbols)?,
                discard: compile_step(&discard, steps, symbols)?,
                defeat: compile_step(&combat_template("defeat")?, steps, symbols)?,
            }
        }
        "mine_path_choice" => Instruction::MineChoice {
            unit: optional_symbol(raw, "unit_id", symbols)?,
            key: optional_symbol(raw, "unit_key", symbols)?,
            destination: symbols.intern(raw["destination_key"].as_str().unwrap_or("target_hex")),
            range: i32::try_from(integer(raw, "range_val", 1)?)
                .map_err(|_| unsupported("mine range outside i32"))?,
            output: symbols.intern(raw["output_key"].as_str().unwrap_or("triggered_mine_ids")),
            prompt: symbols.intern("Choose which mines to move through"),
        },
        "trigger_mine" => {
            let ids = if raw["mine_ids"].is_null() {
                None
            } else {
                Some(
                    raw["mine_ids"]
                        .as_array()
                        .ok_or_else(|| unsupported("invalid explicit mine list"))?
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(|s| symbols.intern(s))
                                .ok_or_else(|| unsupported("invalid explicit mine identity"))
                        })
                        .collect::<Result<Vec<_>>>()?,
                )
            };
            Instruction::TriggerMines {
                ids,
                victim: optional_symbol(raw, "victim_id", symbols)?,
                ids_key: symbols
                    .intern(raw["mine_ids_key"].as_str().unwrap_or("triggered_mine_ids")),
                victim_key: symbols.intern(raw["victim_key"].as_str().unwrap_or("mine_victim_id")),
                rollback: symbols.intern("rollback_reanchor_pending"),
                blast: symbols.intern("mine_blast"),
                discard: None,
            }
        }
        "force_discard" => {
            let victim = optional_symbol(raw, "victim_id", symbols)?;
            let key = optional_symbol(raw, "victim_key", symbols)?;
            if victim.is_none() && key.is_none() {
                return Err(unsupported("forced discard requires victim source"));
            }
            let basic = raw
                .get("card_is_basic")
                .filter(|v| !v.is_null())
                .map(|v| {
                    v.as_bool()
                        .ok_or_else(|| unsupported("invalid forced discard basic filter"))
                })
                .transpose()?;
            let mut select = combat_template("select_card")?;
            select["prompt"] = json!("template");
            select["output_key"] = json!("card_to_discard");
            select["decision_purpose"] = json!("discard");
            let mut discard = combat_template("discard_card")?;
            discard["card_key"] = json!("card_to_discard");
            Instruction::ForceDiscard {
                victim,
                key,
                basic,
                immunity: optional_symbol(raw, "immunity_source_id", symbols)?,
                select: compile_step(&select, steps, symbols)?,
                discard: compile_step(&discard, steps, symbols)?,
            }
        }
        "discard_card" => {
            let mut passive = combat_template("passive")?;
            passive["trigger"] = json!("after_card_discard");
            let passive = compile_step(&passive, steps, symbols)?;
            let played = match raw["source"].as_str() {
                None | Some("HAND") => false,
                Some("PLAYED") => true,
                _ => return Err(unsupported("discard source not compiled")),
            };
            Instruction::Discard {
                hero: optional_symbol(raw, "hero_id", symbols)?,
                hero_key: optional_symbol(raw, "hero_key", symbols)?,
                card: optional_symbol(raw, "card_id", symbols)?,
                card_key: optional_symbol(raw, "card_key", symbols)?,
                played,
                passive,
            }
        }
        "check_passive_abilities" => Instruction::Passive {
            hero: optional_symbol(raw, "hero_id", symbols)?,
            trigger: required_symbol(raw, "trigger", symbols)?,
        },
        "offer_passive" => Instruction::OfferPassive {
            spec: crate::passives::Offer {
                card: required_symbol(raw, "card_id", symbols)?,
                trigger: required_symbol(raw, "trigger", symbols)?,
                optional: flag(raw, "is_optional", true)?,
                prompt: required_symbol(raw, "prompt", symbols)?,
                hero: optional_symbol(raw, "hero_id", symbols)?,
                provider: passive_provider_symbol(raw, symbols)?,
            },
            mark: compile_step(&combat_template("mark_passive")?, steps, symbols)?,
        },
        "mark_passive_used" => Instruction::MarkPassive {
            card: required_symbol(raw, "card_id", symbols)?,
            provider: passive_provider_symbol(raw, symbols)?,
        },
        "resolve_defense_text" | "resolve_on_block_effect" => {
            let on_block = raw["type"] == "resolve_on_block_effect";
            let mut enter = combat_template("set_actor")?;
            enter["actor_key"] = json!("defender_id");
            enter["save_key"] = json!(if on_block {
                "_pre_onblock_actor"
            } else {
                "_pre_defense_actor"
            });
            let mut exit = combat_template("set_actor")?;
            exit["actor_key"] = enter["save_key"].clone();
            exit["save_key"] = json!("_discard");
            let program = DefenseProgram {
                reflect_select: None,
                reflect_discard: None,
                deflect_select: None,
                deflect_discard: None,
                enter: compile_step(&enter, steps, symbols)?,
                exit: compile_step(&exit, steps, symbols)?,
                flag: compile_step(&combat_template("context")?, steps, symbols)?,
                place: compile_step(&combat_template("place_bounty")?, steps, symbols)?,
                remove: compile_step(&combat_template("remove_bounty")?, steps, symbols)?,
            };
            Instruction::DefenseText { on_block, program }
        }
        "place_marker" => {
            let mut passive = combat_template("passive")?;
            passive["trigger"] = json!("after_place_marker");
            Instruction::PlaceMarker {
                kind: serde_json::from_value(raw["marker_type"].clone())
                    .map_err(|e| unsupported(e.to_string()))?,
                target: optional_symbol(raw, "target_id", symbols)?,
                key: optional_symbol(raw, "target_key", symbols)?,
                value: integer(raw, "value", 0)?,
                passive: compile_step(&passive, steps, symbols)?,
            }
        }
        "remove_marker" => Instruction::RemoveMarker(
            serde_json::from_value(raw["marker_type"].clone())
                .map_err(|e| unsupported(e.to_string()))?,
        ),
        "finished_expiring_effect" => Instruction::FinishedEffect,
        "restore_action_type" => Instruction::RestoreAction,
        "restore_action_context" => Instruction::RestoreContext {
            other: flag(raw, "other_hero_action", false)?,
        },
        "resolve_pre_action_movement" => {
            let recipes =
                crate::templates::instructions().map_err(unsupported)?["pre_action_movement"]
                    .as_array()
                    .ok_or_else(|| unsupported("frozen pre-action movement body missing"))?
                    .clone();
            Instruction::PreMovement {
                hero: optional_symbol(raw, "hero_id", symbols)?,
                key: optional_symbol(raw, "hero_key", symbols)?,
                children: recipes
                    .iter()
                    .map(|step| compile_step(step, steps, symbols))
                    .collect::<Result<Vec<_>>>()?,
            }
        }
        "resolve_pre_action_discard" => Instruction::PreDiscard {
            hero: optional_symbol(raw, "hero_id", symbols)?,
            key: optional_symbol(raw, "hero_key", symbols)?,
            abort: flag(raw, "abort_on_defeat", true)?,
            force: compile_step(&combat_template("pre_force_discard")?, steps, symbols)?,
            defeat: compile_step(&combat_template("defeat")?, steps, symbols)?,
        },
        "remove_unit" => Instruction::Remove {
            unit: optional_symbol(raw, "unit_id", symbols)?,
            key: optional_symbol(raw, "unit_key", symbols)?,
        },
        "trigger_game_over" => Instruction::GameOver(GameOutcome {
            team: nullable_symbol(raw, "winner", symbols)?,
            hero: nullable_symbol(raw, "individual_winner_id", symbols)?,
            condition: required_symbol(raw, "condition", symbols)?,
        }),
        "defeat_unit" => {
            let remove = compile_step(&combat_template("remove_unit")?, steps, symbols)?;
            let over = compile_step(&combat_template("game_over")?, steps, symbols)?;
            Instruction::Defeat {
                protection: None,
                victim: optional_symbol(raw, "victim_id", symbols)?,
                key: optional_symbol(raw, "victim_key", symbols)?,
                killer: nullable_symbol(raw, "killer_id", symbols)?,
                assist: integer(raw, "assist_multiplier", 1)?,
                remove,
                over,
            }
        }
        "check_minion_protection" => Instruction::MinionProtection {
            spec: crate::protection::ProtectionBinding {
                minion: required_symbol(raw, "minion_id", symbols)?,
                killer: nullable_symbol(raw, "killer_id", symbols)?,
                tried: Arc::from(
                    raw["tried_effect_ids"]
                        .as_array()
                        .ok_or_else(|| unsupported("protection tried list missing"))?
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(|id| symbols.intern(id))
                                .ok_or_else(|| unsupported("invalid protection identity"))
                        })
                        .collect::<Result<Vec<_>>>()?,
                ),
            },
            remove: compile_step(&combat_template("remove_unit")?, steps, symbols)?,
            discard: compile_step(&combat_template("discard_card")?, steps, symbols)?,
        },
        "resolve_combat" => {
            let defeat = compile_step(&combat_template("defeat")?, steps, symbols)?;
            Instruction::Combat {
                damage: integer(raw, "damage", 0)?,
                target: symbols.intern(raw["target_key"].as_str().unwrap_or("victim_id")),
                defeat,
            }
        }
        // Arena-only primary recipes may be imported as inert templates by the
        // transitional dense path. Reaching them retains the pre-pop guard.
        "co_directional_drag" => Instruction::LiveDrag,
        "move_toward_target" => Instruction::LiveMoveToward,
        "set_card_initiative" => Instruction::LiveSetInitiative,
        "schedule_action_control" => Instruction::LiveActionControl,
        "schedule_journey_return" => Instruction::LiveJourney,
        "set_context_flag" => Instruction::Set {
            key: required_symbol(raw, "key", symbols)?,
            value: Datum::from_json(raw.get("value").unwrap_or(&Value::Bool(true)), symbols)
                .map_err(unsupported)?,
        },
        "add_context_value" => Instruction::Add {
            key: required_symbol(raw, "key", symbols)?,
            amount: integer(raw, "amount", 1)?,
        },
        "set_actor" => Instruction::SetActor {
            literal: optional_symbol(raw, "actor_id", symbols)?,
            key: optional_symbol(raw, "actor_key", symbols)?,
            save: symbols.intern(raw["save_key"].as_str().unwrap_or("saved_actor_id")),
        },
        "record_target" => Instruction::Record {
            input: required_symbol(raw, "input_key", symbols)?,
            output: required_symbol(raw, "output_list_key", symbols)?,
        },
        "flip_tie_breaker_coin" => Instruction::FlipCoin,
        "retrieve_card" => Instruction::Retrieve {
            card: symbols.intern(match raw.get("card_key") {
                None => "",
                Some(Value::String(s)) => s,
                _ => return Err(unsupported("invalid retrieval card key")),
            }),
            hero_key: optional_symbol(raw, "hero_key", symbols)?,
            all: flag(raw, "retrieve_all_discarded", false)?,
        },
        "retrieve_unresolved_card" => Instruction::RetrieveUnresolved {
            hero: required_symbol(raw, "hero_id", symbols)?,
            prompt: symbols.intern("Alternative Timelines: retrieve one of your unresolved cards."),
        },
        "choose_acting_piece" => Instruction::ChoosePiece {
            hero: required_symbol(raw, "hero_id", symbols)?,
            prompt: symbols.intern("Choose which of you performs this action."),
        },
        "set_acting_piece" => Instruction::SetPiece {
            hero: required_symbol(raw, "hero_id", symbols)?,
            piece: match raw.get("piece_id") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => Some(symbols.intern(s)),
                _ => return Err(unsupported("invalid acting-piece literal")),
            },
            key: optional_symbol(raw, "piece_key", symbols)?,
        },
        "fast_travel_sequence" => compile_travel_sequence(raw, steps, symbols)?,
        "fast_travel_unit" => Instruction::FastTravel {
            unit: optional_symbol(raw, "unit_id", symbols)?,
            destination: match raw.get("target_hex_arg") {
                None | Some(Value::Null) => None,
                Some(value) => Some(
                    serde_json::from_value(value.clone())
                        .map_err(|e| unsupported(format!("invalid travel destination: {e}")))?,
                ),
            },
            destination_key: symbols
                .intern(raw["destination_key"].as_str().unwrap_or("target_hex")),
            change: flag(raw, "require_zone_change", false)?,
        },
        "move_sequence" => compile_movement_sequence(raw, steps, symbols)?,
        "move_unit" => {
            Instruction::Move {
                mine_choice: None,
                mine_trigger: None,
                prechosen: flag(raw, "mine_path_prechosen", false)?,
                unit: optional_symbol(raw, "unit_id", symbols)?,
                unit_key: optional_symbol(raw, "unit_key", symbols)?,
                destination: match raw.get("target_hex_arg") {
                    None | Some(Value::Null) => None,
                    Some(value) => Some(serde_json::from_value(value.clone()).map_err(|e| {
                        unsupported(format!("invalid literal move destination: {e}"))
                    })?),
                },
                destination_key: symbols
                    .intern(raw["destination_key"].as_str().unwrap_or("target_hex")),
                range: i32::try_from(integer(raw, "range_val", 1)?)
                    .map_err(|_| unsupported("movement range outside i32"))?,
                through: flag(raw, "pass_through_obstacles", false)?,
                straight: flag(raw, "force_straight_line", false)?,
                crossed: optional_symbol(raw, "crossed_obstacles_output_key", symbols)?,
                success: optional_symbol(raw, "success_output_key", symbols)?,
                is_action: flag(raw, "is_movement_action", false)?,
            }
        }
        "select_step" => {
            if raw["target_type"] == "CARD" {
                Instruction::CardSelect(Arc::new(
                    CardSelectSpec::compile(raw, symbols).map_err(unsupported)?,
                ))
            } else if raw["target_type"] == "UNIT" || raw["target_type"] == "UNIT_OR_TOKEN" {
                let filters = crate::targeting::compile_effective_unit_filters(raw, symbols, false)
                    .map_err(unsupported)?;
                Instruction::Unit {
                    output: symbols.intern(raw["output_key"].as_str().unwrap_or("selection")),
                    filters,
                    tokens: raw["target_type"] == "UNIT_OR_TOKEN",
                    prompt: required_symbol(raw, "prompt", symbols)?,
                    auto: flag(raw, "auto_select_if_one", false)?,
                    owner: optional_symbol(raw, "override_player_id", symbols)?,
                    owner_key: optional_symbol(raw, "override_player_id_key", symbols)?,
                    discard: raw["decision_purpose"] == "discard",
                }
            } else if raw["target_type"] == "HEX" {
                let filters = raw["filters"]
                    .as_array()
                    .ok_or_else(|| unsupported("hex filter list missing"))?
                    .iter()
                    .map(|v| compile_filter(v, symbols).map_err(unsupported))
                    .collect::<Result<Vec<_>>>()?;
                Instruction::Hex {
                    output: symbols.intern(raw["output_key"].as_str().unwrap_or("selection")),
                    filters,
                    prompt: required_symbol(raw, "prompt", symbols)?,
                    auto: flag(raw, "auto_select_if_one", false)?,
                    owner: optional_symbol(raw, "override_player_id", symbols)?,
                    owner_key: optional_symbol(raw, "override_player_id_key", symbols)?,
                    discard: raw["decision_purpose"] == "discard",
                }
            } else {
                if raw["target_type"] != "NUMBER"
                    || raw["filters"].as_array().is_some_and(|v| !v.is_empty())
                {
                    return Err(unsupported("selection target/filter closure not compiled"));
                }
                let options = raw["number_options"]
                    .as_array()
                    .ok_or_else(|| unsupported("number options missing"))?
                    .iter()
                    .map(|v| {
                        v.as_i64()
                            .ok_or_else(|| unsupported("invalid number option"))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let mut labels = Vec::new();
                if let Some(fields) = raw["number_labels"].as_object() {
                    for (number, label) in fields {
                        labels.push((
                            number
                                .parse()
                                .map_err(|_| unsupported("invalid number label index"))?,
                            symbols.intern(
                                label
                                    .as_str()
                                    .ok_or_else(|| unsupported("invalid number label"))?,
                            ),
                        ));
                    }
                }
                Instruction::Number {
                    output: symbols.intern(raw["output_key"].as_str().unwrap_or("selection")),
                    options,
                    labels,
                    prompt: required_symbol(raw, "prompt", symbols)?,
                    auto: flag(raw, "auto_select_if_one", false)?,
                    owner: optional_symbol(raw, "override_player_id", symbols)?,
                    owner_key: optional_symbol(raw, "override_player_id_key", symbols)?,
                    discard: raw["decision_purpose"] == "discard",
                }
            }
        }
        "ask_confirmation" => Instruction::Confirm {
            output: symbols.intern(raw["output_key"].as_str().unwrap_or("confirmation")),
            prompt: required_symbol(raw, "prompt", symbols)?,
            owner: optional_symbol(raw, "player_id", symbols)?,
        },
        "count" | "collect_units" => {
            let collect = raw["type"] == "collect_units";
            Instruction::Query {
                spec: crate::queries::Query::compile(raw, symbols, collect).map_err(unsupported)?,
                collect,
                output: symbols.intern(raw["output_key"].as_str().unwrap_or(if collect {
                    "collected_units"
                } else {
                    "count_result"
                })),
            }
        }
        "check_context_condition" => Instruction::ContextCondition {
            input: required_symbol(raw, "input_key", symbols)?,
            comparison: crate::queries::Comparison::parse(raw["operator"].as_str().unwrap_or(">=")),
            threshold: integer(raw, "threshold", 1)?,
            output: symbols.intern(raw["output_key"].as_str().unwrap_or("condition_met")),
        },
        "may_repeat_once" => {
            let templates = raw["steps_template"]
                .as_array()
                .ok_or_else(|| unsupported("repeat template missing"))?
                .iter()
                .map(|v| compile_step(v, steps, symbols))
                .collect::<Result<Vec<_>>>()?;
            Instruction::Repeat {
                max: integer(raw, "max_repeats", 1)?,
                prompt: required_symbol(raw, "prompt", symbols)?,
                templates,
                check: compile_step(&combat_template("check_lane_push")?, steps, symbols)?,
            }
        }
        "validate_repeat" => Instruction::ValidateRepeat {
            actor: optional_symbol(raw, "actor_id", symbols)?,
            and_key: optional_symbol(raw, "and_with_key", symbols)?,
            output: required_symbol(raw, "output_key", symbols)?,
        },
        "for_each" => {
            let templates = raw["steps_template"]
                .as_array()
                .ok_or_else(|| unsupported("ForEach template missing"))?
                .iter()
                .map(|v| compile_step(v, steps, symbols))
                .collect::<Result<Vec<_>>>()?;
            Instruction::ForEach {
                list: required_symbol(raw, "list_key", symbols)?,
                item: required_symbol(raw, "item_key", symbols)?,
                templates,
            }
        }
        other => return Err(unsupported(format!("step type not compiled: {other}"))),
    };
    let index = integer(
        raw,
        if raw["type"] == "may_repeat_once" {
            "repeats_done"
        } else {
            "current_index"
        },
        0,
    )?;
    let pending = match raw.get("pending_input") {
        None | Some(Value::Null) => None,
        Some(value) => Some(Datum::from_json(value, symbols).map_err(unsupported)?),
    };
    let request = optional_symbol(raw, "pending_request_id", symbols)?.map(RequestToken::Imported);
    let id = steps.len();
    let initial = Frame {
        instruction: id,
        object: None,
        pending,
        request,
        index,
        repeat_checked: flag(raw, "lane_push_checked", false)?,
        unit_override: None,
        generated: None,
        defeat_prompt_victim: None,
        range_override: None,
        through_override: None,
        defeat_override: None,
        outcome_override: None,
        discard_override: None,
        attack_binding: None,
        attack_followup: None,
        effect: None,
        offer_override: None,
        forced: None,
        mine_trigger: None,
        mine_snapshot: None,
        protection: None,
        mine_prechosen: false,
        pre_move: None,
        pre_processed: raw["processed_effect_ids"]
            .as_array()
            .map(|ids| {
                ids.iter()
                    .map(|id| {
                        id.as_str()
                            .map(|id| symbols.intern(id))
                            .ok_or_else(|| unsupported("invalid processed effect ID"))
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default(),
        action: None,
        zone_return: None,
        battle: None,
        continuation: None,
        multi: raw
            .get("selections")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(|s| symbols.intern(s))
                            .ok_or_else(|| unsupported("invalid saved multi-selection"))
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default(),
    };
    steps.push(CompiledStep(Arc::new(StepProgram {
        primary_template: false,
        conditions,
        instruction,
        wire: raw.clone(),
        initial,
    })));
    Ok(id)
}

fn bind_zone_return(
    instruction: &mut Instruction,
    binding: &crate::minion_return::ReturnBinding,
) -> Result<()> {
    use crate::minion_return::ReturnBinding;
    match (instruction, binding) {
        (Instruction::ReturnMinions { processed, .. }, ReturnBinding::Processed(ids)) => {
            *processed = ids.clone()
        }
        (
            Instruction::Move {
                unit,
                destination,
                range,
                ..
            },
            ReturnBinding::Relocate {
                unit: id,
                destination: hex,
                range: Some(distance),
            },
        ) => {
            *unit = Some(*id);
            *destination = Some(*hex);
            *range = *distance;
        }
        (
            Instruction::Place {
                unit, destination, ..
            },
            ReturnBinding::Relocate {
                unit: id,
                destination: hex,
                range: None,
            },
        ) => {
            *unit = Some(*id);
            *destination = Some(*hex);
        }
        _ => return Err(unsupported("invalid minion return continuation binding")),
    }
    Ok(())
}

fn compile_battle_program(
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
) -> Result<BattleProgram> {
    Ok(BattleProgram {
        remove: compile_step(&combat_template("remove_unit")?, steps, symbols)?,
        place: compile_step(&combat_template("place_unit")?, steps, symbols)?,
        victory: compile_step(&combat_template("game_over")?, steps, symbols)?,
        push: compile_step(&combat_template("lane_push")?, steps, symbols)?,
        casualties: compile_step(&combat_template("minion_removal")?, steps, symbols)?,
        displace: compile_step(&combat_template("displacement")?, steps, symbols)?,
    })
}
fn card_error(e: crate::cards::CardError) -> EngineError {
    match e {
        crate::cards::CardError::Illegal(s) => EngineError::Rule(s.into()),
        crate::cards::CardError::Unsupported(s) => unsupported(s),
    }
}
fn compile_flow_program(
    steps: &mut Vec<CompiledStep>,
    symbols: &mut Symbols,
    round_only: bool,
) -> Result<FlowProgram> {
    // Active-effect phase roots currently admit cleanup/upgrades/reset only.
    // Compile their reachable continuations, not unported future action flows.
    let names: &[&str] = if round_only {
        &["upgrades", "round_reset"]
    } else {
        &[
            "resolve_card",
            "confirm_resolution",
            "finalize_hero",
            "respawn",
            "tie",
            "find_actor",
            "retrieve_unresolved",
            "flip_coin",
            "return_minions",
            "check_lane_push",
            "advance_turn",
            "end_phase",
            "end_cleanup",
            "upgrades",
            "round_reset",
            "minion_battle",
        ]
    };
    let entries = names
        .iter()
        .map(|&name| Ok((name, compile_step(&combat_template(name)?, steps, symbols)?)))
        .collect::<Result<Vec<_>>>()?;
    Ok(FlowProgram { entries })
}
fn bind_continuation(
    instruction: &mut Instruction,
    cont: &crate::planning::Continuation,
) -> Result<()> {
    use crate::{planning::Continuation as C, turn::FlowStep as F};
    match (instruction, cont) {
        (Instruction::ResolveCard { hero, .. }, C::Resolve(id))
        | (Instruction::RetrieveUnresolved { hero, .. }, C::Retrieve(id)) => *hero = *id,
        (Instruction::Flow(F::Confirm(hero)), C::Confirm(id))
        | (Instruction::Flow(F::Finalize(hero)), C::Finalize(id))
        | (Instruction::Flow(F::Respawn(hero)), C::Respawn(id)) => *hero = *id,
        (Instruction::Flow(F::Tie(tied)), C::Tie(ids)) => tied.clone_from(ids),
        (Instruction::Flow(F::Next), C::FindActor) | (Instruction::FlipCoin, C::FlipCoin) => {}
        _ => return Err(unsupported("invalid flow continuation binding")),
    }
    Ok(())
}
fn patch_continuation(wire: &mut Value, cont: &crate::planning::Continuation, symbols: &Symbols) {
    use crate::planning::Continuation as C;
    match cont {
        C::Resolve(id) | C::Confirm(id) | C::Finalize(id) | C::Respawn(id) | C::Retrieve(id) => {
            wire["hero_id"] = json!(symbols.name(*id))
        }
        C::Tie(ids) => {
            wire["tied_hero_ids"] =
                json!(ids.iter().map(|id| symbols.name(*id)).collect::<Vec<_>>())
        }
        _ => {}
    }
}

fn protection_selection(value: &Datum, symbols: &Symbols, selection: Symbol) -> Option<Datum> {
    if let Datum::Object(fields) = value {
        symbols
            .lookup("selected_card_id")
            .and_then(|key| fields.iter().find(|(k, _)| *k == key).map(|(_, v)| v))
            .filter(|v| v.truthy(symbols))
            .or_else(|| fields.iter().find(|(k, _)| *k == selection).map(|(_, v)| v))
            .cloned()
    } else {
        Some(value.clone())
    }
}
fn passive_provider_symbol(raw: &Value, symbols: &mut Symbols) -> Result<Option<Symbol>> {
    match raw.get("active_effect_id") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) => Ok(Some(symbols.intern(id))),
        _ => Err(unsupported("invalid passive provider identity")),
    }
}
fn bind_passive_owner(value: &mut Value, owner: &str) {
    match value {
        Value::String(s) if s == "template-passive-owner" => *s = owner.to_owned(),
        Value::Array(values) => {
            for value in values {
                bind_passive_owner(value, owner);
            }
        }
        Value::Object(fields) => {
            for value in fields.values_mut() {
                bind_passive_owner(value, owner);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
