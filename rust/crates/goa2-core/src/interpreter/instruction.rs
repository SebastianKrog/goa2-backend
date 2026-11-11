use super::RequestToken;
use crate::{
    actions::ActionStep,
    attack::{AttackBinding, AttackSpec},
    card_selection::{CardSelectSpec, ForceBinding},
    combat::{DefeatBinding, GameOutcome},
    data::{Datum, Symbol},
    geometry::Hex,
    primary::PrimaryValues,
    spatial::HexFilter,
    stats::{MarkerKind, Stat},
    targeting::UnitFilter,
};
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone, Debug, Hash)]
pub(super) struct EffectBinding {
    pub(super) card: Symbol,
    pub(super) context: Option<(Symbol, Datum)>,
    pub(super) range: Option<i64>,
    pub(super) primary: Option<PrimaryValues>,
}
#[derive(Clone, Debug)]
pub(super) struct PrimaryProgram {
    pub(super) bodies: Vec<(Symbol, Vec<usize>)>,
    pub(super) movement: usize,
    pub(super) attack: usize,
    pub(super) defense: usize,
    pub(super) skill: usize,
}
#[derive(Clone, Debug)]
pub(super) struct DefenseProgram {
    pub(super) enter: usize,
    pub(super) exit: usize,
    pub(super) flag: usize,
    pub(super) place: usize,
    pub(super) remove: usize,
    pub(super) reflect_select: Option<usize>,
    pub(super) reflect_discard: Option<usize>,
    pub(super) deflect_select: Option<usize>,
    pub(super) deflect_discard: Option<usize>,
}
#[derive(Clone, Debug)]
pub(super) struct FlowProgram {
    pub(super) entries: Vec<(&'static str, usize)>,
}
#[derive(Clone, Debug)]
pub(super) struct BattleProgram {
    pub(super) remove: usize,
    pub(super) place: usize,
    pub(super) victory: usize,
    pub(super) push: usize,
    pub(super) casualties: usize,
    pub(super) displace: usize,
}
#[derive(Clone, Debug)]
pub(super) struct PassiveBody {
    pub(super) effect: Symbol,
    pub(super) owner: Symbol,
    pub(super) templates: Vec<usize>,
}
#[derive(Clone, Debug, Hash)]
pub(super) struct Frame {
    pub(super) instruction: usize,
    pub(super) object: Option<crate::objects::ObjectId>,
    pub(super) pending: Option<Datum>,
    pub(super) request: Option<RequestToken>,
    pub(super) index: i64,
    pub(super) repeat_checked: bool,
    pub(super) unit_override: Option<Option<Symbol>>,
    pub(super) generated: Option<u64>,
    pub(super) defeat_prompt_victim: Option<Symbol>,
    pub(super) range_override: Option<i32>,
    pub(super) through_override: Option<bool>,
    pub(super) defeat_override: Option<DefeatBinding>,
    pub(super) outcome_override: Option<GameOutcome>,
    pub(super) discard_override: Option<(Symbol, Symbol)>,
    pub(super) attack_binding: Option<AttackBinding>,
    pub(super) attack_followup: Option<Symbol>,
    pub(super) effect: Option<EffectBinding>,
    pub(super) offer_override: Option<crate::passives::Offer>,
    pub(super) forced: Option<ForceBinding>,
    pub(super) mine_trigger: Option<(Option<Symbol>, Symbol, Option<Symbol>)>,
    pub(super) mine_snapshot: Option<(Vec<Symbol>, Symbol)>,
    pub(super) protection: Option<crate::protection::ProtectionBinding>,
    pub(super) mine_prechosen: bool,
    pub(super) pre_move: Option<(Symbol, i32)>,
    pub(super) pre_processed: Vec<Symbol>,
    pub(super) action: Option<ActionStep>,
    pub(super) multi: Vec<Symbol>,
    pub(super) zone_return: Option<crate::minion_return::ReturnBinding>,
    pub(super) battle: Option<crate::battle::BattleStep>,
    pub(super) continuation: Option<crate::planning::Continuation>,
}
#[derive(Clone, Debug)]
pub(super) struct Conditions {
    pub(super) active: Option<Symbol>,
    pub(super) skip: Option<Symbol>,
    pub(super) mandatory: bool,
}
#[derive(Clone, Copy, Debug)]
pub(super) enum CoordinateOp {
    TurnStart,
    Distance,
    Record,
    Compute,
    Adjacency,
}
#[derive(Clone, Debug)]
pub(super) enum Instruction {
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
pub(super) struct CompiledStep(pub(super) Arc<StepProgram>);

#[derive(Clone, Debug)]
pub(super) struct StepProgram {
    pub(super) primary_template: bool,
    pub(super) conditions: Conditions,
    pub(super) instruction: Instruction,
    pub(super) wire: Value,
    pub(super) initial: Frame,
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
