use crate::{
    actions::ActionMenu,
    combat::CombatEvent,
    data::{Datum, Symbol},
    geometry::Hex,
    markers::MarkerEvent,
    reactions::DefenseMenu,
    stats::Stat,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RequestToken {
    Imported(Symbol),
    Generated(u64),
}

#[derive(Clone, Debug, Hash, PartialEq)]
pub enum RequestChoices {
    SpellCards {
        values: Vec<crate::cards::CardId>,
        cast: bool,
        removed: usize,
        max: i64,
    },
    Minions(Vec<(Symbol, crate::world::MinionKind)>),
    Protection(Vec<crate::cards::CardId>),
    Mines(Vec<crate::mines::MinePath>),
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
    Runes(Vec<Symbol>),
    CardColors(Vec<Symbol>),
    RuneInscription {
        hero: Symbol,
        placed: Vec<(Symbol, Symbol)>,
        remaining: Vec<Symbol>,
    },
    Passive {
        card: Symbol,
        name: Symbol,
    },
    ConfirmAction,
    Laugh,
    MirrorDistances(Vec<(i64, i64)>),
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
    PrimaryReplacement {
        actor: Symbol,
        card: Symbol,
        primary: crate::world::ActionKind,
        replacement: crate::world::ActionKind,
    },
    Action(ActionMenu),
    Defense(DefenseMenu),
    Hexes(Vec<Hex>),
    ZoneHexes {
        values: Vec<Hex>,
        unit: Option<Symbol>,
    },
}
#[derive(Clone, Debug, Hash, PartialEq)]
pub struct NativeRequest {
    pub id: RequestToken,
    pub owner: Symbol,
    /// Original prompt actor; ownership is remapped only when the request is issued.
    pub(super) controlled: Option<Symbol>,
    pub(super) prompt: Symbol,
    pub(super) movement_range: Option<i32>,
    pub(super) pre_action_range: Option<i32>,
    pub choices: RequestChoices,
    pub(super) can_skip: bool,
    pub(super) discard: bool,
}
#[derive(Clone, Debug)]
pub enum NativeEvent {
    AdditionalLifeSpent {
        team: Symbol,
        target: Symbol,
        total: i64,
        change: i64,
    },
    LifePenaltyAnnihilation {
        winner: Symbol,
    },
    DirectionalMove {
        unit: Symbol,
        from: Hex,
        to: Hex,
        direction: i64,
    },
    ItemGranted {
        owner: Symbol,
        stat: crate::stats::Stat,
        amount: i64,
    },
    GuessSelected {
        actor: Option<Symbol>,
        victim: Symbol,
        attempt: i64,
    },
    HeroLaughed(Symbol),
    HeroPieceRemoved {
        owner: Symbol,
        piece: Symbol,
        from: Option<Hex>,
    },
    HeroPieceSpawned {
        owner: Symbol,
        piece: Symbol,
        to: Hex,
    },
    ResolvedCardsSwapped {
        actor: Option<Symbol>,
        target: Symbol,
        a: Symbol,
        b: Symbol,
    },
    HandCardRevealed {
        actor: Option<Symbol>,
        target: Symbol,
        owner: Symbol,
        card: Symbol,
        name: Symbol,
        color: Option<Symbol>,
        tier: Symbol,
        tier_value: i64,
    },
    GuessRevealed {
        actor: Option<Symbol>,
        victim: Symbol,
        attempt: i64,
        card: Symbol,
        name: Symbol,
        actual: Option<Symbol>,
        guessed: Symbol,
        correct: bool,
    },

    DeckCardSwapped {
        actor: Symbol,
        outgoing: Symbol,
        incoming: Symbol,
        state: &'static str,
        facedown: bool,
    },
    RunesPlaced {
        hero: Symbol,
        placed: Vec<(Symbol, Symbol)>,
    },
    JourneyScheduled {
        actor: Symbol,
        target: Symbol,
        kind: Symbol,
    },
    ActionControlScheduled {
        actor: Symbol,
        target: Symbol,
        card: Symbol,
    },
    InitiativeOverride {
        actor: Option<Symbol>,
        target: Symbol,
        card: Symbol,
        value: i64,
    },
    MineTriggered {
        actor: Option<Symbol>,
        target: Symbol,
        from: Hex,
        kind: Option<Symbol>,
        blast: bool,
    },
    EffectCreated(crate::effect_steps::EffectCreated),
    GoldEffect {
        actor: Option<Symbol>,
        target: Option<Symbol>,
        owner: Symbol,
        amount: Datum,
        lost: bool,
    },
    Spell(crate::spells::SpellEvent),
    CoinsStolen {
        actor: Symbol,
        victim: Symbol,
        amount: Datum,
    },
    LifeRestored {
        hero: Symbol,
        team: Symbol,
        change: i64,
        remaining: i64,
    },
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
    BoardEntityPlaced {
        actor: Option<Symbol>,
        entity: Symbol,
        from: Option<Hex>,
        to: Hex,
        obstacle: bool,
    },
    BoardEntityRemoved {
        actor: Option<Symbol>,
        entity: Symbol,
        from: Hex,
    },
    TokenPlaced {
        actor: Option<Symbol>,
        token: Symbol,
        to: Hex,
        kind: Symbol,
    },
    TokenReplaced {
        actor: Option<Symbol>,
        token: Symbol,
        from: Hex,
        to: Hex,
        kind: Symbol,
    },
    TokenWalked {
        actor: Option<Symbol>,
        token: Symbol,
        from: Hex,
        to: Hex,
        range: i64,
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
        effects_removed: usize,
    },
    Marker(MarkerEvent),
    Combat(CombatEvent),
    UnitPlaced {
        unit: Symbol,
        from: Option<Hex>,
        to: Hex,
    },
    ItemGained {
        actor: Symbol,
        stat: Stat,
        card: Symbol,
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
