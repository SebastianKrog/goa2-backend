//! Native stack execution. Definitions are compiled once; advancing uses typed
//! instructions/registers and does not serialize JSON or call Python.
//! Unrecognized instruction families reject at import. Dynamic rule closures
//! fail explicitly when reached; no approximate rule fallback is used.
use crate::{
    actions::{ActionMenu, ActionResolution, ActionRules, ActionStep, Passive},
    attack::{AttackBinding, AttackRules, AttackSpec},
    card_selection::{CardSelectSpec, ForceBinding},
    codec::{Phase, StateEnvelope},
    combat::{CombatError, CombatEvent, CombatRules, DefeatBinding, GameOutcome},
    data::{Datum, Registers, Symbol, Symbols},
    geometry::Hex,
    markers::{MarkerEvent, MarkerPlacement, MarkerRules},
    primary::{PrimaryOperands, PrimaryValues},
    reactions::{DefenseMenu, DefenseOp, ReactionRules, Window},
    spatial::{HexFilter, Origin, SpatialContext, compile_filter},
    stats::{CardStats, MarkerKind, Stat},
    targeting::UnitFilter,
    world::{EntityKind, NativeWorld},
};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineError {
    Unsupported(String),
    InvalidInput(String),
    Rule(String),
    Cancelled,
    Deadline,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestToken {
    Imported(Symbol),
    Generated(u64),
}
#[derive(Clone, Debug)]
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
    reflect_select: usize,
    reflect_discard: usize,
    deflect_select: usize,
    deflect_discard: usize,
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
struct Frame {
    instruction: usize,
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
#[derive(Clone, Debug)]
enum Instruction {
    CreateEffect(Arc<crate::effect_steps::EffectStepSpec>),
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
        body: usize,
        mark: usize,
    },
    MarkPassive {
        card: Symbol,
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
    PreAction,

    Combat {
        damage: i64,
        target: Symbol,
        defeat: usize,
    },
    Defeat {
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
struct CompiledStep {
    conditions: Conditions,
    instruction: Instruction,
    wire: Value,
    initial: Frame,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Team {
    Red,
    Blue,
}
#[derive(Clone, Debug)]
pub enum RequestChoices {
    Numbers {
        values: Vec<i64>,
        labels: Vec<(i64, Symbol)>,
    },
    Units {
        tokens: bool,
        values: Vec<(Symbol, Option<(Symbol, Hex)>)>,
    },
    YesNo,
    Repeat {
        done: i64,
        max: i64,
    },
    Passive {
        card: Symbol,
        name: Symbol,
    },
    ConfirmAction,
    Upgrades(Vec<crate::round::UpgradePlayer>),
    Actors {
        team: Symbol,
        values: Vec<Symbol>,
    },
    Respawn {
        hexes: Vec<Hex>,
        location: bool,
    },
    Cards(Vec<(Symbol, Symbol, i64)>),
    CardIds {
        values: Vec<Symbol>,
        forced_victim: Option<Symbol>,
        defeat_victim: Option<Symbol>,
    },
    Multiple {
        values: Vec<Symbol>,
        count: usize,
        max: i64,
    },
    Action(ActionMenu),
    Defense(DefenseMenu),
    Hexes(Vec<Hex>),
    ZoneHexes {
        values: Vec<Hex>,
        unit: Option<Symbol>,
    },
}
#[derive(Clone, Debug)]
pub struct NativeRequest {
    pub id: RequestToken,
    pub owner: Symbol,
    prompt: Symbol,
    movement_range: Option<i32>,
    pub choices: RequestChoices,
    can_skip: bool,
    discard: bool,
}
#[derive(Clone, Debug)]
pub enum NativeEvent {
    EffectCreated(crate::effect_steps::EffectCreated),
    Round(crate::round::RoundEvent),
    TurnEnded(Symbol),
    RespawnCleared {
        hero: Symbol,
        token: Symbol,
        from: Hex,
    },
    Pushed {
        actor: Option<Symbol>,
        target: Symbol,
        token: bool,
        from: Hex,
        to: Hex,
        distance: usize,
        collision: bool,
    },
    TokenMoved {
        actor: Option<Symbol>,
        token: Symbol,
        from: Option<Hex>,
        to: Hex,
    },
    Swapped {
        a: Symbol,
        b: Symbol,
        from: Hex,
        to: Hex,
    },
    TokenRemoved {
        actor: Option<Symbol>,
        token: Symbol,
        from: Hex,
    },
    Marker(MarkerEvent),
    Combat(CombatEvent),
    UnitPlaced {
        unit: Symbol,
        from: Option<Hex>,
        to: Hex,
    },
    CardRetrieved {
        hero: Symbol,
        card: Symbol,
        name: Symbol,
        kept: Option<Symbol>,
    },
    TieBreakerFlipped {
        blue: bool,
    },
    UnitMoved {
        unit: Symbol,
        from: Hex,
        to: Hex,
        range: i32,
    },
}
#[derive(Clone, Debug, Default)]
pub struct AdvanceResult {
    pub request: Option<NativeRequest>,
    pub events: Vec<NativeEvent>,
    pub steps: usize,
}
pub struct ExecutionBudget<'a> {
    pub deadline: Option<Instant>,
    pub cancelled: Option<&'a AtomicBool>,
    pub max_steps: usize,
}
impl Default for ExecutionBudget<'_> {
    fn default() -> Self {
        Self {
            deadline: None,
            cancelled: None,
            max_steps: 1000,
        }
    }
}
impl ExecutionBudget<'_> {
    fn check(&self, steps: usize) -> Result<()> {
        if self.cancelled.is_some_and(|v| v.load(Ordering::Relaxed)) {
            return Err(EngineError::Cancelled);
        }
        if self.deadline.is_some_and(|v| Instant::now() >= v) {
            return Err(EngineError::Deadline);
        }
        if steps >= self.max_steps.min(1000) {
            return Err(EngineError::StepLimit);
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct NativeSession {
    // These JSON definitions are touched only by compile/export, never advance.
    source: Arc<StateEnvelope>,
    steps: Arc<Vec<CompiledStep>>,
    symbols: Arc<Symbols>,
    context: Registers,
    world: NativeWorld,
    combat_rules: CombatRules,
    marker_rules: MarkerRules,
    attack_rules: AttackRules,
    action_rules: ActionRules,
    primary_operands: PrimaryOperands,
    return_rules: Arc<crate::minion_return::ReturnRules>,
    battle_rules: Option<Arc<crate::battle::BattleRules>>,
    battle_program: Option<Arc<BattleProgram>>,
    reaction_rules: Arc<ReactionRules>,
    passive_rules: Arc<crate::passives::PassiveRules>,
    passive_offer: Option<usize>,
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
    selection_key: Symbol,
    action_stack_key: Symbol,
    yes: Symbol,
    skip: Symbol,
    no_actor: Symbol,
    coordinates: [Symbol; 3],
}
impl NativeSession {
    pub fn compile(source: StateEnvelope) -> Result<Self> {
        let header = source.validate().map_err(unsupported)?;
        if !matches!(
            header.phase,
            Phase::Planning | Phase::Resolution | Phase::Cleanup | Phase::LevelUp | Phase::GameOver
        ) {
            return Err(unsupported("phase flow not yet compiled"));
        }
        let raw = &source.state;
        let has_effects = raw["active_effects"]
            .as_array()
            .is_some_and(|v| !v.is_empty());
        let lifecycle_only = matches!(header.phase, Phase::Resolution)
            && raw["execution_stack"].as_array().is_some_and(|stack| {
                stack.iter().all(|step| {
                    matches!(
                        step["type"].as_str(),
                        Some("create_effect" | "set_context_flag" | "log_message")
                    )
                })
            });
        if has_effects && !lifecycle_only {
            return Err(unsupported(
                "active-effect and action-controller closure not yet compiled",
            ));
        }
        let mut symbols = Symbols::default();
        let selection_key = symbols.intern("selection");
        let action_stack_key = symbols.intern("action_context_stack");
        let yes = symbols.intern("YES");
        let skip = symbols.intern("SKIP");
        let no_actor = symbols.intern("None");
        let coordinates = ["q", "r", "s"].map(|name| symbols.intern(name));
        let planning = if has_effects {
            crate::planning::Planning::compile_effect_boundary(&source, &mut symbols)
        } else {
            crate::planning::Planning::compile_flow(&source, &mut symbols)
        }
        .map_err(unsupported)?;
        let team = match raw["tie_breaker_team"].as_str() {
            Some("RED") => Team::Red,
            Some("BLUE") => Team::Blue,
            _ => return Err(unsupported("invalid tie-breaker team")),
        };
        let combat_rules = CombatRules::compile(&mut symbols);
        let marker_rules = MarkerRules::compile(&mut symbols);
        let attack_rules = AttackRules::compile(&mut symbols);
        let action_rules = ActionRules::compile(&mut symbols);
        let primary_operands = PrimaryOperands::compile(&mut symbols);
        let reaction_rules = Arc::new(ReactionRules::compile(&mut symbols).map_err(unsupported)?);
        let mut context_items = Vec::new();
        for (key, value) in raw["execution_context"]
            .as_object()
            .ok_or_else(|| unsupported("invalid execution context"))?
        {
            context_items.push((
                symbols.intern(key),
                Datum::from_json(value, &mut symbols).map_err(unsupported)?,
            ));
        }
        let mut steps = Vec::new();
        let mut stack = Vec::new();
        for wire in raw["execution_stack"]
            .as_array()
            .ok_or_else(|| unsupported("invalid execution stack"))?
        {
            let id = compile_step(wire, &mut steps, &mut symbols)?;
            stack.push(steps[id].initial.clone());
        }
        let has_flow = header.phase == Phase::Planning
            || steps
                .iter()
                .any(|s| matches!(s.instruction, Instruction::Flow(_)));
        let flow_program = if has_flow {
            Some(Arc::new(compile_flow_program(&mut steps, &mut symbols)?))
        } else {
            None
        };
        let boundary =
            crate::turn::Boundary::compile(&source, &mut symbols).map_err(unsupported)?;
        let round =
            crate::round::RoundState::compile(&source, &mut symbols).map_err(unsupported)?;
        let round_rules = if has_flow {
            Some(Arc::new(
                crate::round::RoundRules::compile(&mut symbols).map_err(unsupported)?,
            ))
        } else {
            None
        };
        let world = NativeWorld::compile(&source, &mut symbols).map_err(unsupported)?;
        let passive_rules = Arc::new(
            crate::passives::PassiveRules::compile(&world, &mut symbols).map_err(unsupported)?,
        );
        let passive_offer = if steps
            .iter()
            .any(|s| matches!(s.instruction, Instruction::Passive { .. }))
        {
            Some(compile_step(
                &combat_template("offer_passive")?,
                &mut steps,
                &mut symbols,
            )?)
        } else {
            None
        };
        let turn_rules =
            has_flow.then(|| Arc::new(crate::turn::TurnRules::compile(&world, &mut symbols)));
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
                world.ensure_combat_closure().map_err(unsupported)?;
                for hero in &world.heroes {
                    world
                        .lifecycle_hero(hero.id)
                        .map_err(|e| unsupported(e.to_string()))?;
                }
            }
            if matches!(
                step.instruction,
                Instruction::FastTravel { .. } | Instruction::TravelSequence { .. }
            ) || matches!(&step.instruction,Instruction::Hex {filters,..} if filters.iter().any(|f|matches!(f,HexFilter::FastTravel {..})))
            {
                world.ensure_fast_travel_closure().map_err(unsupported)?;
            }
            if let Instruction::Unit { .. } = step.instruction {
                world.ensure_target_closure().map_err(unsupported)?;
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
        Ok(Self {
            source: Arc::new(source),
            steps: Arc::new(steps),
            symbols: Arc::new(symbols),
            context,
            world,
            combat_rules,
            marker_rules,
            attack_rules,
            action_rules,
            primary_operands,
            return_rules,
            battle_rules,
            battle_program,
            reaction_rules,
            passive_rules,
            passive_offer,
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
            selection_key,
            action_stack_key,
            yes,
            skip,
            no_actor,
            coordinates,
        })
    }
    fn multi_choices(
        &self,
        tokens: bool,
        filters: &[UnitFilter],
        frame: &Frame,
    ) -> Result<Vec<Symbol>> {
        let context = SpatialContext {
            world: &self.world,
            registers: &self.context,
            symbols: &self.symbols,
            actor: self.planning.actor,
            frame: frame.unit_override,
            coordinates: self.coordinates,
            range_override: None,
            through_override: None,
        };
        Ok(self
            .world
            .select_units(tokens, self.planning.actor, filters, &context)
            .map_err(unsupported)?
            .into_iter()
            .filter(|id| !frame.multi.contains(id))
            .collect())
    }
    fn normalized_owner(&self, actor: Symbol) -> Symbol {
        self.world.hero(actor).map_or(actor, |hero| hero.id)
    }
    fn symbol_from_context(&self, key: Symbol) -> Result<Option<Symbol>> {
        match self.context.get(key) {
            Some(Datum::Text(id)) if !self.symbols.name(*id).is_empty() => Ok(Some(*id)),
            Some(value) if value.truthy(&self.symbols) => {
                Err(unsupported("non-string actor/controller context"))
            }
            _ => Ok(None),
        }
    }
    fn prompt_owner(&self, owner: Option<Symbol>, owner_key: Option<Symbol>) -> Result<Symbol> {
        let raw = if owner.is_some() {
            owner
        } else if let Some(key) = owner_key {
            self.symbol_from_context(key)?.or(self.planning.actor)
        } else {
            self.planning.actor
        };
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
            self.world.history.discarded.clear();
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
                    self.world.history.discarded.clear();
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
                )
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
            result.steps += 1;
            let mut frame = self.stack.pop().unwrap();
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
                continue;
            }
            // Instructions are immutable and shared by cloned sessions. A clone
            // here contains only this small primitive payload, never a game state.
            let mut instruction = compiled.instruction.clone();
            if let Some(action) = &frame.action {
                bind_action(&mut instruction, action, &self.symbols)?;
            }
            if let Some(values) = frame.effect.as_ref().and_then(|e| e.primary.as_ref()) {
                bind_primary(&mut instruction, values, &self.primary_operands)?;
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
                    Instruction::CreateEffect(_) | Instruction::Set { .. } | Instruction::Log
                )
            {
                return Err(unsupported(
                    "active-effect gameplay/controller consumers not yet compiled",
                ));
            }
            let mut request = None;
            match instruction {
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
                                self.world
                                    .resolve_current_card(hero, Some(self.world.combat.turn))
                                    .map_err(card_error)?;
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
                            frame.pending = None;
                            if selected == Some(Datum::Text(rules.confirm)) {
                                continue;
                            }
                            request = Some(NativeRequest {
                                id: RequestToken::Generated(0),
                                owner: hero,
                                prompt: rules.confirm_prompt,
                                movement_range: None,
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
                                    id: RequestToken::Generated(0),
                                    owner: rules.owners[index],
                                    prompt: rules.confirm_prompt,
                                    movement_range: None,
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
                            let candidates = rules.respawn_candidates(&self.world, hero);
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
                                id: RequestToken::Generated(0),
                                owner: hero,
                                prompt: if location { *hex } else { *choice },
                                movement_range: None,
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
                                id: RequestToken::Generated(0),
                                owner: rules.simultaneous,
                                prompt: rules.prompt,
                                movement_range: None,
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
                                id: RequestToken::Generated(0),
                                owner: p.owner,
                                prompt: p.prompt,
                                movement_range: None,
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
                        .next(&self.world, &self.symbols, &processed, favored)
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
                        id: RequestToken::Generated(0),
                        owner: plan.owner,
                        prompt: plan.prompt,
                        movement_range: None,
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
                    if self.planning.actor.is_none() {
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
                        id: RequestToken::Generated(0),
                        owner: self.normalized_owner(self.planning.actor.unwrap()),
                        prompt,
                        movement_range: None,
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
                    if self.world.has_effects {
                        return Err(unsupported("token-bound effect removal not compiled"));
                    }
                    if let Some(from) =
                        self.world.remove_entity(token).map_err(EngineError::Rule)?
                    {
                        result.events.push(NativeEvent::TokenRemoved {
                            actor: self.planning.actor,
                            token,
                            from,
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
                                id: RequestToken::Generated(0),
                                owner: hero,
                                prompt: menu.name,
                                movement_range: None,
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
                    if c.facedown {
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
                            if self.world.has_effects {
                                return Err(unsupported("effect-aware adjacency not compiled"));
                            }
                            from.distance(to) == 1
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
                    if self.world.has_effects {
                        return Err(unsupported("placement prevention not compiled"));
                    }
                    if self.world.entity(unit).is_none() || self.world.occupant(to).is_some() {
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
                            actor: self.planning.actor,
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
                    if self.world.has_effects {
                        return Err(unsupported("swap prevention not compiled"));
                    }
                    if self.world.entity(a).is_none() || self.world.entity(b).is_none() {
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
                            frame.forced,
                        )
                        .map_err(unsupported)?;
                    let choices = spec
                        .choices(
                            &self.world,
                            &self.context,
                            &self.symbols,
                            owner,
                            frame.forced,
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
                        id: RequestToken::Generated(0),
                        owner: self.normalized_owner(owner.unwrap_or(self.no_actor)),
                        prompt: spec.prompt,
                        movement_range: None,
                        choices: RequestChoices::CardIds {
                            values: choices
                                .iter()
                                .map(|id| {
                                    self.world.definitions.cards
                                        [self.world.card(*id).unwrap().definition]
                                        .printed_id
                                })
                                .collect(),
                            forced_victim: frame.forced.map(|f| f.victim),
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
                    if !hero.hand.iter().any(|id| {
                        self.world.card(*id).is_some_and(|c| {
                            basic.is_none_or(|b| {
                                self.world.definitions.cards[c.definition].basic == b
                            })
                        })
                    }) {
                        continue;
                    }
                    if self.world.has_effects {
                        return Err(unsupported(if immunity.is_some() {
                            "forced-discard immunity/shield closure not compiled"
                        } else {
                            "forced-discard shield closure not compiled"
                        }));
                    }
                    for id in [discard, select] {
                        let mut child = self.generated_frame(id, None, None)?;
                        child.forced = Some(ForceBinding { victim, basic });
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
                    if immunity.is_some() && self.world.has_effects {
                        return Err(unsupported(
                            "discard-or-defeat immunity closure not compiled",
                        ));
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
                                id: RequestToken::Generated(0),
                                owner: menu.owner,
                                prompt: target,
                                movement_range: None,
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
                        let hero = if let Some(forced) = frame.forced {
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
                            self.passive_offer.expect("passive offer program compiled"),
                            None,
                            None,
                        )?;
                        child.offer_override = Some(binding);
                        self.stack.push(child);
                    }
                }
                Instruction::OfferPassive { spec, body, mark } => {
                    let spec = frame.offer_override.unwrap_or(spec);
                    let Some(owner) = spec
                        .hero
                        .or(self.planning.actor)
                        .and_then(|h| self.world.hero(h).map(|h| h.id))
                    else {
                        continue;
                    };
                    let Some(card) =
                        crate::passives::PassiveRules::card(&self.world, owner, spec.card)
                    else {
                        continue;
                    };
                    let c = self.world.card(card).unwrap();
                    let d = &self.world.definitions.cards[c.definition];
                    if c.facedown
                        || !d.has_effect
                        || !self.reaction_rules.registered_effect(d.effect)
                    {
                        continue;
                    }
                    if spec.optional && selected.is_none() {
                        request = Some(NativeRequest {
                            id: RequestToken::Generated(0),
                            owner,
                            prompt: spec.prompt,
                            movement_range: None,
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
                                card,
                                spec.trigger,
                            )
                            .map_err(unsupported)?
                    {
                        let mut child = self.generated_frame(mark, None, None)?;
                        child.offer_override = Some(spec);
                        self.stack.push(child);
                        let child = self.generated_frame(body, None, None)?;
                        self.stack.push(child);
                    }
                }
                Instruction::MarkPassive { card } => {
                    let card = frame.offer_override.map_or(card, |o| o.card);
                    crate::passives::PassiveRules::mark(&mut self.world, self.planning.actor, card)
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
                                        program.deflect_select
                                    } else {
                                        program.reflect_select
                                    },
                                    None,
                                    None,
                                    Some(range),
                                ),
                                DefenseOp::ProjectileDiscard { deflect } => (
                                    if deflect {
                                        program.deflect_discard
                                    } else {
                                        program.reflect_discard
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
                Instruction::RestoreAction => self
                    .reaction_rules
                    .restore(&mut self.context)
                    .map_err(unsupported)?,
                Instruction::PreAction => {
                    if self.world.has_effects {
                        return Err(unsupported("pre-action effect execution not compiled"));
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
                    self.context
                        .set(save, self.planning.actor.map_or(Datum::Null, Datum::Text));
                    let next = if let Some(key) = key {
                        self.symbol_from_context(key)?
                    } else {
                        literal
                    };
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
                        id: RequestToken::Generated(0),
                        owner: hero,
                        prompt,
                        movement_range: None,
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
                        id: RequestToken::Generated(0),
                        owner: hero,
                        prompt,
                        movement_range: None,
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
                        .select_units(tokens, target_actor, &filters, &spatial)
                        .map_err(unsupported)?;
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
                        id: RequestToken::Generated(0),
                        owner: self.normalized_owner(target_actor.unwrap_or(self.no_actor)),
                        prompt,
                        movement_range: None,
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
                    let raw_owner = if let Some(owner) = owner {
                        Some(owner)
                    } else if let Some(key) = owner_key {
                        self.symbol_from_context(key)?.or(self.planning.actor)
                    } else {
                        self.planning.actor
                    };
                    let owner = self.normalized_owner(raw_owner.unwrap_or(self.no_actor));
                    request = Some(NativeRequest {
                        id: RequestToken::Generated(0),
                        owner,
                        prompt,
                        movement_range: None,
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
                        id: RequestToken::Generated(0),
                        owner: self.prompt_owner(owner, owner_key)?,
                        prompt,
                        movement_range: frame.range_override,
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
                    let through = unit
                        .is_some_and(|u| self.world.movement_aura_passes(u))
                        .then_some(true);
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
                        id: RequestToken::Generated(0),
                        owner: self.normalized_owner(owner),
                        prompt,
                        movement_range: None,
                        choices: RequestChoices::YesNo,
                        can_skip: false,
                        discard: false,
                    });
                }
                Instruction::Move {
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
                    let valid = if from == to {
                        range >= 0
                    } else {
                        self.world.path(from, to, range, through)
                    };
                    if !valid {
                        continue;
                    }
                    let line = if straight {
                        let Some(line) = from.line_to(to) else {
                            continue;
                        };
                        if line.iter().take(line.len().saturating_sub(1)).any(|hex| {
                            !self.world.on_map(*hex) || (!through && self.world.obstacle(*hex))
                        }) {
                            continue;
                        }
                        line
                    } else {
                        Vec::new()
                    };
                    if let Some(key) = crossed {
                        let hexes = line
                            .iter()
                            .take(line.len().saturating_sub(1))
                            .filter(|hex| self.world.obstacle(**hex))
                            .map(|hex| native_hex(*hex, self.coordinates))
                            .collect::<Vec<_>>();
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
                }
                Instruction::Query {
                    spec,
                    collect,
                    output,
                } => {
                    let spatial = SpatialContext {
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
                        id: RequestToken::Generated(0),
                        owner: actor,
                        prompt,
                        movement_range: None,
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
    pub fn commit_card(&mut self, hero: Symbol, card: crate::cards::CardId) -> Result<()> {
        let result = self.planning.commit(&mut self.world, hero, card);
        self.finish_planning_command(result)
    }
    pub fn uncommit_card(&mut self, hero: Symbol) -> Result<()> {
        let result = self.planning.uncommit(&mut self.world, hero);
        self.finish_planning_command(result)
    }
    pub fn pass_planning(&mut self, hero: Symbol) -> Result<()> {
        let result = self.planning.pass(&mut self.world, hero);
        self.finish_planning_command(result)
    }
    pub fn finish_planning(&mut self, hero: Symbol) -> Result<()> {
        let result = self.planning.finish(&mut self.world, hero);
        self.finish_planning_command(result)
    }
    pub fn submit(&mut self, request: RequestToken, selection: Datum) -> Result<()> {
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
    pub fn import_selection(&mut self, value: &Value) -> Result<Datum> {
        Datum::from_json(value, Arc::make_mut(&mut self.symbols)).map_err(unsupported)
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
            RequestToken::Imported(Arc::make_mut(&mut self.symbols).intern(value))
        }
    }
    /// Compact outer boundary preserves physical instance aliases on restore.
    pub fn export_envelope(&self) -> Result<StateEnvelope> {
        let mut source = (*self.source).clone();
        source.native_resume = Some(crate::codec::NativeResume {
            version: 1,
            request_high_water: self.next_request,
            step_high_water: self.next_step,
        });
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
                            values.patch_wire(&mut wire, &self.symbols);
                            if matches!(&self.steps[frame.instruction].instruction,Instruction::Attack{spec,..} if spec.card_ranged){wire["is_ranged"]=json!(values.ranged);}
                        }
                        if let Some(cont)=&frame.continuation {patch_continuation(&mut wire,cont,&self.symbols);}
                        if let Some(battle)=&frame.battle {battle.patch_wire(&mut wire,&self.symbols);}
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
                            if wire["type"] == "move_unit" {
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
                        if let Some(forced) = frame.forced {
                            if wire["type"] == "select_step" {
                                wire["context_hero_id"] = json!(self.symbols.name(forced.victim));
                                wire["override_player_id"] =
                                    json!(self.symbols.name(forced.victim));
                                wire["card_is_basic"] = json!(forced.basic);
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
    pub fn semantic_state(&self) -> Result<Value> {
        self.export_envelope()?.materialize().map_err(unsupported)
    }
    pub fn request_json(&self, request: &NativeRequest) -> Value {
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
            return json!({"id":self.request_token(request.id),"request_type":"CHOOSE_ACTOR","player_id":self.symbols.name(request.owner),
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
            return json!({"id":self.request_token(request.id),"request_type":if *location{"CHOOSE_RESPAWN_HEX"}else{"CHOOSE_RESPAWN"},"player_id":self.symbols.name(request.owner),
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
            RequestChoices::Defense(_) | RequestChoices::Action(_)|RequestChoices::Actors{..}|RequestChoices::Respawn{..}|RequestChoices::Upgrades(_)=>unreachable!(),
            RequestChoices::ConfirmAction=>("CHOOSE_ACTION",vec![json!({"id":"CONFIRM","text":"Confirm","metadata":{}})]),
            RequestChoices::Multiple{values,..}=>("SELECT_UNIT",values.iter().map(|id|json!({"id":self.symbols.name(*id),"text":self.symbols.name(*id),"metadata":{}})).collect()),
            RequestChoices::Units {tokens,values}=>(if *tokens {"SELECT_UNIT_OR_TOKEN"}else{"SELECT_UNIT"},values.iter().map(|(id,label)|{
                let text=label.map_or_else(||self.symbols.name(*id).to_owned(),|(name,hex)|{let(q,r,s)=hex.coordinates();format!("{} at ({q}, {r}, {s})",self.symbols.name(name))});
                json!({"id":self.symbols.name(*id),"text":text,"metadata":{}})
            }).collect()),
            RequestChoices::Hexes(values)|RequestChoices::ZoneHexes{values,..}=>("SELECT_HEX",values.iter().map(|hex|{
                let (q,r,s)=hex.coordinates();json!({"id":format!("hex_{q}_{r}_{s}"),"text":format!("({q},{r},{s})"),"metadata":{"hex":hex,"raw":hex}})
            }).collect()),
            RequestChoices::CardIds{values,..}=>("SELECT_CARD",values.iter().map(|id|json!({"id":self.symbols.name(*id),"text":self.symbols.name(*id),"metadata":{}})).collect()),
            RequestChoices::Cards(values)=>("SELECT_CARD",values.iter().map(|(id,name,initiative)|json!({"id":self.symbols.name(*id),"text":format!("{} (initiative {initiative})",self.symbols.name(*name)),"metadata":{}})).collect()),
            RequestChoices::Passive{..}=>("CONFIRM_PASSIVE",vec![json!({"id":"YES","text":"Yes","metadata":{}}),json!({"id":"NO","text":"No","metadata":{}})]),
            RequestChoices::Repeat{..} | RequestChoices::YesNo=>("SELECT_OPTION",vec![json!({"id":"YES","text":"Yes","metadata":{}}),json!({"id":"NO","text":"No","metadata":{}})]),
            RequestChoices::Numbers {values,labels}=>("SELECT_NUMBER",values.iter().map(|n|{
                let label=labels.iter().find(|(number,_)|number==n).map(|(_,id)|self.symbols.name(*id).to_owned()).unwrap_or_else(||n.to_string());
                json!({"id":n.to_string(),"text":label,"metadata":if labels.is_empty(){json!({})}else{json!({"raw":n})}})
            }).collect()),
        };
        let prompt = if let RequestChoices::Repeat { done, max } = &request.choices {
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
        json!({"id":self.request_token(request.id),"request_type":kind,"player_id":self.symbols.name(request.owner),
            "prompt":prompt,"options":options,"can_skip":request.can_skip,"can_rollback":false,"context":match &request.choices {RequestChoices::Passive{card,name}=>json!({"card_id":self.symbols.name(*card),"card_name":self.symbols.name(*name)}),RequestChoices::ZoneHexes{unit:Some(id),..}=>json!({"context_unit_id":self.symbols.name(*id)}),_=>json!({})},
            "decision_context":if request.discard {json!({"purpose":"discard","actor_id":self.symbols.name(request.owner),"source_card_id":null,"choice_facts":{}})}else{Value::Null}})
    }
    pub fn event_json(&self, event: &NativeEvent) -> Value {
        match event {
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
            NativeEvent::TokenRemoved { actor, token, from } => {
                json!({"event_type":"TOKEN_REMOVED","actor_id":actor.map(|id|self.symbols.name(id)),"target_id":self.symbols.name(*token),"from_hex":from,"to_hex":null,"metadata":{"effects_removed":0}})
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
            NativeEvent::TieBreakerFlipped { blue } => {
                json!({"event_type":"TIE_BREAKER_FLIPPED","actor_id":null,"target_id":null,"from_hex":null,"to_hex":null,
                "metadata":{"tie_breaker_team":if *blue {"BLUE"}else{"RED"},"coin_face":if *blue {"BLUE"}else{"ORANGE"}}})
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
            match filter {
                HexFilter::Obstacle { exclude, .. } => {
                    *exclude = Origin {
                        frame: true,
                        ..Default::default()
                    }
                }
                HexFilter::Movement { origin, .. }
                | HexFilter::Straight { origin, .. }
                | HexFilter::ClearLine { origin, .. } => {
                    *origin = Origin {
                        frame: true,
                        ..Default::default()
                    }
                }
                HexFilter::FastTravel { .. }
                | HexFilter::Spawn { .. }
                | HexFilter::AdjacentSpawn { .. }
                | HexFilter::AdjacentTags(_) => {}
                HexFilter::Range { frame_range, .. } => {
                    *frame_range = true;
                }
            }
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
                let id = compile_step(wire, steps, symbols)?;
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
    let number = |n: i64| PrimaryOperands::number(n, v).map_err(unsupported);
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
        Datum::Int(n) => *n = PrimaryOperands::number(*n, v).map_err(unsupported)?,
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
        (ActionStep::PreMovement(_) | ActionStep::PreDiscard(_), Instruction::PreAction) => {}
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
        "offer_passive" => {
            if raw.get("active_effect_id").is_some_and(|v| !v.is_null()) {
                return Err(unsupported("effect-backed passive offer not compiled"));
            }
            Instruction::OfferPassive {
                spec: crate::passives::Offer {
                    card: required_symbol(raw, "card_id", symbols)?,
                    trigger: required_symbol(raw, "trigger", symbols)?,
                    optional: flag(raw, "is_optional", true)?,
                    prompt: required_symbol(raw, "prompt", symbols)?,
                    hero: optional_symbol(raw, "hero_id", symbols)?,
                },
                body: compile_step(&combat_template("discard_or_defeat")?, steps, symbols)?,
                mark: compile_step(&combat_template("mark_passive")?, steps, symbols)?,
            }
        }
        "mark_passive_used" => {
            if raw.get("active_effect_id").is_some_and(|v| !v.is_null()) {
                return Err(unsupported("effect-backed passive usage not compiled"));
            }
            Instruction::MarkPassive {
                card: required_symbol(raw, "card_id", symbols)?,
            }
        }
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
            let projectile = |deflect: bool,
                              steps: &mut Vec<CompiledStep>,
                              symbols: &mut Symbols|
             -> Result<(usize, usize)> {
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
            };
            let (reflect_select, reflect_discard) = projectile(false, steps, symbols)?;
            let (deflect_select, deflect_discard) = projectile(true, steps, symbols)?;
            let program = DefenseProgram {
                reflect_select,
                reflect_discard,
                deflect_select,
                deflect_discard,
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
        "restore_action_type" => Instruction::RestoreAction,
        "resolve_pre_action_movement" | "resolve_pre_action_discard" => Instruction::PreAction,
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
                victim: optional_symbol(raw, "victim_id", symbols)?,
                key: optional_symbol(raw, "victim_key", symbols)?,
                killer: nullable_symbol(raw, "killer_id", symbols)?,
                assist: integer(raw, "assist_multiplier", 1)?,
                remove,
                over,
            }
        }
        "resolve_combat" => {
            let defeat = compile_step(&combat_template("defeat")?, steps, symbols)?;
            Instruction::Combat {
                damage: integer(raw, "damage", 0)?,
                target: symbols.intern(raw["target_key"].as_str().unwrap_or("victim_id")),
                defeat,
            }
        }
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
            if flag(raw, "mine_path_prechosen", false)? {
                return Err(unsupported("preselected mine triggers are not compiled"));
            }
            Instruction::Move {
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
    steps.push(CompiledStep {
        conditions,
        instruction,
        wire: raw.clone(),
        initial,
    });
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
) -> Result<FlowProgram> {
    let names = [
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
    ];
    let entries = names
        .into_iter()
        .map(|name| Ok((name, compile_step(&combat_template(name)?, steps, symbols)?)))
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
#[cfg(test)]
mod tests {
    use super::*;
    fn source(name: &str) -> StateEnvelope {
        #[derive(serde::Deserialize)]
        struct Entry {
            name: String,
            path: String,
        }
        #[derive(serde::Deserialize)]
        struct Index {
            schema_version: u32,
            cases: Vec<Entry>,
        }
        #[derive(serde::Deserialize)]
        struct Input {
            envelope: StateEnvelope,
        }
        static INDEX: std::sync::OnceLock<Index> = std::sync::OnceLock::new();
        let base =
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures"));
        let index = INDEX.get_or_init(|| {
            serde_json::from_reader(std::io::BufReader::new(
                std::fs::File::open(base.join("stack-traces.json")).unwrap(),
            ))
            .unwrap()
        });
        assert_eq!(index.schema_version, 2);
        let entry = index.cases.iter().find(|case| case.name == name).unwrap();
        let input: Input = serde_json::from_reader(std::io::BufReader::new(
            std::fs::File::open(base.join(&entry.path)).unwrap(),
        ))
        .unwrap();
        input.envelope
    }

    #[test]
    fn card_action_workers_run_distinct_secondary_choices_with_shared_programs() {
        let mut root = NativeSession::compile(source("action-execution-Arien-MOVEMENT")).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        assert!(matches!(request.choices, RequestChoices::Action(_)));
        let before = root.semantic_state().unwrap();
        let mut moving = root.clone();
        let mut holding = root.clone();
        let movement = moving.import_selection(&json!("MOVEMENT")).unwrap();
        let hold = holding.import_selection(&json!("HOLD")).unwrap();
        moving.submit(request.id, movement).unwrap();
        holding.submit(request.id, hold).unwrap();
        assert!(
            moving
                .advance(&ExecutionBudget::default())
                .unwrap()
                .request
                .is_some()
        );
        assert!(
            holding
                .advance(&ExecutionBudget::default())
                .unwrap()
                .request
                .is_none()
        );
        assert_eq!(
            holding.semantic_state().unwrap()["execution_context"]["action_done"],
            true
        );
        assert_eq!(root.semantic_state().unwrap(), before);
        assert!(Arc::ptr_eq(&moving.steps, &holding.steps));
        assert!(Arc::ptr_eq(
            &moving.world.definitions,
            &holding.world.definitions
        ));
    }
    #[test]
    fn generated_action_recipe_restores_before_movement_and_never_fakes_primary_text() {
        let mut session =
            NativeSession::compile(source("action-execution-Arien-MOVEMENT")).unwrap();
        let request = session
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let movement = session.import_selection(&json!("MOVEMENT")).unwrap();
        session.submit(request.id, movement).unwrap();
        session.advance_prefix(1).unwrap();
        let mut restored = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
        let first = session
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let second = restored
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let mut expected = session.request_json(&first);
        let mut actual = restored.request_json(&second);
        expected.as_object_mut().unwrap().remove("id");
        actual.as_object_mut().unwrap().remove("id");
        assert_eq!(expected, actual);
        let destination = match first.choices {
            RequestChoices::Hexes(v) => v[0],
            _ => panic!("movement prompt expected"),
        };
        let selected = serde_json::to_value(destination).unwrap();
        let a = session.import_selection(&selected).unwrap();
        let b = restored.import_selection(&selected).unwrap();
        session.submit(first.id, a).unwrap();
        restored.submit(second.id, b).unwrap();
        session.advance(&ExecutionBudget::default()).unwrap();
        restored.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(
            session.semantic_state().unwrap(),
            restored.semantic_state().unwrap()
        );
        let mut envelope = source("action-execution-Arien-MOVEMENT");
        for definition in &mut envelope.definitions {
            if definition.fields["id"] == "noble_blade" {
                definition
                    .fields
                    .insert("effect_id".into(), json!("spell_break"));
            }
        }
        let mut unported = NativeSession::compile(envelope).unwrap();
        let request = unported
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let primary = match &request.choices {
            RequestChoices::Action(menu) => menu
                .options
                .iter()
                .find(|option| option.primary_effect)
                .unwrap()
                .kind
                .name(),
            _ => panic!("action prompt expected"),
        };
        let primary = unported.import_selection(&json!(primary)).unwrap();
        unported.submit(request.id, primary).unwrap();
        assert!(
            matches!(unported.advance(&ExecutionBudget::default()),Err(EngineError::Unsupported(message)) if message.contains("primary card effect"))
        );
    }

    #[test]
    fn primary_text_distinguishes_hidden_no_op_from_empty_identifier_fallback() {
        for hidden in [false, true] {
            let mut envelope = source("action-execution-Arien-MOVEMENT");
            for definition in &mut envelope.definitions {
                if definition.kind == crate::codec::InstanceKind::Card {
                    definition.fields.insert("effect_id".into(), json!(""));
                }
            }
            for instance in &mut envelope.instances {
                if instance.fields.contains_key("is_facedown") {
                    instance.fields.insert("is_facedown".into(), json!(hidden));
                }
            }
            let mut text = combat_template("card_text").unwrap();
            text["hero_id"] = json!("hero_arien");
            let mut done = combat_template("context").unwrap();
            done["key"] = json!("action_done");
            done["value"] = json!(true);
            envelope.state["execution_stack"] = json!([done, text]);
            let mut session = NativeSession::compile(envelope).unwrap();
            let result = session.advance(&ExecutionBudget::default());
            if hidden {
                assert!(result.unwrap().request.is_none());
                assert_eq!(
                    session.semantic_state().unwrap()["execution_context"]["action_done"],
                    true
                );
            } else {
                assert!(result.is_ok());
                assert_eq!(
                    session.semantic_state().unwrap()["execution_context"]["attack_is_ranged"],
                    false
                );
            }
        }
    }
    #[test]
    fn multi_selection_workers_and_restored_partial_choices_are_independent() {
        let mut root = NativeSession::compile(source("action-execution-multi-maximum")).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let before = root.semantic_state().unwrap();
        let mut left = root.clone();
        let mut right = root.clone();
        let a = left.import_selection(&json!("native_token_0")).unwrap();
        let b = right.import_selection(&json!("native_token_1")).unwrap();
        left.submit(request.id, a).unwrap();
        right.submit(request.id, b).unwrap();
        left.advance(&ExecutionBudget::default()).unwrap();
        right.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(
            left.semantic_state().unwrap()["execution_context"]["picked"],
            json!(["native_token_0"])
        );
        assert_eq!(
            right.semantic_state().unwrap()["execution_context"]["picked"],
            json!(["native_token_1"])
        );
        assert_eq!(root.semantic_state().unwrap(), before);
        let mut restored = NativeSession::compile(left.export_envelope().unwrap()).unwrap();
        let request = restored
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let done = restored.import_selection(&json!("DONE")).unwrap();
        restored.submit(request.id, done).unwrap();
        restored.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(
            restored.semantic_state().unwrap()["execution_context"]["picked"],
            json!(["native_token_0"])
        );
        assert_eq!(
            restored.semantic_state().unwrap()["execution_context"]["multi_done"],
            true
        );
    }

    #[test]
    fn clear_restores_between_each_generated_token_removal_without_losing_events() {
        let mut ready = NativeSession::compile(source("action-execution-tokens")).unwrap();
        let mut request = ready
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let clear = ready.import_selection(&json!("CLEAR")).unwrap();
        ready.submit(request.id, clear).unwrap();
        request = ready
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        for i in 0..3 {
            let token = ready
                .import_selection(&json!(format!("native_token_{i}")))
                .unwrap();
            ready.submit(request.id, token).unwrap();
            if i < 2 {
                request = ready
                    .advance(&ExecutionBudget::default())
                    .unwrap()
                    .request
                    .unwrap();
            }
        }
        let mut baseline = ready.clone();
        let complete = baseline.advance(&ExecutionBudget::default()).unwrap();
        assert!(complete.request.is_none());
        assert_eq!(complete.events.len(), 3);
        let expected_events = complete
            .events
            .iter()
            .map(|e| baseline.event_json(e))
            .collect::<Vec<_>>();
        let expected_state = baseline.semantic_state().unwrap();
        for cut in 0..12 {
            let mut partial = ready.clone();
            let prefix = partial.advance_prefix(cut).unwrap();
            assert!(prefix.request.is_none());
            let mut events = prefix
                .events
                .iter()
                .map(|e| partial.event_json(e))
                .collect::<Vec<_>>();
            let mut restored = NativeSession::compile(partial.export_envelope().unwrap()).unwrap();
            let suffix = restored.advance(&ExecutionBudget::default()).unwrap();
            assert!(suffix.request.is_none());
            events.extend(suffix.events.iter().map(|e| restored.event_json(e)));
            assert_eq!(events, expected_events, "cut {cut}");
            assert_eq!(
                restored.semantic_state().unwrap(),
                expected_state,
                "cut {cut}"
            );
        }
    }

    #[test]
    fn turn_handoffs_restore_every_instruction_and_prompt() {
        fn run(mut session: NativeSession, restore: bool, movement: bool) -> (Value, Vec<Value>) {
            let mut events = Vec::new();
            for _ in 0..200 {
                let result = session.advance_prefix(1).unwrap();
                events.extend(result.events.iter().map(|e| session.event_json(e)));
                let answer = result.request.as_ref().map(|req| {
                    let wire = session.request_json(req);
                    let options = wire["options"].as_array().unwrap();
                    let find = |id: &str| options.iter().find(|o| o["id"] == id);
                    let option = if movement { find("MOVEMENT") } else { None }
                        .or_else(|| find("HOLD"))
                        .or_else(|| find("CONFIRM"))
                        .or_else(|| find("SKIP"))
                        .unwrap_or(&options[0]);
                    (
                        session.request_token(req.id),
                        option["metadata"]
                            .get("hex")
                            .unwrap_or(&option["id"])
                            .clone(),
                    )
                });
                if restore {
                    session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
                }
                if let Some((token, value)) = answer {
                    let request = if restore {
                        session.advance_prefix(1).unwrap().request.unwrap()
                    } else {
                        result.request.unwrap()
                    };
                    assert_eq!(session.request_token(request.id), token);
                    let selected = session.import_selection(&value).unwrap();
                    session.submit(request.id, selected).unwrap();
                }
                if session.stack.is_empty() {
                    return (session.semantic_state().unwrap(), events);
                }
            }
            panic!("turn did not terminate");
        }
        for name in [
            "turn-handoff-unique",
            "turn-handoff-cross-team-tie",
            "turn-handoff-movement",
            "turn-respawn-fallback",
            "turn-respawn-token",
            "turn-finalize-piece",
            "turn-handoff-abort",
            "turn-finalize-guess-own",
        ] {
            let root = NativeSession::compile(source(name)).unwrap();
            let expected = run(root.clone(), false, name.ends_with("movement"));
            assert_eq!(
                run(root, true, name.ends_with("movement")),
                expected,
                "{name}"
            );
        }
    }
    #[test]
    fn turn_workers_keep_cards_counters_and_position_snapshots_private() {
        let root = NativeSession::compile(source("turn-finalize-ordinary")).unwrap();
        let before = root.semantic_state().unwrap();
        let mut a = root.clone();
        let mut b = root.clone();
        let hero = a.symbols.lookup("hero_arien").unwrap();
        let to = a
            .world
            .tile_hexes()
            .iter()
            .copied()
            .find(|h| !a.world.obstacle(*h))
            .unwrap();
        a.world.move_entity(hero, to).unwrap();
        a.world.combat.turn = 2;
        a.advance(&ExecutionBudget::default()).unwrap();
        b.advance(&ExecutionBudget::default()).unwrap();
        let av = a.semantic_state().unwrap();
        let bv = b.semantic_state().unwrap();
        assert_eq!(av["phase"], "PLANNING");
        assert_eq!(av["turn"], 3);
        assert_eq!(bv["turn"], 2);
        assert_ne!(av["last_turn_positions"], bv["last_turn_positions"]);
        assert_eq!(av["last_turn_positions"], av["entity_locations"]);
        assert_eq!(root.semantic_state().unwrap(), before);
        assert!(Arc::ptr_eq(
            a.flow_program.as_ref().unwrap(),
            b.flow_program.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(
            a.turn_rules.as_ref().unwrap(),
            b.turn_rules.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(&a.world.definitions, &b.world.definitions));
    }
    #[test]
    fn resolved_cards_gate_stats_only_for_uncompiled_aura_hooks() {
        let envelope = source("turn-finalize-ordinary");
        let mut symbols = Symbols::default();
        let mut plain = NativeWorld::compile(&envelope, &mut symbols).unwrap();
        let hero = symbols.lookup("hero_arien").unwrap();
        plain.resolve_current_card(hero, Some(1)).unwrap();
        assert!(!plain.has_uncompiled_auras);
        assert_eq!(plain.computed_stat(hero, Stat::Initiative, 3).unwrap(), 3);
        let mut changed = envelope;
        changed
            .definitions
            .iter_mut()
            .find(|d| d.fields.get("effect_id") == Some(&json!("noble_blade")))
            .unwrap()
            .fields
            .insert("effect_id".into(), json!("witching_hour"));
        let mut symbols = Symbols::default();
        let mut guarded = NativeWorld::compile(&changed, &mut symbols).unwrap();
        let hero = symbols.lookup("hero_arien").unwrap();
        assert!(!guarded.has_uncompiled_auras);
        guarded.resolve_current_card(hero, Some(1)).unwrap();
        assert!(guarded.has_uncompiled_auras);
        assert!(
            guarded
                .computed_stat(hero, Stat::Initiative, 3)
                .unwrap_err()
                .contains("aura")
        );
    }
    #[test]
    fn active_movement_aura_freezes_permission_and_restores_every_instruction() {
        fn run(mut session: NativeSession, restore: bool) -> (Value, Vec<Value>) {
            let mut events = Vec::new();
            for _ in 0..40 {
                let result = session.advance_prefix(1).unwrap();
                events.extend(result.events.iter().map(|e| session.event_json(e)));
                let answer = result.request.as_ref().map(|r| {
                    let RequestChoices::Hexes(hexes) = &r.choices else {
                        panic!("hexes");
                    };
                    let chosen = hexes
                        .iter()
                        .find(|h| h.coordinates() == (2, 0, -2))
                        .unwrap_or(&hexes[0]);
                    serde_json::to_value(chosen).unwrap()
                });
                if restore {
                    session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
                }
                if let Some(value) = answer {
                    let req = if restore {
                        session.advance_prefix(1).unwrap().request.unwrap()
                    } else {
                        result.request.unwrap()
                    };
                    let value = session.import_selection(&value).unwrap();
                    session.submit(req.id, value).unwrap();
                    if restore {
                        session =
                            NativeSession::compile(session.export_envelope().unwrap()).unwrap();
                    }
                }
                if session.stack.is_empty() {
                    return (session.semantic_state().unwrap(), events);
                }
            }
            panic!("aura movement did not finish");
        }
        for name in [
            "aura-movement-ordinary",
            "aura-movement-straight",
            "aura-movement-full-distance",
            "aura-movement-computed-range",
        ] {
            let root = NativeSession::compile(source(name)).unwrap();
            let expected = run(root.clone(), false);
            assert_eq!(run(root, true), expected, "{name}");
            assert_eq!(
                expected.0["entity_locations"]["hero_xargatha"],
                json!({"q":2,"r":0,"s":-2})
            );
        }
        let mut root = NativeSession::compile(source("aura-movement-ordinary")).unwrap();
        root.advance_prefix(1).unwrap();
        let mut paused = root.export_envelope().unwrap();
        // The movement-start aura is frozen in both generated child instructions.
        assert!(
            paused.state["execution_stack"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["pass_through_obstacles"] == true)
        );
        for team in paused.state["teams"].as_object_mut().unwrap().values_mut() {
            for hero in team["heroes"].as_array_mut().unwrap() {
                if hero["id"] == "hero_xargatha" {
                    hero["level"] = json!(7);
                }
            }
        }
        let root = NativeSession::compile(paused).unwrap();
        let state = run(root, true).0;
        assert_eq!(
            state["entity_locations"]["hero_xargatha"],
            json!({"q":2,"r":0,"s":-2})
        );
    }

    #[test]
    fn aura_movement_workers_share_programs_but_not_positions_or_adjacency_counts() {
        let mut root = NativeSession::compile(source("aura-movement-computed-range")).unwrap();
        let hero = root.symbols.lookup("hero_xargatha").unwrap();
        let enemy = root.symbols.lookup("hero_brogan").unwrap();
        assert_eq!(
            root.world.computed_stat(hero, Stat::Movement, 1).unwrap(),
            2
        );
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let before = root.semantic_state().unwrap();
        let mut a = root.clone();
        let mut b = root.clone();
        a.world.remove_entity(enemy).unwrap();
        assert_eq!(a.world.computed_stat(hero, Stat::Movement, 1).unwrap(), 1);
        assert_eq!(b.world.computed_stat(hero, Stat::Movement, 1).unwrap(), 2);
        let to = crate::geometry::Hex::new(2, 0, -2).unwrap();
        a.submit(request.id, native_hex(to, a.coordinates)).unwrap();
        a.advance(&ExecutionBudget::default()).unwrap();
        // Removing an adjacent enemy after the action starts does not shrink its frozen range.
        assert_eq!(a.world.position(hero), Some(to));
        let stay = crate::geometry::Hex::new(0, 0, 0).unwrap();
        b.submit(request.id, native_hex(stay, b.coordinates))
            .unwrap();
        assert!(
            b.advance(&ExecutionBudget::default())
                .unwrap()
                .request
                .is_none()
        );
        assert_ne!(a.world.position(hero), b.world.position(hero));
        assert_eq!(root.semantic_state().unwrap(), before);
        assert!(Arc::ptr_eq(&a.steps, &b.steps));
        assert!(Arc::ptr_eq(&a.world.definitions, &b.world.definitions));
    }

    #[test]
    fn conditional_stats_follow_live_private_context_and_explicit_cards() {
        let root =
            NativeSession::compile(source("aura-conditional-changing-context-trace")).unwrap();
        let hero = root.symbols.lookup("hero_sabina").unwrap();
        let key = root.symbols.lookup("performing_card_id").unwrap();
        let basic = root.symbols.lookup("point_blank_shot").unwrap();
        let current = root.world.hero(hero).unwrap().current.unwrap();
        let other =
            root.world.definitions.cards[root.world.card(current).unwrap().definition].printed_id;
        let basic_card = root.world.card_for_hero_printed(hero, basic).unwrap();
        assert!(
            root.world
                .computed_stat_for_card(hero, Stat::Attack, 1, Some(crate::cards::CardId(u32::MAX)))
                .is_err()
        );
        let before = root.semantic_state().unwrap();
        let mut a = root.clone();
        let mut b = root.clone();
        a.context.set(key, Datum::Text(basic));
        b.context.set(key, Datum::Text(other));
        assert_eq!(
            a.world
                .computed_stat_in_context(hero, Stat::Attack, 1, &a.context)
                .unwrap(),
            3
        );
        assert_eq!(
            b.world
                .computed_stat_in_context(hero, Stat::Attack, 1, &b.context)
                .unwrap(),
            1
        );
        assert_eq!(a.world.computed_stat(hero, Stat::Attack, 1).unwrap(), 1);
        assert_eq!(
            a.world
                .computed_stat_for_card(hero, Stat::Attack, 1, Some(current))
                .unwrap(),
            1
        );
        assert_eq!(
            b.world.card_stats(hero, basic_card).unwrap().primary_value,
            4
        );
        a.context.set(key, Datum::Text(other));
        assert_eq!(
            a.world
                .computed_stat_in_context(hero, Stat::Range, 1, &a.context)
                .unwrap(),
            1
        );
        a.context.set(key, Datum::Text(basic));
        assert_eq!(
            a.world
                .computed_stat_in_context(hero, Stat::Range, 1, &a.context)
                .unwrap(),
            3
        );
        assert_eq!(root.semantic_state().unwrap(), before);
        assert!(Arc::ptr_eq(&a.world.definitions, &b.world.definitions));
        assert!(Arc::ptr_eq(&a.steps, &b.steps));
    }

    fn run_instruction_restores(mut session: NativeSession, restore: bool) -> (Value, Vec<Value>) {
        let mut events = Vec::new();
        for _ in 0..150 {
            let result = session.advance_prefix(1).unwrap();
            events.extend(result.events.iter().map(|e| session.event_json(e)));
            let answer = result.request.as_ref().map(|req| {
                let wire = session.request_json(req);
                let options = wire["options"].as_array().unwrap();
                let option = options
                    .iter()
                    .find(|o| o["id"] == "ATTACK")
                    .unwrap_or(&options[0]);
                option["metadata"]
                    .get("hex")
                    .unwrap_or(&option["id"])
                    .clone()
            });
            if restore {
                session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
            }
            if let Some(value) = answer {
                let req = if restore {
                    session.advance_prefix(1).unwrap().request.unwrap()
                } else {
                    result.request.unwrap()
                };
                let value = session.import_selection(&value).unwrap();
                session.submit(req.id, value).unwrap();
                if restore {
                    session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
                }
            }
            if session.stack.is_empty() {
                return (session.semantic_state().unwrap(), events);
            }
        }
        panic!("conditional continuation did not finish");
    }

    #[test]
    fn nested_card_stats_adjacency_and_discard_or_defeat_restore_every_instruction() {
        for name in [
            "aura-conditional-changing-context-trace",
            "aura-conditional-primary-basic-trace",
            "aura-conditional-primary-ctx-nonbasic-trace",
            "discard-or-defeat-hand",
            "discard-or-defeat-one-card",
            "discard-or-defeat-empty",
            "discard-or-defeat-offboard",
            "discard-or-defeat-missing",
            "discard-or-defeat-falsy",
            "discard-or-defeat-unknown",
            "discard-or-defeat-minion",
            "discard-or-defeat-killer-literal",
            "discard-or-defeat-killer-key",
            "discard-or-defeat-killer-fallback",
            "discard-or-defeat-no-actor",
            "discard-or-defeat-multi-owner",
            "discard-or-defeat-multi-piece",
            "discard-or-defeat-multi-offboard",
            "discard-or-defeat-multi-empty",
            "discard-or-defeat-immunity-source",
        ] {
            let root = NativeSession::compile(source(name)).unwrap();
            assert_eq!(
                run_instruction_restores(root.clone(), true),
                run_instruction_restores(root, false),
                "{name}"
            );
        }
    }

    #[test]
    fn passive_continuations_restore_at_every_instruction() {
        for name in [
            "passive-enemy",
            "passive-empty",
            "passive-friendly",
            "passive-minion",
            "passive-missing",
            "passive-falsy",
            "passive-offboard",
            "passive-unknown",
            "passive-piece",
            "passive-owner",
            "passive-level-seven",
            "passive-facedown",
            "passive-regular",
            "passive-unresolved",
            "passive-alias",
            "passive-spent",
            "passive-override",
            "passive-wrong-trigger",
            "passive-offer-yes",
            "passive-offer-no",
            "passive-offer-skip",
            "passive-offer-low-level",
            "passive-offer-missing-card",
            "passive-offer-facedown",
            "passive-offer-no-owner",
            "passive-offer-wrong-trigger",
            "passive-mark",
            "passive-mark-facedown",
            "passive-mark-missing",
            "passive-mark-override-actor",
            "passive-push",
            "passive-push-empty",
            "passive-push-twice",
            "passive-push-blocked",
            "passive-primary",
        ] {
            let root = NativeSession::compile(source(name)).unwrap();
            assert_eq!(
                run_instruction_restores(root.clone(), true),
                run_instruction_restores(root, false),
                "{name}"
            );
        }
    }
    #[test]
    fn passive_workers_keep_usage_and_context_private() {
        let root = NativeSession::compile(source("passive-enemy")).unwrap();
        let before = root.semantic_state().unwrap();
        let mut a = root.clone();
        let mut b = root.clone();
        assert!(Arc::ptr_eq(&a.passive_rules, &b.passive_rules));
        assert!(Arc::ptr_eq(&a.steps, &b.steps));
        let key = b.symbols.lookup("push_victim_id").unwrap();
        b.context.remove(key);
        assert!(
            b.advance(&ExecutionBudget::default())
                .unwrap()
                .request
                .is_none()
        );
        let request = a
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let wire = a.request_json(&request);
        let value = a.import_selection(&wire["options"][0]["id"]).unwrap();
        a.submit(request.id, value).unwrap();
        assert!(
            a.advance(&ExecutionBudget::default())
                .unwrap()
                .request
                .is_none()
        );
        let hero = a.symbols.lookup("hero_sabina").unwrap();
        let card = a.world.hero(hero).unwrap().ultimate.unwrap();
        assert_eq!(a.world.card(card).unwrap().passive_uses, 1);
        assert_eq!(b.world.card(card).unwrap().passive_uses, 0);
        assert_eq!(root.semantic_state().unwrap(), before);
    }

    #[test]
    fn optional_passive_rechecks_live_card_before_acceptance() {
        let mut root = NativeSession::compile(source("passive-offer-yes")).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let before = root.semantic_state().unwrap();
        let hero = root.symbols.lookup("hero_sabina").unwrap();
        let card = root.world.hero(hero).unwrap().ultimate.unwrap();
        for remove in [false, true] {
            let mut worker = root.clone();
            if remove {
                let index = worker.world.hero_index(hero).unwrap();
                worker.world.heroes[index].ultimate = None;
            } else {
                worker.world.card_mut(card).unwrap().facedown = true;
            }
            worker.submit(request.id, Datum::Text(worker.yes)).unwrap();
            assert!(
                worker
                    .advance(&ExecutionBudget::default())
                    .unwrap()
                    .request
                    .is_none()
            );
            assert_eq!(worker.world.card(card).unwrap().passive_uses, 0);
        }
        assert_eq!(root.semantic_state().unwrap(), before);
    }
    #[test]
    fn unported_passive_providers_remain_explicit_and_do_not_mutate_the_root() {
        let root = NativeSession::compile(source("passive-enemy")).unwrap();
        let before = root.semantic_state().unwrap();
        let hero = root.symbols.lookup("hero_sabina").unwrap();
        let card = root.world.hero(hero).unwrap().ultimate.unwrap();
        let mut worker = root.clone();
        let definition = worker.world.card(card).unwrap().definition;
        let effect = worker.symbols.lookup("battle_fury").unwrap();
        Arc::make_mut(&mut worker.world.definitions).cards[definition].effect = effect;
        let trigger = worker.symbols.lookup("after_card_discard").unwrap();
        assert!(
            worker
                .passive_rules
                .scan(&worker.world, Some(hero), None, trigger)
                .unwrap_err()
                .contains("eligible passive")
        );
        assert_eq!(worker.world.card(card).unwrap().passive_uses, 0);
        assert_eq!(root.semantic_state().unwrap(), before);
    }

    #[test]
    fn tactical_primaries_restore_at_every_instruction() {
        for name in [
            "primary-back_to_back-ordinary",
            "primary-back_to_back-items",
            "primary-listen_up-ordinary",
            "primary-listen_up-items",
            "primary-roger_roger-ordinary",
            "primary-roger_roger-items",
            "primary-ready_and_waiting-ordinary",
            "primary-ready_and_waiting-items",
            "primary-troop_movement-ordinary",
            "primary-troop_movement-items",
            "primary-marching_orders-ordinary",
            "primary-marching_orders-items",
            "primary-path_to_victory-ordinary",
            "primary-path_to_victory-items",
            "primary-close_support-ordinary",
            "primary-close_support-items",
            "primary-covering_fire-ordinary",
            "primary-covering_fire-items",
            "primary-shootout-ordinary",
            "primary-shootout-items",
            "primary-bullet_hell-ordinary",
            "primary-bullet_hell-items",
        ] {
            let root = NativeSession::compile(source(name)).unwrap();
            assert_eq!(
                run_instruction_restores(root.clone(), true),
                run_instruction_restores(root, false),
                "{name}"
            );
        }
    }
    #[test]
    fn repeat_continuations_restore_at_every_instruction() {
        for name in [
            "repeat-once",
            "repeat-twice",
            "repeat-decline",
            "repeat-skip",
            "repeat-once-then-no",
            "repeat-nested",
            "repeat-already-one",
            "repeat-negative-done",
            "repeat-zero-max",
            "repeat-negative-max",
            "repeat-done",
            "repeat-no-actor",
            "repeat-offboard",
            "repeat-unknown-actor",
            "repeat-piece",
            "repeat-lane-push",
            "repeat-lane-checked",
            "repeat-lane-empty",
        ] {
            let root = NativeSession::compile(source(name)).unwrap();
            assert_eq!(
                run_instruction_restores(root.clone(), true),
                run_instruction_restores(root, false),
                "{name}"
            );
        }
    }
    #[test]
    fn repeat_workers_keep_counts_and_nested_copies_private() {
        let mut root = NativeSession::compile(source("repeat-nested")).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let before = root.semantic_state().unwrap();
        let mut accepted = root.clone();
        let mut declined = root.clone();
        assert!(Arc::ptr_eq(&accepted.steps, &declined.steps));
        accepted
            .submit(request.id, Datum::Text(accepted.yes))
            .unwrap();
        let no = declined.import_selection(&json!("NO")).unwrap();
        declined.submit(request.id, no).unwrap();
        assert!(
            declined
                .advance(&ExecutionBudget::default())
                .unwrap()
                .request
                .is_none()
        );
        for _ in 0..10 {
            let next = accepted.advance(&ExecutionBudget::default()).unwrap();
            if let Some(request) = next.request {
                accepted
                    .submit(request.id, Datum::Text(accepted.yes))
                    .unwrap();
            }
            if accepted.stack.is_empty() {
                break;
            }
        }
        assert!(accepted.stack.is_empty());
        let key = root.symbols.lookup("repeat_count").unwrap();
        assert_eq!(accepted.context.get(key), Some(&Datum::Int(2)));
        assert_eq!(declined.context.get(key), Some(&Datum::Int(0)));
        assert_eq!(root.semantic_state().unwrap(), before);
    }
    #[test]
    fn repeat_validation_guards_unported_effects_and_aggregate_aliases() {
        let root = NativeSession::compile(source("repeat-validation-true")).unwrap();
        let before = root.semantic_state().unwrap();
        let mut worker = root.clone();
        let key = worker.symbols.lookup("previous").unwrap();
        worker.context.set(key, Datum::List(vec![Datum::Int(1)]));
        assert!(
            matches!(worker.advance(&ExecutionBudget::default()),Err(EngineError::Unsupported(message)) if message.contains("aggregate alias"))
        );
        assert_eq!(root.semantic_state().unwrap(), before);
        let mut worker = root.clone();
        worker.world.has_effects = true;
        assert!(!worker.world.can_repeat_action(None).unwrap());
        assert!(
            worker
                .world
                .can_repeat_action(worker.planning.actor)
                .unwrap_err()
                .contains("repeat-prevention")
        );
    }

    #[test]
    fn conditional_card_queries_restore_at_every_instruction() {
        for name in [
            "conditional-primary-quickdraw-attack-current",
            "conditional-primary-quickdraw-attack-resolved",
            "conditional-primary-quickdraw-piece-attack",
            "conditional-primary-gunslinger-attack-current",
            "conditional-primary-gunslinger-attack-resolved",
            "conditional-primary-gunslinger-piece-attack",
            "conditional-primary-dead_shot-attack-current",
            "conditional-primary-dead_shot-attack-resolved",
            "conditional-primary-dead_shot-piece-attack",
            "conditional-primary-shield_bash-attack-current",
            "conditional-primary-shield_bash-attack-resolved",
            "conditional-primary-shield_bash-piece-attack",
            "conditional-primary-counterattack-attack-current",
            "conditional-primary-counterattack-attack-resolved",
            "conditional-primary-counterattack-piece-attack",
        ] {
            let root = NativeSession::compile(source(name)).unwrap();
            assert_eq!(
                run_instruction_restores(root.clone(), true),
                run_instruction_restores(root, false),
                "{name}"
            );
        }
    }
    #[test]
    fn effect_creation_and_frozen_payloads_restore_at_each_instruction() {
        for name in [
            "effect-step-prefix",
            "effect-step-freeze-colors",
            "effect-step-duplicate",
            "effect-step-duplicate-dormant",
            "effect-step-token-piece",
            "effect-step-nested-color",
            "effect-step-id-collision",
        ] {
            let root = NativeSession::compile(source(name)).unwrap();
            assert_eq!(
                run_instruction_restores(root.clone(), true),
                run_instruction_restores(root, false),
                "{name}"
            );
        }
    }
    #[test]
    fn created_effects_do_not_enable_unported_gameplay_or_controller_requests() {
        let mut envelope = source("effect-step-plain");
        let mut root = NativeSession::compile(envelope.clone()).unwrap();
        root.advance(&ExecutionBudget::default()).unwrap();
        let before = root.semantic_state().unwrap();
        envelope = root.export_envelope().unwrap();
        envelope.state["execution_stack"] = json!([{"type":"select","target_type":"NUMBER","options":[1],"output_key":"answer","prompt":"Choose","step_id":"fixture/effect-unsafe-input"}]);
        assert!(
            matches!(NativeSession::compile(envelope),Err(EngineError::Unsupported(message)) if message.contains("active-effect"))
        );
        assert_eq!(root.semantic_state().unwrap(), before);
    }
    #[test]
    fn effect_step_workers_freeze_different_colors_without_mutating_templates() {
        let root = NativeSession::compile(source("effect-step-freeze-colors")).unwrap();
        let before = root.semantic_state().unwrap();
        let mut a = root.clone();
        let mut b = root.clone();
        let key = b.symbols.lookup("color").unwrap();
        let red = b.symbols.lookup("RED").unwrap();
        b.context.set(key, Datum::Text(red));
        a.advance_prefix(1).unwrap();
        b.advance_prefix(1).unwrap();
        assert!(Arc::ptr_eq(&a.steps, &b.steps));
        assert_ne!(
            a.world.effects.export(&a.symbols),
            b.world.effects.export(&b.symbols)
        );
        assert_eq!(root.semantic_state().unwrap(), before);
    }
    #[test]
    fn ordinary_retrieval_restores_at_each_instruction() {
        for name in [
            "retrieval-current",
            "retrieval-played",
            "retrieval-all",
            "retrieval-hero-key-piece",
            "retrieval-global-shadow",
            "retrieval-played-discard-alias",
            "discard-expiry-global-shadow",
            "discard-expiry-enraged",
            "retrieval-primary-steady_advance-ordinary",
            "retrieval-primary-unwavering_resolve-ordinary",
            "retrieval-primary-devoted_followers-ordinary",
            "retrieval-primary-fresh_converts-ordinary",
        ] {
            let root = NativeSession::compile(source(name)).unwrap();
            assert_eq!(
                run_instruction_restores(root.clone(), true),
                run_instruction_restores(root, false),
                "{name}"
            );
        }
    }
    #[test]
    fn ordinary_retrieval_workers_have_private_collections_and_expiry() {
        let root = NativeSession::compile(source("retrieval-all")).unwrap();
        let before = root.semantic_state().unwrap();
        let actor = root.planning.actor.unwrap();
        let cards = root.world.hero(actor).unwrap().discard.clone();
        assert_eq!(cards.len(), 2);
        let mut a = root.clone();
        let mut b = root.clone();
        let printed =
            b.world.definitions.cards[b.world.card(cards[0]).unwrap().definition].printed_id;
        let events = b.world.retrieve_card(actor, Some(printed), false).unwrap();
        assert_eq!(events.len(), 1);
        a.advance(&ExecutionBudget::default()).unwrap();
        assert!(a.world.hero(actor).unwrap().discard.is_empty());
        assert_eq!(b.world.hero(actor).unwrap().discard, vec![cards[1]]);
        assert!(!a.world.card(cards[1]).unwrap().active);
        assert!(b.world.card(cards[1]).unwrap().active);
        assert!(Arc::ptr_eq(&a.steps, &b.steps));
        assert!(Arc::ptr_eq(&a.world.definitions, &b.world.definitions));
        assert_eq!(root.semantic_state().unwrap(), before);
    }
    #[test]
    fn retrieval_rejects_unported_expiry_and_partial_failure_before_mutation() {
        let root = NativeSession::compile(source("retrieval-all")).unwrap();
        let actor = root.planning.actor.unwrap();
        let mut worker = root.clone();
        worker.world.has_effects = true;
        let before = worker.semantic_state().unwrap();
        assert!(matches!(
            worker.world.retrieve_card(actor, None, true),
            Err(crate::cards::CardError::Unsupported(_))
        ));
        assert_eq!(worker.semantic_state().unwrap(), before);
        let mut worker = root.clone();
        let card = worker.world.hero(actor).unwrap().discard[0];
        worker
            .world
            .heroes
            .iter_mut()
            .find(|h| h.id == actor)
            .unwrap()
            .discard
            .push(card);
        let before = worker.semantic_state().unwrap();
        assert!(matches!(
            worker.world.retrieve_card(actor, None, true),
            Err(crate::cards::CardError::Unsupported(_))
        ));
        assert_eq!(worker.semantic_state().unwrap(), before);
    }

    #[test]
    fn queries_restore_order_and_scalar_results_at_instruction_boundaries() {
        for name in [
            "query-collect-order",
            "query-played-current",
            "query-immunity-on",
            "query-hex-range",
            "query-piece-owner",
            "query-collect-immune-token",
        ] {
            let root = NativeSession::compile(source(name)).unwrap();
            assert_eq!(
                run_instruction_restores(root.clone(), true),
                run_instruction_restores(root, false),
                "{name}"
            );
        }
    }
    #[test]
    fn query_workers_read_private_live_cards_slots_and_occupancy() {
        for name in ["query-played-current", "query-played-resolved"] {
            let root = NativeSession::compile(source(name)).unwrap();
            let before = root.semantic_state().unwrap();
            let mut a = root.clone();
            let mut b = root.clone();
            assert!(Arc::ptr_eq(&a.steps, &b.steps));
            assert!(Arc::ptr_eq(&a.world.definitions, &b.world.definitions));
            if name.ends_with("current") {
                let hero = b.symbols.lookup("hero_brogan").unwrap();
                let card = b.world.hero(hero).unwrap().current.unwrap();
                b.world.card_mut(card).unwrap().facedown = true;
            } else {
                b.world.combat.turn = 2;
            }
            a.advance(&ExecutionBudget::default()).unwrap();
            b.advance(&ExecutionBudget::default()).unwrap();
            let output = root.symbols.lookup("result").unwrap();
            assert_eq!(a.context.get(output), Some(&Datum::Int(1)));
            assert_eq!(b.context.get(output), Some(&Datum::Int(0)));
            assert_eq!(root.semantic_state().unwrap(), before);
        }
        let root = NativeSession::compile(source("query-collect")).unwrap();
        let before = root.semantic_state().unwrap();
        let mut a = root.clone();
        let mut b = root.clone();
        let hero = b.symbols.lookup("hero_arien").unwrap();
        b.world.remove_entity(hero).unwrap();
        a.advance(&ExecutionBudget::default()).unwrap();
        b.advance(&ExecutionBudget::default()).unwrap();
        let output = root.symbols.lookup("result").unwrap();
        let (Some(Datum::List(all)), Some(Datum::List(less))) =
            (a.context.get(output), b.context.get(output))
        else {
            panic!("expected collected IDs")
        };
        assert_eq!(all.len(), less.len() + 1);
        assert_eq!(root.semantic_state().unwrap(), before);
    }
    #[test]
    fn query_boundaries_reject_invalid_flags_filters_and_nonfinite_comparisons() {
        for field in ["skip_self_filter", "skip_immunity_filter"] {
            let mut invalid = source("query-all");
            invalid.state["execution_stack"][0][field] = json!([]);
            assert!(matches!(
                NativeSession::compile(invalid),
                Err(EngineError::Unsupported(_))
            ));
        }
        for (field, value) in [
            ("action_type", "UNKNOWN_ACTION"),
            ("card_color", "UNKNOWN_COLOR"),
        ] {
            let mut invalid = source("query-played-current");
            invalid.state["execution_stack"][0]["filters"][0][field] = json!(value);
            assert!(matches!(
                NativeSession::compile(invalid),
                Err(EngineError::Unsupported(_))
            ));
        }
        assert!(
            crate::queries::Comparison::Ge
                .evaluate(Some(&Datum::Float(f64::NAN)), 0)
                .unwrap_err()
                .contains("nonfinite")
        );
    }

    #[test]
    fn conditional_stat_hook_exposes_one_eligible_push_passive() {
        let root =
            NativeSession::compile(source("aura-conditional-changing-context-trace")).unwrap();
        let hero = root.symbols.lookup("hero_sabina").unwrap();
        let trigger = root.symbols.lookup("after_push").unwrap();
        let before = root.semantic_state().unwrap();
        assert!(!root.world.has_uncompiled_auras);
        assert!(
            root.passive_rules
                .scan(&root.world, Some(hero), None, trigger)
                .unwrap()
                .len()
                == 1
        );
        assert_eq!(root.semantic_state().unwrap(), before);
    }

    #[test]
    fn ordinary_round_cleanup_advances_into_planning() {
        let mut root = NativeSession::compile(source("turn-advance-round-end")).unwrap();
        let result = root.advance(&ExecutionBudget::default()).unwrap();
        assert!(result.request.is_none());
        let state = root.semantic_state().unwrap();
        assert_eq!(state["round"], 2);
        assert_eq!(state["turn"], 1);
        assert_eq!(state["phase"], "PLANNING");
    }

    fn upgrade_answer(session: &NativeSession, request: &NativeRequest, side: usize) -> Datum {
        let RequestChoices::Upgrades(players) = &request.choices else {
            panic!("upgrade prompt expected");
        };
        let player = players
            .iter()
            .find(|p| p.remaining > 0 && !p.options.is_empty())
            .unwrap();
        let id = player.options[0].pair[side];
        let printed =
            session.world.definitions.cards[session.world.card(id).unwrap().definition].printed_id;
        Datum::Object(vec![
            (
                session.symbols.lookup("hero_id").unwrap(),
                Datum::Text(player.hero),
            ),
            (
                session.symbols.lookup("card_id").unwrap(),
                Datum::Text(printed),
            ),
        ])
    }

    #[test]
    fn round_cleanup_upgrades_and_ultimate_unlocks_restore_at_every_instruction() {
        fn run(mut session: NativeSession, restore: bool) -> (Value, Vec<Value>) {
            let mut events = Vec::new();
            for _ in 0..100 {
                let result = session.advance_prefix(1).unwrap();
                events.extend(result.events.iter().map(|e| session.event_json(e)));
                let answer = result
                    .request
                    .as_ref()
                    .map(|r| upgrade_answer(&session, r, 0).to_json(&session.symbols));
                if restore {
                    session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
                }
                if let Some(value) = answer {
                    let request = if restore {
                        session.advance_prefix(1).unwrap().request.unwrap()
                    } else {
                        result.request.unwrap()
                    };
                    // Symbol IDs are private to each compiled session.
                    let value = session.import_selection(&value).unwrap();
                    session.submit(request.id, value).unwrap();
                    if restore {
                        session =
                            NativeSession::compile(session.export_envelope().unwrap()).unwrap();
                    }
                }
                if session.stack.is_empty() {
                    return (session.semantic_state().unwrap(), events);
                }
            }
            panic!("round did not finish");
        }
        for name in [
            "round-cleanup-played-discard-current",
            "round-cleanup-markers",
            "round-token-duplicate-pool",
            "round-gold-28-batch-True",
            "round-cleanup-sparse-items",
            "round-ultimate-Ursafar-unlock-six",
            "round-ultimate-Takahide-unlock-six",
        ] {
            let root = NativeSession::compile(source(name)).unwrap();
            assert_eq!(run(root.clone(), true), run(root, false), "{name}");
        }
    }

    #[test]
    fn invalid_upgrade_batch_is_atomic_and_can_be_retried() {
        let mut root = NativeSession::compile(source("round-gold-3-batch-False")).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let RequestChoices::Upgrades(players) = &request.choices else {
            panic!("upgrade");
        };
        let p = &players[0];
        assert_eq!(p.remaining, 2);
        let printed =
            |id| root.world.definitions.cards[root.world.card(id).unwrap().definition].printed_id;
        let make = |cards: Vec<Symbol>| {
            Datum::Object(vec![
                (root.symbols.lookup("hero_id").unwrap(), Datum::Text(p.hero)),
                (
                    root.symbols.lookup("card_ids").unwrap(),
                    Datum::List(cards.into_iter().map(Datum::Text).collect()),
                ),
            ])
        };
        let invalid = make(p.options[0].pair.iter().copied().map(printed).collect());
        let valid = make(vec![
            printed(p.options[0].pair[0]),
            printed(p.options[1].pair[0]),
        ]);
        let before = root.semantic_state().unwrap();
        let wire = root.request_json(&request);
        assert!(matches!(
            root.submit(request.id, invalid),
            Err(EngineError::InvalidInput(_))
        ));
        assert_eq!(root.semantic_state().unwrap(), before);
        assert_eq!(root.request_json(root.last_request.as_ref().unwrap()), wire);
        root.submit(request.id, valid).unwrap();
        assert!(
            root.advance(&ExecutionBudget::default())
                .unwrap()
                .request
                .is_none()
        );
        let state = root.semantic_state().unwrap();
        assert_eq!(state["round"], 2);
        assert_eq!(state["phase"], "PLANNING");
        assert_eq!(state["pending_upgrades"], json!({}));
        assert_eq!(state["upgrade_reveal_snapshot"], json!({}));
    }

    #[test]
    fn upgrade_workers_share_rules_but_keep_cards_items_and_reveal_state_private() {
        let mut root = NativeSession::compile(source("round-gold-3-batch-False")).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let before = root.semantic_state().unwrap();
        let mut left = root.clone();
        let mut right = root.clone();
        let a = upgrade_answer(&left, &request, 0);
        let b = upgrade_answer(&right, &request, 1);
        left.submit(request.id, a).unwrap();
        right.submit(request.id, b).unwrap();
        left.advance(&ExecutionBudget::default()).unwrap();
        right.advance(&ExecutionBudget::default()).unwrap();
        assert_ne!(left.world.heroes[0].hand, right.world.heroes[0].hand);
        assert_ne!(left.world.heroes[0].items, right.world.heroes[0].items);
        assert_eq!(root.semantic_state().unwrap(), before);
        assert!(Arc::ptr_eq(
            left.round_rules.as_ref().unwrap(),
            right.round_rules.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(
            left.flow_program.as_ref().unwrap(),
            right.flow_program.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(
            &left.world.definitions,
            &right.world.definitions
        ));
    }

    #[test]
    fn planning_command_branches_restore_without_sharing_card_commits() {
        let mut root = NativeSession::compile(source("game-double")).unwrap();
        let hero = root.symbols.lookup("hero_emmitt").unwrap();
        let initial_hand = root.world.hero(hero).unwrap().hand.len();
        let card = root.world.hero(hero).unwrap().hand[0];
        root.commit_card(hero, card).unwrap();
        let before = root.semantic_state().unwrap();
        let mut left = root.clone();
        let mut right = NativeSession::compile(root.export_envelope().unwrap()).unwrap();
        let right_hero = right.symbols.lookup("hero_emmitt").unwrap();
        let second = left.world.hero(hero).unwrap().hand[0];
        left.commit_card(hero, second).unwrap();
        right.finish_planning(right_hero).unwrap();
        assert_eq!(root.semantic_state().unwrap(), before);
        assert_ne!(
            left.semantic_state().unwrap()["pending_second_cards"],
            right.semantic_state().unwrap()["pending_second_cards"]
        );
        left = NativeSession::compile(left.export_envelope().unwrap()).unwrap();
        right = NativeSession::compile(right.export_envelope().unwrap()).unwrap();
        let left_hero = left.symbols.lookup("hero_emmitt").unwrap();
        let right_hero = right.symbols.lookup("hero_emmitt").unwrap();
        left.uncommit_card(left_hero).unwrap();
        right.uncommit_card(right_hero).unwrap();
        // Undo removes the second commit before the first, matching the phase API.
        assert_eq!(
            left.world.hero(left_hero).unwrap().hand.len(),
            initial_hand - 1
        );
        assert_eq!(
            right.world.hero(right_hero).unwrap().hand.len(),
            initial_hand
        );
        left.uncommit_card(left_hero).unwrap();
        assert_eq!(left.world.hero(left_hero).unwrap().hand.len(), initial_hand);
        for session in [&left, &right] {
            let h = session
                .world
                .hero(session.symbols.lookup("hero_emmitt").unwrap())
                .unwrap();
            assert!(h.hand.iter().all(|id| h.deck.contains(id)));
        }
    }

    #[test]
    fn battle_restores_every_instruction_and_team_prompt() {
        fn run(mut session: NativeSession, restore: bool) -> (Value, Vec<Value>) {
            let mut events = Vec::new();
            for _ in 0..150 {
                let result = session.advance_prefix(1).unwrap();
                events.extend(result.events.iter().map(|e| session.event_json(e)));
                let answer = result.request.as_ref().map(|req| {
                    let wire = session.request_json(req);
                    let option = &wire["options"][0];
                    (
                        session.request_token(req.id),
                        option["metadata"]
                            .get("hex")
                            .unwrap_or(&option["id"])
                            .clone(),
                    )
                });
                if restore {
                    session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
                }
                if let Some((token, value)) = answer {
                    let request = if restore {
                        session.advance_prefix(1).unwrap().request.unwrap()
                    } else {
                        result.request.unwrap()
                    };
                    assert_eq!(session.request_token(request.id), token);
                    let selected = session.import_selection(&value).unwrap();
                    session.submit(request.id, selected).unwrap();
                }
                if session.stack.is_empty() {
                    return (session.semantic_state().unwrap(), events);
                }
            }
            panic!("battle did not terminate");
        }
        for name in [
            "battle-spawn-blocked-two",
            "battle-displace-same-origin",
            "battle-casualties-force-all",
            "battle-casualties-choice",
            "battle-multi-both-thrones",
            "battle-multi-last-red",
            "battle-red-push",
            "battle-minions-army",
            "battle-minions-all-lanes",
        ] {
            let root = NativeSession::compile(source(name)).unwrap();
            let expected = run(root.clone(), false);
            let actual = run(root, true);
            assert_eq!(actual, expected, "{name}");
        }
    }
    #[test]
    fn battle_workers_keep_waves_zones_and_victory_private() {
        let root = NativeSession::compile(source("battle-red-push")).unwrap();
        let before = root.semantic_state().unwrap();
        let mut final_wave = root.clone();
        let mut ordinary = root.clone();
        final_wave.world.waves[0].1 = 1;
        final_wave.advance(&ExecutionBudget::default()).unwrap();
        ordinary.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(final_wave.planning.phase, Phase::GameOver);
        assert_eq!(ordinary.planning.phase, Phase::Resolution);
        assert_eq!(final_wave.world.waves[0].1, 0);
        assert_eq!(ordinary.world.waves[0].1, 4);
        assert_ne!(final_wave.world.battle_zones, ordinary.world.battle_zones);
        assert_eq!(root.semantic_state().unwrap(), before);
        assert!(Arc::ptr_eq(
            root.battle_rules.as_ref().unwrap(),
            ordinary.battle_rules.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(
            root.battle_program.as_ref().unwrap(),
            final_wave.battle_program.as_ref().unwrap()
        ));
        assert!(Arc::ptr_eq(
            &ordinary.world.definitions,
            &final_wave.world.definitions
        ));
    }
    #[test]
    fn simultaneous_pushes_freeze_targets_and_flip_waves_once() {
        let mut root = NativeSession::compile(source("battle-multi-same-direction")).unwrap();
        root.advance_prefix(1).unwrap();
        let state = root.semantic_state().unwrap();
        let pushes = state["execution_stack"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["type"] == "lane_push")
            .collect::<Vec<_>>();
        assert_eq!(pushes.len(), 2);
        for push in pushes {
            assert_eq!(push["skip_wave_counter"], true);
            assert!(push["target_zone_id"].as_str().unwrap().ends_with("-3"));
        }
        assert!(root.world.waves.iter().all(|(_, n)| *n == 4));
        let mut restored = NativeSession::compile(root.export_envelope().unwrap()).unwrap();
        root.advance(&ExecutionBudget::default()).unwrap();
        restored.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(
            root.semantic_state().unwrap(),
            restored.semantic_state().unwrap()
        );
        assert!(root.world.waves.iter().all(|(_, n)| *n == 4));
    }

    #[test]
    fn minion_return_restores_every_generated_boundary() {
        for name in [
            "return-prefix",
            "return-prefix-fallback",
            "return-two-teams",
            "return-processed-first",
            "return-blocked-then-legal",
        ] {
            let mut root = NativeSession::compile(source(name)).unwrap();
            let mut expected = root.clone();
            let mut expected_events = Vec::new();
            loop {
                let result = expected.advance_prefix(1).unwrap();
                expected_events.extend(result.events.iter().map(|e| expected.event_json(e)));
                if let Some(req) = result.request {
                    let RequestChoices::Hexes(hexes) = req.choices else {
                        panic!("return destination");
                    };
                    expected
                        .submit(req.id, native_hex(hexes[0], expected.coordinates))
                        .unwrap();
                }
                if expected.stack.is_empty() {
                    break;
                }
            }
            let mut actual_events = Vec::new();
            for _ in 0..30 {
                let result = root.advance_prefix(1).unwrap();
                actual_events.extend(result.events.iter().map(|e| root.event_json(e)));
                root = NativeSession::compile(root.export_envelope().unwrap()).unwrap();
                if let Some(req) = result.request {
                    // Regenerate the imported request before accepting its saved token.
                    let resumed = root
                        .advance(&ExecutionBudget::default())
                        .unwrap()
                        .request
                        .unwrap();
                    let RequestChoices::Hexes(hexes) = resumed.choices else {
                        panic!("return destination");
                    };
                    assert_eq!(root.request_token(resumed.id), root.request_token(req.id));
                    root.submit(resumed.id, native_hex(hexes[0], root.coordinates))
                        .unwrap();
                }
                if root.stack.is_empty() {
                    break;
                }
            }
            assert!(root.stack.is_empty(), "{name}");
            assert_eq!(actual_events, expected_events, "{name}");
            assert_eq!(
                root.semantic_state().unwrap(),
                expected.semantic_state().unwrap(),
                "{name}"
            );
        }
    }

    #[test]
    fn minion_return_choices_are_private_to_workers() {
        let mut root = NativeSession::compile(source("return-choice")).unwrap();
        let req = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let RequestChoices::Hexes(ref hexes) = req.choices else {
            panic!("return destination");
        };
        assert!(hexes.len() > 1);
        let before = root.semantic_state().unwrap();
        let mut a = root.clone();
        let mut b = root.clone();
        a.submit(req.id, native_hex(hexes[0], a.coordinates))
            .unwrap();
        b.submit(req.id, native_hex(*hexes.last().unwrap(), b.coordinates))
            .unwrap();
        a.advance(&ExecutionBudget::default()).unwrap();
        b.advance(&ExecutionBudget::default()).unwrap();
        assert_ne!(
            a.semantic_state().unwrap()["entity_locations"],
            b.semantic_state().unwrap()["entity_locations"]
        );
        assert_eq!(root.semantic_state().unwrap(), before);
        assert!(Arc::ptr_eq(&a.steps, &b.steps));
        assert!(Arc::ptr_eq(&a.return_rules, &b.return_rules));
        assert!(Arc::ptr_eq(&a.world.definitions, &b.world.definitions));
    }

    #[test]
    fn minion_return_rejects_uncompiled_aura_restrictions_before_relocation() {
        let mut root = NativeSession::compile(source("return-walk")).unwrap();
        root.world.has_uncompiled_auras = true;
        let locations = root.world.locations.clone();
        assert!(matches!(
            root.advance(&ExecutionBudget::default()),
            Err(EngineError::Unsupported(_))
        ));
        assert_eq!(root.world.locations, locations);
    }

    #[test]
    fn primary_workers_keep_victim_choices_and_compiled_recipes_independent() {
        let mut root = NativeSession::compile(source("primary-noble_blade-ordinary")).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let choices = match &request.choices {
            RequestChoices::Units { values, .. } => {
                values.iter().map(|(id, _)| *id).collect::<Vec<_>>()
            }
            _ => panic!("unit choices expected"),
        };
        assert!(choices.len() >= 2);
        let before = root.semantic_state().unwrap();
        let mut left = root.clone();
        let mut right = root.clone();
        left.submit(request.id, Datum::Text(choices[0])).unwrap();
        right.submit(request.id, Datum::Text(choices[1])).unwrap();
        left.advance(&ExecutionBudget::default()).unwrap();
        right.advance(&ExecutionBudget::default()).unwrap();
        assert_ne!(
            left.semantic_state().unwrap()["execution_context"]["victim_id"],
            right.semantic_state().unwrap()["execution_context"]["victim_id"]
        );
        assert_eq!(root.semantic_state().unwrap(), before);
        assert!(Arc::ptr_eq(&left.steps, &right.steps));
        assert!(Arc::ptr_eq(
            &left.world.definitions,
            &right.world.definitions
        ));
    }
    #[test]
    fn primary_effects_restore_every_instruction_with_frozen_stats_and_exact_events() {
        fn finish(mut session: NativeSession, restore: bool) -> (Value, Vec<Value>) {
            let mut events = Vec::new();
            for _ in 0..100 {
                if restore {
                    session = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
                }
                let result = session.advance_prefix(1).unwrap();
                events.extend(result.events.iter().map(|event| session.event_json(event)));
                if let Some(request) = result.request {
                    let value = if request.can_skip {
                        json!("SKIP")
                    } else {
                        match &request.choices {
                            RequestChoices::Units { values, .. } => {
                                json!(session.symbols.name(values[0].0))
                            }
                            RequestChoices::Hexes(values) => {
                                serde_json::to_value(values[0]).unwrap()
                            }
                            RequestChoices::Defense(_) => json!("PASS"),
                            RequestChoices::Numbers { values, .. } => json!(values.last().unwrap()),
                            _ => panic!("unexpected primary request"),
                        }
                    };
                    let selection = session.import_selection(&value).unwrap();
                    session.submit(request.id, selection).unwrap();
                }
                if session.stack.is_empty() {
                    return (session.semantic_state().unwrap(), events);
                }
            }
            panic!("primary continuation did not finish")
        }
        for effect in [
            "noble_blade",
            "onslaught",
            "leaping_strike",
            "poisoned_dagger",
            "liquid_leap",
            "arcane_whirlpool",
            "brutal_jab",
            "blink_strike",
            "mad_dash",
            "rogue_wave",
        ] {
            let root = NativeSession::compile(source(&format!("primary-{effect}-items"))).unwrap();
            let expected = finish(root.clone(), false);
            let restored = finish(root, true);
            assert_eq!(restored, expected, "{effect}");
            assert!(!restored.0.to_string().contains(crate::primary::HERO));
            assert!(!restored.0.to_string().contains(crate::primary::CARD));
        }
    }
    #[test]
    fn invalid_loaded_map_placement_is_a_rule_error_before_world_mutation() {
        let mut envelope = source("primary-primitive-place-literal");
        envelope.state["execution_context"]["dest"] = json!({"q":1000,"r":-1000,"s":0});
        let mut session = NativeSession::compile(envelope).unwrap();
        let before = session.world.position(session.planning.actor.unwrap());
        assert!(matches!(
            session.advance(&ExecutionBudget::default()),
            Err(EngineError::Rule(_))
        ));
        assert_eq!(
            session.world.position(session.planning.actor.unwrap()),
            before
        );
    }

    #[test]
    fn push_workers_keep_positions_collision_and_victim_context_private() {
        let root = NativeSession::compile(source("push-ordinary")).unwrap();
        let before = root.semantic_state().unwrap();
        let mut left = root.clone();
        let mut right = root.clone();
        let distance = left.symbols.lookup("distance").unwrap();
        left.context.set(distance, Datum::Int(1));
        right.context.set(distance, Datum::Int(3));
        let a = left.advance_prefix(1).unwrap();
        let b = right.advance_prefix(1).unwrap();
        let target = left.symbols.lookup("hero_brogan").unwrap();
        assert_ne!(left.world.position(target), right.world.position(target));
        assert_eq!(left.event_json(&a.events[0])["metadata"]["distance"], 1);
        assert_eq!(right.event_json(&b.events[0])["metadata"]["distance"], 3);
        assert_eq!(root.semantic_state().unwrap(), before);
        assert!(Arc::ptr_eq(&left.steps, &right.steps));
        assert!(Arc::ptr_eq(
            &left.world.definitions,
            &right.world.definitions
        ));
    }
    #[test]
    fn primary_ranged_operands_are_frozen_before_export_and_restore() {
        for (variant, ranged) in [("melee", false), ("ranged", true)] {
            let mut original =
                NativeSession::compile(source(&format!("primary-rogue_wave-{variant}"))).unwrap();
            original.advance_prefix(1).unwrap();
            let exported = original.export_envelope().unwrap();
            assert_eq!(
                exported.state["execution_stack"]
                    .as_array()
                    .unwrap()
                    .last()
                    .unwrap()["is_ranged"],
                ranged
            );
            let mut restored = NativeSession::compile(exported).unwrap();
            restored.advance_prefix(1).unwrap();
            assert_eq!(
                restored.semantic_state().unwrap()["execution_context"]["attack_is_ranged"],
                ranged
            );
        }
    }
    #[test]
    fn primary_defense_marker_workers_keep_actor_flags_and_markers_private() {
        let root = NativeSession::compile(source("defense-effect-atomic-close_call")).unwrap();
        let before = root.semantic_state().unwrap();
        let mut worker = root.clone();
        worker.advance_prefix(3).unwrap();
        let halfway = worker.semantic_state().unwrap();
        assert_eq!(halfway["current_actor_id"], "hero_bain");
        assert_eq!(halfway["execution_context"]["auto_block"], true);
        assert_eq!(halfway["markers"]["bounty"]["target_id"], "hero_brogan");
        let event = worker.advance_prefix(1).unwrap();
        assert_eq!(event.events.len(), 1);
        assert!(matches!(
            event.events[0],
            NativeEvent::Marker(MarkerEvent::Placed { .. })
        ));
        assert_eq!(
            worker.semantic_state().unwrap()["markers"]["bounty"]["target_id"],
            "hero_bain"
        );
        worker.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(
            worker.semantic_state().unwrap()["current_actor_id"],
            "hero_brogan"
        );
        assert_eq!(root.semantic_state().unwrap(), before);
        assert!(Arc::ptr_eq(
            &root.world.definitions,
            &worker.world.definitions
        ));
        assert!(Arc::ptr_eq(&root.reaction_rules, &worker.reaction_rules));
    }
    #[test]
    fn defense_effect_generated_flags_markers_and_actor_restoration_survive_export() {
        for name in [
            "defense-effect-atomic-aspiring_duelist",
            "defense-effect-atomic-close_call",
            "defense-effect-atomic-narrow_escape",
        ] {
            let root = NativeSession::compile(source(name)).unwrap();
            let mut complete = root.clone();
            complete.advance(&ExecutionBudget::default()).unwrap();
            let expected = complete.semantic_state().unwrap();
            for steps in 0..=6 {
                let mut partial = root.clone();
                partial.advance_prefix(steps).unwrap();
                let mut restored =
                    NativeSession::compile(partial.export_envelope().unwrap()).unwrap();
                assert_eq!(
                    partial.semantic_state().unwrap(),
                    restored.semantic_state().unwrap()
                );
                restored.advance(&ExecutionBudget::default()).unwrap();
                assert_eq!(restored.semantic_state().unwrap(), expected);
            }
        }
    }
    #[test]
    fn projectile_on_block_target_and_discard_prompts_execute_the_custom_body() {
        let mut envelope = source("defense-effect-reflect_projectiles-text-True");
        envelope.state["execution_context"]["block_succeeded"] = json!(true);
        let mut step = combat_template("on_block").unwrap();
        step["step_id"] = json!("fixture/projectile-on-block");
        envelope.state["execution_stack"] = json!([step]);
        let mut session = NativeSession::compile(envelope).unwrap();
        let wasp = session.symbols.lookup("hero_wasp").unwrap();
        let victim = session.symbols.lookup("hero_brogan").unwrap();
        let origin = session.world.position(wasp).unwrap();
        let near = session
            .world
            .tile_hexes()
            .iter()
            .copied()
            .find(|h| origin.distance(*h) <= 3 && !session.world.obstacle(*h))
            .unwrap();
        session.world.move_entity(victim, near).unwrap();
        let target = session
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        assert_eq!(target.owner, wasp);
        let RequestChoices::Units { values, .. } = &target.choices else {
            panic!("expected custom target choice");
        };
        assert!(values.iter().any(|(id, _)| *id == victim));
        session.submit(target.id, Datum::Text(victim)).unwrap();
        let discard = session
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        assert_eq!(discard.owner, victim);
        let RequestChoices::CardIds { values, .. } = &discard.choices else {
            panic!("expected forced discard");
        };
        let chosen = values[0];
        let printed = session.symbols.name(chosen).to_owned();
        session.submit(discard.id, Datum::Text(chosen)).unwrap();
        session.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(
            session.semantic_state().unwrap()["turn_discard_log"]["hero_brogan"],
            json!([printed])
        );
        assert_eq!(session.planning.actor, Some(victim));
    }
    #[test]
    fn card_selection_workers_keep_choices_and_metadata_private() {
        let mut root = NativeSession::compile(source("card-select-hand")).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let RequestChoices::CardIds { values, .. } = &request.choices else {
            panic!("expected card choice");
        };
        let before = root.semantic_state().unwrap();
        let mut left = root.clone();
        let mut right = root.clone();
        left.submit(request.id, Datum::Text(values[0])).unwrap();
        right.submit(request.id, Datum::Text(values[1])).unwrap();
        left.advance(&ExecutionBudget::default()).unwrap();
        right.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(root.semantic_state().unwrap(), before);
        assert_ne!(
            left.semantic_state().unwrap()["execution_context"]["chosen"],
            right.semantic_state().unwrap()["execution_context"]["chosen"]
        );
        assert_ne!(
            left.semantic_state().unwrap()["execution_context"]["chosen_color"],
            right.semantic_state().unwrap()["execution_context"]["chosen_color"]
        );
        assert!(Arc::ptr_eq(&left.steps, &right.steps));
    }
    #[test]
    fn forced_discard_snapshots_victim_and_restores_before_selection_and_discard() {
        let mut session = NativeSession::compile(source("card-select-force-key")).unwrap();
        session.advance_prefix(1).unwrap();
        let victim_key = session.symbols.lookup("victim").unwrap();
        let different = session.symbols.lookup("hero_arien").unwrap();
        session.context.set(victim_key, Datum::Text(different));
        let mut restored = NativeSession::compile(session.export_envelope().unwrap()).unwrap();
        let request = restored
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        assert_eq!(restored.symbols.name(request.owner), "hero_brogan");
        let RequestChoices::CardIds { values, .. } = &request.choices else {
            panic!("expected captured victim's cards");
        };
        let chosen = values[0];
        let printed = restored.symbols.name(chosen).to_owned();
        restored.submit(request.id, Datum::Text(chosen)).unwrap();
        restored.advance_prefix(1).unwrap();
        let mut final_step = NativeSession::compile(restored.export_envelope().unwrap()).unwrap();
        final_step.advance(&ExecutionBudget::default()).unwrap();
        let state = final_step.semantic_state().unwrap();
        assert_eq!(state["turn_discard_log"]["hero_brogan"], json!([printed]));
        assert_eq!(state["execution_context"]["victim"], "hero_arien");
        assert_eq!(
            state["execution_context"]["discarded_card_owner_id"],
            "hero_brogan"
        );
    }
    #[test]
    fn attack_workers_choose_different_targets_without_sharing_mutations() {
        let mut root = NativeSession::compile(source("attack-minion")).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let before = root.semantic_state().unwrap();
        let mut minion = root.clone();
        let mut hero = root.clone();
        let enemy = root.symbols.lookup("enemy").unwrap();
        let blue = root.symbols.lookup("blue").unwrap();
        minion.submit(request.id, Datum::Text(enemy)).unwrap();
        hero.submit(request.id, Datum::Text(blue)).unwrap();
        minion.advance(&ExecutionBudget::default()).unwrap();
        let defense = hero
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        assert!(matches!(defense.choices, RequestChoices::Defense(_)));
        assert_eq!(root.semantic_state().unwrap(), before);
        assert!(minion.world.position(enemy).is_none());
        assert!(hero.world.position(enemy).is_some());
        assert_eq!(hero.world.position(blue), root.world.position(blue));
        assert!(Arc::ptr_eq(
            &minion.world.definitions,
            &hero.world.definitions
        ));
    }
    #[test]
    fn attack_bindings_and_optional_followups_restore_between_steps_and_prompts() {
        for name in ["attack-minion", "attack-optional-minion", "attack-bonuses"] {
            let envelope = source(name);
            let mut uninterrupted = NativeSession::compile(envelope.clone()).unwrap();
            let request = uninterrupted
                .advance(&ExecutionBudget::default())
                .unwrap()
                .request
                .unwrap();
            let target = uninterrupted.symbols.lookup("enemy").unwrap();
            uninterrupted
                .submit(request.id, Datum::Text(target))
                .unwrap();
            uninterrupted.advance(&ExecutionBudget::default()).unwrap();
            let expected = uninterrupted.semantic_state().unwrap();
            let mut partial = NativeSession::compile(envelope).unwrap();
            assert_eq!(
                partial
                    .advance(&ExecutionBudget {
                        max_steps: 1,
                        ..Default::default()
                    })
                    .unwrap_err(),
                EngineError::StepLimit
            );
            let mut restored = NativeSession::compile(partial.export_envelope().unwrap()).unwrap();
            assert_eq!(
                partial.semantic_state().unwrap(),
                restored.semantic_state().unwrap()
            );
            let prompt = restored
                .advance(&ExecutionBudget::default())
                .unwrap()
                .request
                .unwrap();
            let selection = restored.import_selection(&json!("enemy")).unwrap();
            restored.submit(prompt.id, selection).unwrap();
            restored.advance(&ExecutionBudget::default()).unwrap();
            assert_eq!(restored.semantic_state().unwrap(), expected);
        }
        let mut attack = NativeSession::compile(source("attack-preselected-hero")).unwrap();
        let request = attack
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let mut restored = NativeSession::compile(attack.export_envelope().unwrap()).unwrap();
        let pass = attack.symbols.lookup("PASS").unwrap();
        attack.submit(request.id, Datum::Text(pass)).unwrap();
        let restored_pass = restored.import_selection(&json!("PASS")).unwrap();
        let token = restored.import_request_token(&attack.request_token(request.id));
        restored.submit(token, restored_pass).unwrap();
        attack.advance(&ExecutionBudget::default()).unwrap();
        restored.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(
            attack.semantic_state().unwrap(),
            restored.semantic_state().unwrap()
        );
    }
    #[test]
    fn defense_workers_keep_card_knowledge_history_and_action_state_private() {
        let mut root = NativeSession::compile(source("reaction-secondary-combat")).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let RequestChoices::Defense(menu) = &request.choices else {
            panic!("expected defense menu");
        };
        let id = menu.cards.iter().find(|c| !c.primary).unwrap().id;
        let before = root.semantic_state().unwrap();
        let mut defense = root.clone();
        let mut pass = root.clone();
        defense.submit(request.id, Datum::Text(id)).unwrap();
        let pass_id = pass.symbols.lookup("PASS").unwrap();
        pass.submit(request.id, Datum::Text(pass_id)).unwrap();
        defense.advance(&ExecutionBudget::default()).unwrap();
        pass.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(root.semantic_state().unwrap(), before);
        let state = defense.semantic_state().unwrap();
        assert!(
            state["public_revealed_cards"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["card_id"] == root.symbols.name(id))
        );
        assert_eq!(
            state["turn_discard_log"][root.symbols.name(menu.owner)],
            json!([root.symbols.name(id)])
        );
        assert_eq!(state["execution_context"]["current_action_type"], "ATTACK");
        assert!(
            pass.semantic_state().unwrap()["entity_locations"]
                .get(root.symbols.name(menu.owner))
                .is_none()
        );
        assert!(Arc::ptr_eq(
            &defense.world.definitions,
            &pass.world.definitions
        ));
    }
    #[test]
    fn defense_discard_and_passive_bindings_restore_between_generated_steps() {
        let mut root = NativeSession::compile(source("reaction-secondary-combat")).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let RequestChoices::Defense(menu) = &request.choices else {
            panic!("expected defense menu");
        };
        let id = menu.cards.iter().find(|c| !c.primary).unwrap().id;
        root.submit(request.id, Datum::Text(id)).unwrap();
        let mut complete = root.clone();
        complete.advance(&ExecutionBudget::default()).unwrap();
        let expected = complete.semantic_state().unwrap();
        for count in [1, 2] {
            let mut interrupted = root.clone();
            assert_eq!(
                interrupted
                    .advance(&ExecutionBudget {
                        max_steps: count,
                        ..Default::default()
                    })
                    .unwrap_err(),
                EngineError::StepLimit
            );
            let mut restored =
                NativeSession::compile(interrupted.export_envelope().unwrap()).unwrap();
            assert_eq!(
                restored.semantic_state().unwrap(),
                interrupted.semantic_state().unwrap()
            );
            restored.advance(&ExecutionBudget::default()).unwrap();
            assert_eq!(restored.semantic_state().unwrap(), expected);
        }
    }
    #[test]
    fn combat_workers_keep_life_gold_markers_and_occupancy_private() {
        let root = NativeSession::compile(source("combat-markers-razzle_piece_1")).unwrap();
        let before = root.semantic_state().unwrap();
        let mut worker = root.clone();
        worker.advance(&ExecutionBudget::default()).unwrap();
        let after = worker.semantic_state().unwrap();
        assert_eq!(root.semantic_state().unwrap(), before);
        assert_eq!(
            after["teams"]["RED"]["life_counters"].as_i64().unwrap(),
            before["teams"]["RED"]["life_counters"].as_i64().unwrap() - 2
        );
        assert!(after["markers"]["bounty"]["target_id"].is_null());
        assert!(after["entity_locations"].get("razzle_piece_1").is_none());
        assert!(after["entity_locations"].get("razzle_piece_2").is_none());
        assert_eq!(
            after["teams"]["RED"]["heroes"][0]["gold"].as_i64().unwrap(),
            before["teams"]["RED"]["heroes"][0]["gold"]
                .as_i64()
                .unwrap()
                + 1
        );
        assert!(Arc::ptr_eq(
            &worker.world.definitions,
            &root.world.definitions
        ));
        assert!(Arc::ptr_eq(&worker.steps, &root.steps));
    }
    #[test]
    fn generated_defeat_removal_and_game_over_restore_at_step_boundaries() {
        for name in ["combat-hit", "combat-annihilation-1", "combat-card-alias"] {
            let source = source(name);
            let mut complete = NativeSession::compile(source.clone()).unwrap();
            complete.advance(&ExecutionBudget::default()).unwrap();
            let expected = complete.semantic_state().unwrap();
            let boundaries: &[usize] = if name == "combat-card-alias" {
                &[1]
            } else {
                &[1, 2]
            };
            for &count in boundaries {
                let mut interrupted = NativeSession::compile(source.clone()).unwrap();
                assert_eq!(
                    interrupted
                        .advance(&ExecutionBudget {
                            max_steps: count,
                            ..Default::default()
                        })
                        .unwrap_err(),
                    EngineError::StepLimit
                );
                let envelope = interrupted.export_envelope().unwrap();
                let before = interrupted.semantic_state().unwrap();
                let mut restored = NativeSession::compile(envelope).unwrap();
                assert_eq!(before, restored.semantic_state().unwrap());
                interrupted.advance(&ExecutionBudget::default()).unwrap();
                restored.advance(&ExecutionBudget::default()).unwrap();
                assert_eq!(restored.semantic_state().unwrap(), expected);
                assert_eq!(interrupted.semantic_state().unwrap(), expected);
            }
        }
    }
    #[test]
    fn unported_combat_effect_closure_rejects_before_rewards_or_removal() {
        let mut envelope = source("combat-hit");
        // NativeWorld only inspects the non-empty effect collection here; full
        // effect compilation is deliberately unavailable, so no transition runs.
        envelope.state["active_effects"] = json!([{"effect_type":"MINION_PROTECTION"}]);
        assert!(matches!(
            NativeSession::compile(envelope),
            Err(EngineError::Unsupported(_))
        ));
    }
    #[test]
    fn restored_identifier_counters_fail_explicitly_instead_of_overflowing() {
        let mut envelope = source("travel-sequence");
        envelope.state["execution_stack"][0]["step_id"] =
            json!(format!("native-step-{}", u64::MAX - 1));
        let mut session = NativeSession::compile(envelope).unwrap();
        assert!(
            matches!(session.advance(&ExecutionBudget::default()),Err(EngineError::Unsupported(message)) if message.contains("step generation counter exhausted"))
        );
        let mut envelope = source("number-input-resume");
        envelope.state["execution_stack"][0]["pending_request_id"] =
            json!(format!("native-request-{}", u64::MAX - 1));
        let mut session = NativeSession::compile(envelope).unwrap();
        let request = session
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        session.submit(request.id, Datum::Null).unwrap();
        assert!(
            matches!(session.advance(&ExecutionBudget::default()),Err(EngineError::Unsupported(message)) if message.contains("request generation counter exhausted"))
        );
    }
    #[test]
    fn travel_workers_relocate_independently_and_emit_placement_not_movement() {
        let mut root = NativeSession::compile(source("travel-sequence")).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let RequestChoices::Hexes(choices) = request.choices else {
            panic!("expected destination choice");
        };
        let before = root.semantic_state().unwrap();
        let mut left = root.clone();
        let mut right = root.clone();
        left.submit(request.id, native_hex(choices[0], root.coordinates))
            .unwrap();
        right
            .submit(request.id, native_hex(choices[1], root.coordinates))
            .unwrap();
        let events = left.advance(&ExecutionBudget::default()).unwrap().events;
        assert!(matches!(
            events.as_slice(),
            [NativeEvent::UnitPlaced { .. }]
        ));
        right.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(root.semantic_state().unwrap(), before);
        let actor = root.planning.actor.unwrap();
        assert_eq!(left.world.position(actor), Some(choices[0]));
        assert_eq!(right.world.position(actor), Some(choices[1]));
        assert!(Arc::ptr_eq(
            &left.world.definitions,
            &right.world.definitions
        ));
    }
    #[test]
    fn compact_travel_continuation_restores_real_card_aliases_and_request_identity() {
        // Use a registered full game, rather than a cardless board.
        let mut root =
            NativeSession::compile(source("travel-forgotten_island-change-False")).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let envelope = root.export_envelope().unwrap();
        envelope.validate().unwrap();
        let mut restored = NativeSession::compile(envelope).unwrap();
        let original_hero = root.world.hero(root.planning.actor.unwrap()).unwrap();
        let restored_hero = restored
            .world
            .hero(restored.planning.actor.unwrap())
            .unwrap();
        assert_eq!(original_hero.hand, restored_hero.hand);
        assert_eq!(original_hero.deck, restored_hero.deck);
        assert!(
            restored_hero
                .hand
                .iter()
                .all(|id| restored_hero.deck.contains(id))
        );
        let resumed = restored
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        assert_eq!(root.request_json(&request), restored.request_json(&resumed));
        let RequestChoices::Hexes(choices) = request.choices else {
            panic!("expected destination choice");
        };
        root.submit(request.id, native_hex(choices[0], root.coordinates))
            .unwrap();
        restored
            .submit(resumed.id, native_hex(choices[0], restored.coordinates))
            .unwrap();
        root.advance(&ExecutionBudget::default()).unwrap();
        restored.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(
            root.semantic_state().unwrap(),
            restored.semantic_state().unwrap()
        );
    }
    #[test]
    fn acting_piece_binding_is_private_to_each_search_worker() {
        let mut root = NativeSession::compile(source("unit-choose-piece-invalid")).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let RequestChoices::Units { values, .. } = &request.choices else {
            panic!("expected piece choice");
        };
        let mut left = root.clone();
        let mut right = root.clone();
        left.submit(request.id, Datum::Text(values[0].0)).unwrap();
        right.submit(request.id, Datum::Text(values[1].0)).unwrap();
        left.advance(&ExecutionBudget::default()).unwrap();
        right.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(root.world.acting_piece, None);
        assert_eq!(left.world.acting_piece, Some(values[0].0));
        assert_eq!(right.world.acting_piece, Some(values[1].0));
        assert!(Arc::ptr_eq(
            &left.world.definitions,
            &right.world.definitions
        ));
        assert_ne!(
            left.semantic_state().unwrap(),
            right.semantic_state().unwrap()
        );
    }
    #[test]
    fn heavy_immunity_tracks_each_workers_support_position() {
        let root = NativeSession::compile(source("unit-heavy-immune")).unwrap();
        let heavy = root.symbols.lookup("heavy").unwrap();
        let support = root.symbols.lookup("support").unwrap();
        let actor = root.planning.actor;
        assert!(root.world.ordinary_immune(heavy, actor));
        let mut worker = root.clone();
        worker
            .world
            .move_entity(support, Hex::new(4, -4, 0).unwrap())
            .unwrap();
        assert!(!worker.world.ordinary_immune(heavy, actor));
        assert!(root.world.ordinary_immune(heavy, actor));
    }
    #[test]
    fn retrieval_workers_can_keep_different_cards_without_mutating_root() {
        let mut root = NativeSession::compile(source("retrieve-unresolved-first")).unwrap();
        let request = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let RequestChoices::Cards(choices) = &request.choices else {
            panic!("expected cards");
        };
        let before = root.semantic_state().unwrap();
        let mut left = root.clone();
        let mut right = root.clone();
        left.submit(request.id, Datum::Text(choices[0].0)).unwrap();
        right.submit(request.id, Datum::Text(choices[1].0)).unwrap();
        assert_eq!(
            left.advance(&ExecutionBudget::default())
                .unwrap()
                .events
                .len(),
            1
        );
        assert_eq!(
            right
                .advance(&ExecutionBudget::default())
                .unwrap()
                .events
                .len(),
            1
        );
        assert_eq!(root.semantic_state().unwrap(), before);
        assert_ne!(
            left.semantic_state().unwrap(),
            right.semantic_state().unwrap()
        );
        assert!(Arc::ptr_eq(
            &left.world.definitions,
            &right.world.definitions
        ));
        let hero = root
            .world
            .heroes
            .iter()
            .find(|h| root.symbols.name(h.id) == "hero_emmitt")
            .unwrap()
            .id;
        assert_ne!(
            left.world.hero(hero).unwrap().current,
            right.world.hero(hero).unwrap().current
        );
    }
    #[test]
    fn independent_workers_share_only_immutable_program_data() {
        let mut root = NativeSession::compile(source("number-input-resume")).unwrap();
        root.advance(&ExecutionBudget::default()).unwrap();
        let before = root.semantic_state().unwrap();
        let mut child = root.clone();
        assert!(Arc::ptr_eq(&root.source, &child.source));
        assert!(Arc::ptr_eq(&root.steps, &child.steps));
        let token = child.last_request.as_ref().unwrap().id;
        child.submit(token, Datum::Int(2)).unwrap();
        child.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(root.semantic_state().unwrap(), before);
        assert_ne!(child.semantic_state().unwrap(), before);
        assert_eq!(root.last_request.as_ref().unwrap().id, token);
    }
    #[test]
    fn spatial_workers_share_definitions_but_never_positions_or_occupancy() {
        let mut root = NativeSession::compile(source("movement-resume-events")).unwrap();
        root.advance(&ExecutionBudget::default()).unwrap();
        let before = root.semantic_state().unwrap();
        let mut child = root.clone();
        assert!(Arc::ptr_eq(
            &root.world.definitions,
            &child.world.definitions
        ));
        let request = child.last_request.as_ref().unwrap().id;
        child.submit(request, Datum::Int(1)).unwrap();
        let moved = child.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(moved.events.len(), 1);
        assert_eq!(root.semantic_state().unwrap(), before);
        assert_ne!(child.semantic_state().unwrap(), before);
        let symbol = child.symbols.lookup("hero_arien").unwrap();
        assert_ne!(root.world.position(symbol), child.world.position(symbol));
    }
    #[test]
    fn generated_movement_continuations_restore_without_losing_bindings_or_identity() {
        let mut session = NativeSession::compile(source("macro-default")).unwrap();
        session.advance(&ExecutionBudget::default()).unwrap();
        let mut envelope = source("macro-default");
        envelope.state = session.semantic_state().unwrap();
        let mut restored = NativeSession::compile(envelope).unwrap();
        assert!(restored.next_step >= 2);
        let request = restored
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        assert_eq!(
            restored.request_token(request.id),
            session.request_token(session.last_request.as_ref().unwrap().id)
        );
        let choice = serde_json::json!({"q":1,"r":-1,"s":0});
        let value = restored.import_selection(&choice).unwrap();
        restored.submit(request.id, value).unwrap();
        restored.advance(&ExecutionBudget::default()).unwrap();
        let value = session.import_selection(&choice).unwrap();
        session
            .submit(session.last_request.as_ref().unwrap().id, value)
            .unwrap();
        session.advance(&ExecutionBudget::default()).unwrap();
        assert_eq!(
            restored.semantic_state().unwrap(),
            session.semantic_state().unwrap()
        );
    }
    #[test]
    fn deadlines_and_cancellation_preserve_waiting_state() {
        let mut session = NativeSession::compile(source("number-input-resume")).unwrap();
        session.advance(&ExecutionBudget::default()).unwrap();
        let before = session.semantic_state().unwrap();
        let token = session.last_request.as_ref().unwrap().id;
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            session
                .advance(&ExecutionBudget {
                    cancelled: Some(&cancelled),
                    ..Default::default()
                })
                .unwrap_err(),
            EngineError::Cancelled
        );
        assert_eq!(session.last_request.as_ref().unwrap().id, token);
        assert_eq!(session.semantic_state().unwrap(), before);
        assert_eq!(
            session
                .advance(&ExecutionBudget {
                    deadline: Some(Instant::now()),
                    ..Default::default()
                })
                .unwrap_err(),
            EngineError::Deadline
        );
        assert_eq!(session.semantic_state().unwrap(), before);
    }
    #[test]
    fn unsupported_nested_instruction_rejects_whole_root() {
        let mut source = source("foreach-paused-copies");
        source.state["execution_stack"][1]["steps_template"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"damage"}));
        assert!(matches!(
            NativeSession::compile(source),
            Err(EngineError::Unsupported(_))
        ));
    }
    #[test]
    fn consumed_request_ids_do_not_reappear_after_restore_without_a_pending_frame() {
        let mut root = NativeSession::compile(source("number-input-resume")).unwrap();
        let first = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let old = root.request_token(first.id);
        root.submit(first.id, Datum::Int(2)).unwrap();
        let consumed = root.advance_prefix(1).unwrap();
        assert!(consumed.request.is_none());
        assert!(root.stack.iter().all(|f| f.request.is_none()));
        let encoded = serde_json::to_value(root.export_envelope().unwrap()).unwrap();
        let envelope = serde_json::from_value(encoded).unwrap();
        let mut restored = NativeSession::compile(envelope).unwrap();
        let next = restored
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let next_name = restored.request_token(next.id);
        assert_ne!(
            next_name, old,
            "a completed prompt must not identify a later prompt"
        );
        let before = restored.semantic_state().unwrap();
        let stale = restored.import_request_token(&old);
        assert!(matches!(
            restored.submit(stale, Datum::Text(restored.yes)),
            Err(EngineError::InvalidInput(_))
        ));
        assert_eq!(restored.semantic_state().unwrap(), before);
        let uninterrupted = root
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        assert_eq!(root.request_token(uninterrupted.id), next_name);
        restored.submit(next.id, Datum::Text(restored.yes)).unwrap();
    }

    #[test]
    fn request_wire_identity_is_exact_for_generated_and_imported_prompts() {
        let mut session = NativeSession::compile(source("number-input-resume")).unwrap();
        let request = session
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        assert_eq!(session.request_token(request.id), "native-request-1");
        let before = session.semantic_state().unwrap();
        for alias in [
            "native-request-01",
            "native-request-+1",
            "native-request-0001",
        ] {
            let token = session.import_request_token(alias);
            assert!(
                matches!(
                    session.submit(token, Datum::Int(2)),
                    Err(EngineError::InvalidInput(_))
                ),
                "{alias}"
            );
            assert_eq!(session.semantic_state().unwrap(), before);
        }
        let mut envelope = session.export_envelope().unwrap();
        let frame = envelope.state["execution_stack"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap();
        frame["pending_request_id"] = json!("native-request-01");
        let mut imported = NativeSession::compile(envelope).unwrap();
        let request = imported
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        assert_eq!(imported.request_token(request.id), "native-request-01");
        let alias = imported.import_request_token("native-request-1");
        assert!(matches!(
            imported.submit(alias, Datum::Int(2)),
            Err(EngineError::InvalidInput(_))
        ));
        let exact = imported.import_request_token("native-request-01");
        imported.submit(exact, Datum::Int(2)).unwrap();
    }
    #[test]
    fn completed_generated_steps_keep_identity_history_outside_game_state() {
        let mut root = NativeSession::compile(source("travel-sequence")).unwrap();
        for _ in 0..20 {
            let result = root.advance(&ExecutionBudget::default()).unwrap();
            if let Some(request) = result.request {
                let wire = root.request_json(&request);
                let value = wire["options"][0]["metadata"]["hex"].clone();
                let value = root.import_selection(&value).unwrap();
                root.submit(request.id, value).unwrap();
            }
            if root.stack.is_empty() {
                break;
            }
        }
        assert!(root.stack.is_empty());
        assert!(root.next_step > 0 && root.next_request > 0);
        let semantic = root.semantic_state().unwrap();
        assert!(semantic.get("native_resume").is_none());
        let encoded = serde_json::to_value(root.export_envelope().unwrap()).unwrap();
        let envelope = serde_json::from_value(encoded).unwrap();
        let mut restored = NativeSession::compile(envelope).unwrap();
        assert_eq!(restored.semantic_state().unwrap(), semantic);
        assert_eq!(restored.next_step, root.next_step);
        assert_eq!(restored.next_request, root.next_request);
        assert!(
            restored
                .advance(&ExecutionBudget::default())
                .unwrap()
                .request
                .is_none()
        );
        assert_eq!(
            restored.export_envelope().unwrap().native_resume,
            root.export_envelope().unwrap().native_resume
        );
    }
    #[test]
    fn resume_rejects_regressing_and_exhausted_identity_history() {
        let mut root = NativeSession::compile(source("travel-sequence")).unwrap();
        root.advance(&ExecutionBudget::default()).unwrap();
        let envelope = root.export_envelope().unwrap();
        for request_counter in [false, true] {
            let mut bad = envelope.clone();
            let metadata = bad.native_resume.as_mut().unwrap();
            if request_counter {
                metadata.request_high_water = 0;
            } else {
                metadata.step_high_water = 0;
            }
            assert!(
                matches!(NativeSession::compile(bad), Err(EngineError::Unsupported(message)) if message.contains("precede live"))
            );
        }
        for request_counter in [false, true] {
            let mut bad = envelope.clone();
            let metadata = bad.native_resume.as_mut().unwrap();
            if request_counter {
                metadata.request_high_water = u64::MAX;
            } else {
                metadata.step_high_water = u64::MAX;
            }
            assert!(
                matches!(NativeSession::compile(bad), Err(EngineError::Unsupported(message)) if message.contains("counter exhausted"))
            );
        }
    }
    #[test]
    fn generated_template_id_prefixes_reserve_their_generation() {
        let mut envelope = source("travel-sequence");
        envelope.state["execution_stack"][1]["step_id"] = json!("native-step-200-template-3");
        let mut root = NativeSession::compile(envelope).unwrap();
        assert_eq!(root.next_step, 200);
        root.advance_prefix(1).unwrap();
        let wire = root.semantic_state().unwrap();
        let names = wire["execution_stack"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s["step_id"].as_str())
            .collect::<Vec<_>>();
        assert!(names.contains(&"native-step-201") && names.contains(&"native-step-202"));
    }

    #[test]
    fn imported_requests_work_and_rejected_choice_gets_fresh_identity() {
        let mut source = source("number-input-resume");
        let frames = source.state["execution_stack"].as_array_mut().unwrap();
        // Drop automatic prefix and import the paused number step directly.
        frames.pop();
        frames.pop();
        frames.last_mut().unwrap()["pending_request_id"] = json!("native-request-1");
        let mut session = NativeSession::compile(source).unwrap();
        let first = session
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        let imported = session.import_request_token("native-request-1");
        assert_eq!(imported, first.id);
        session.submit(imported, Datum::Null).unwrap();
        let next = session
            .advance(&ExecutionBudget::default())
            .unwrap()
            .request
            .unwrap();
        assert_ne!(
            session.request_token(first.id),
            session.request_token(next.id)
        );
        assert!(matches!(
            session.submit(imported, Datum::Int(2)),
            Err(EngineError::InvalidInput(_))
        ));
    }
    #[test]
    fn automatic_step_limit_is_real_and_does_not_count_fake_progress() {
        let mut session = NativeSession::compile(source("number-input-resume")).unwrap();
        assert_eq!(
            session
                .advance(&ExecutionBudget {
                    max_steps: 1,
                    ..Default::default()
                })
                .unwrap_err(),
            EngineError::StepLimit
        );
        let state = session.semantic_state().unwrap();
        assert_eq!(state["execution_context"]["enabled"], true);
        assert!(state["execution_context"].get("count").is_none());
    }
}
