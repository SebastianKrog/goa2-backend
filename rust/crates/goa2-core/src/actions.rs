//! Card action menus and complete ordered action orchestration recipes.
//! This layer does not run card effects or claim a complete action closure.
use crate::{
    data::{Datum, Registers, Symbol, Symbols},
    stats::Stat,
    world::{ActionKind, NativeWorld},
};
#[derive(Clone, Copy, Debug, Hash, PartialEq)]
pub struct ActionOption {
    pub kind: ActionKind,
    pub value: i64,
    pub display: Option<i64>,
    pub primary_label: bool,
    pub primary_effect: bool,
}
#[derive(Clone, Debug, Hash, PartialEq)]
pub struct ActionMenu {
    pub hero: Symbol,
    pub card: Symbol,
    pub name: Symbol,
    pub options: Vec<ActionOption>,
    pub copied: bool,
}
#[derive(Clone, Copy, Debug, Hash)]
pub enum Passive {
    BeforeAction,
    BeforeAttack,
    BeforeMovement,
    BeforeSkill,
    AfterAttack,
    AfterMovement,
    AfterBasicSkill,
    AfterBasicAction,
    AfterPrimaryAction,
    AfterResolveCard,
}
impl Passive {
    pub fn name(self) -> &'static str {
        match self {
            Self::BeforeAction => "before_action",
            Self::BeforeAttack => "before_attack",
            Self::BeforeMovement => "before_movement",
            Self::BeforeSkill => "before_skill",
            Self::AfterAttack => "after_attack",
            Self::AfterMovement => "after_movement",
            Self::AfterBasicSkill => "after_basic_skill",
            Self::AfterBasicAction => "after_basic_action",
            Self::AfterPrimaryAction => "after_primary_action",
            Self::AfterResolveCard => "after_resolve_card",
        }
    }
}
#[derive(Clone, Debug, Hash)]
pub enum ActionStep {
    ChoosePiece(Symbol),
    Passive(Passive),
    PreMovement(Symbol),
    PreDiscard(Symbol),
    CardText { hero: Symbol, card: Symbol },
    Movement { hero: Symbol, base: i64 },
    FastTravel { hero: Symbol, card: Symbol },
    Attack { damage: i64, range: i64 },
    Clear,
    Hold(Symbol),
    Defend(Symbol),
    PerformLog(Symbol, ActionKind),
    Context(Symbol, Datum),
}
#[derive(Clone, Debug)]
pub enum ActionResolution {
    Prompt(ActionMenu),
    Finished(Vec<ActionStep>),
}
#[derive(Clone, Debug)]
pub struct ActionRules {
    current_card: Symbol,
    current_action: Symbol,
    performing_card: Symbol,
    performing_owner: Symbol,
    attack_effect: Symbol,
    attack_card: Symbol,
    basic_type: Symbol,
    basic_value: Symbol,
    basic_range: Symbol,
    basic_effect: Symbol,
    basic_card: Symbol,
    gold: Symbol,
    silver: Symbol,
    kinds: [Symbol; 8],
}
impl ActionKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Movement => "MOVEMENT",
            Self::Attack => "ATTACK",
            Self::Skill => "SKILL",
            Self::Defense => "DEFENSE",
            Self::DefenseSkill => "DEFENSE_SKILL",
            Self::Hold => "HOLD",
            Self::Clear => "CLEAR",
            Self::FastTravel => "FAST_TRAVEL",
        }
    }
    fn index(self) -> usize {
        match self {
            Self::Movement => 0,
            Self::Attack => 1,
            Self::Skill => 2,
            Self::Defense => 3,
            Self::DefenseSkill => 4,
            Self::Hold => 5,
            Self::Clear => 6,
            Self::FastTravel => 7,
        }
    }
}
impl ActionRules {
    pub fn compile(symbols: &mut Symbols) -> Self {
        Self {
            current_card: symbols.intern("current_card_id"),
            current_action: symbols.intern("current_action_type"),
            performing_card: symbols.intern("performing_card_id"),
            performing_owner: symbols.intern("performing_card_owner_id"),
            attack_effect: symbols.intern("attack_effect_id"),
            attack_card: symbols.intern("attack_card_id"),
            basic_type: symbols.intern("basic_action_type"),
            basic_value: symbols.intern("basic_action_value"),
            basic_range: symbols.intern("basic_action_range"),
            basic_effect: symbols.intern("basic_action_effect_id"),
            basic_card: symbols.intern("basic_action_card_id"),
            gold: symbols.intern("GOLD"),
            silver: symbols.intern("SILVER"),
            kinds: [
                "MOVEMENT",
                "ATTACK",
                "SKILL",
                "DEFENSE",
                "DEFENSE_SKILL",
                "HOLD",
                "CLEAR",
                "FAST_TRAVEL",
            ]
            .map(|s| symbols.intern(s)),
        }
    }
    pub fn resolve(
        &self,
        world: &NativeWorld,
        hero: Symbol,
        selected: Option<&Datum>,
        context: &mut Registers,
    ) -> Result<ActionResolution, String> {
        let Some(h) = world.hero(hero) else {
            return Ok(ActionResolution::Finished(Vec::new()));
        };
        let Some(id) = h.current else {
            return Ok(ActionResolution::Finished(Vec::new()));
        };
        let presence = world.has_board_presence(hero);
        if !presence {
            return Ok(ActionResolution::Finished(Vec::new()));
        }
        self.resolve_for_card(world, hero, id, selected, context, None)
    }
    /// Copy a physical card without changing either hero's turn-card binding.
    pub fn resolve_copied(
        &self,
        world: &NativeWorld,
        hero: Symbol,
        id: crate::cards::CardId,
        selected: Option<&Datum>,
        context: &mut Registers,
        owner_and_suppress: (Symbol, bool),
    ) -> Result<ActionResolution, String> {
        self.resolve_for_card(world, hero, id, selected, context, Some(owner_and_suppress))
    }
    fn resolve_for_card(
        &self,
        world: &NativeWorld,
        hero: Symbol,
        id: crate::cards::CardId,
        selected: Option<&Datum>,
        context: &mut Registers,
        copied: Option<(Symbol, bool)>,
    ) -> Result<ActionResolution, String> {
        let instance = world.card(id).ok_or("missing active card")?;
        let card = &world.definitions.cards[instance.definition];
        if card.spell && copied.is_none() {
            return Err("spell current-face action recipes not compiled".into());
        }
        if world.has_uncompiled_auras {
            return Err("action prevention aura closure not compiled".into());
        }
        if copied.is_none() {
            context.set(self.current_card, Datum::Text(card.printed_id));
        }
        let primary = if instance.facedown {
            None
        } else {
            card.primary
        };
        let normalized = primary.map(|k| {
            if k == ActionKind::DefenseSkill {
                ActionKind::Skill
            } else {
                k
            }
        });
        let legality_context = Registers::default();
        let available = |kind| -> Result<bool, String> {
            Ok(world
                .action_legality(
                    hero,
                    kind,
                    Some(id),
                    if copied.is_some() {
                        &legality_context
                    } else {
                        context
                    },
                )?
                .allowed
                && (copied.is_some()
                    || kind != ActionKind::FastTravel
                    || world.has_safe_travel_zone(hero)))
        };
        let mut options = Vec::new();
        let mut add = |kind: ActionKind, base: Option<i64>, primary_label| -> Result<(), String> {
            if !available(kind)? {
                return Ok(());
            }
            let stat = match kind {
                ActionKind::Movement => Some(Stat::Movement),
                ActionKind::Attack => Some(Stat::Attack),
                ActionKind::Defense | ActionKind::DefenseSkill if copied.is_none() => {
                    Some(Stat::Defense)
                }
                _ => None,
            };
            let value = if let Some(stat) = stat {
                world.computed_stat_in_context(hero, stat, base.unwrap_or(0), context)?
            } else {
                base.unwrap_or(0)
            };
            options.push(ActionOption {
                kind,
                value,
                display: if stat.is_some() { Some(value) } else { base },
                primary_label,
                primary_effect: Some(kind) == normalized,
            });
            Ok(())
        };
        if let Some(kind) = normalized.filter(|k| *k != ActionKind::Defense) {
            add(kind, card.primary_value, true)?;
        }
        for (kind, value) in &card.secondary {
            if *kind == ActionKind::Defense {
                continue;
            }
            if instance.facedown {
                if *kind == ActionKind::Hold {
                    add(*kind, Some(0), false)?;
                }
            } else {
                add(*kind, Some(*value), false)?;
            }
        }
        let chosen = if let Some(Datum::Text(choice)) = selected {
            options
                .iter()
                .find(|o| self.kinds[o.kind.index()] == *choice)
        } else {
            None
        };
        let Some(chosen) = chosen else {
            return Ok(ActionResolution::Prompt(ActionMenu {
                hero,
                card: card.printed_id,
                name: card.name,
                options,
                copied: copied.is_some(),
            }));
        };
        context.set(
            self.current_action,
            Datum::Text(self.kinds[chosen.kind.index()]),
        );
        if let Some((owner, _)) = copied {
            context.set(self.current_card, Datum::Text(card.printed_id));
            context.set(self.performing_card, Datum::Text(card.printed_id));
            context.set(self.performing_owner, Datum::Text(owner));
        }
        let kind = chosen.kind;
        // Execution checks both the raw primary and DEFENSE_SKILL's SKILL alias.
        // Decision facts deliberately retain the source normalized-role test.
        let primary_effect = Some(kind) == primary || Some(kind) == normalized;
        let mut steps = vec![
            ActionStep::ChoosePiece(hero),
            ActionStep::Passive(Passive::BeforeAction),
        ];
        if let Some(trigger) = match kind {
            ActionKind::Attack => Some(Passive::BeforeAttack),
            ActionKind::Movement => Some(Passive::BeforeMovement),
            ActionKind::Skill => Some(Passive::BeforeSkill),
            _ => None,
        } {
            steps.push(ActionStep::Passive(trigger));
        }
        let secondary = |kind| {
            card.secondary
                .iter()
                .find(|(k, _)| *k == kind)
                .map(|(_, v)| *v)
                .unwrap_or(chosen.value)
        };
        let range = card.range.filter(|v| *v != 0).unwrap_or(1);
        if primary_effect {
            steps.extend([
                ActionStep::PreMovement(hero),
                ActionStep::PreDiscard(hero),
                ActionStep::CardText {
                    hero,
                    card: card.printed_id,
                },
            ]);
        } else {
            match kind {
                ActionKind::Movement => steps.push(ActionStep::Movement {
                    hero,
                    base: secondary(kind),
                }),
                ActionKind::FastTravel => steps.push(ActionStep::FastTravel {
                    hero,
                    card: card.printed_id,
                }),
                ActionKind::Attack => steps.push(ActionStep::Attack {
                    damage: secondary(kind),
                    range,
                }),
                ActionKind::Clear => steps.push(ActionStep::Clear),
                ActionKind::Hold if copied.is_none() => steps.push(ActionStep::Hold(hero)),
                ActionKind::Defense if copied.is_none() => steps.push(ActionStep::Defend(hero)),
                _ if copied.is_some() => steps.push(ActionStep::PerformLog(hero, kind)),
                _ => {}
            }
        }
        let effect = primary_effect && !instance.facedown; // Nonempty IDs handled by the compiled definition flag below.
        if kind == ActionKind::Attack {
            if effect && card.has_effect {
                steps.extend([
                    ActionStep::Context(self.attack_effect, Datum::Text(card.effect)),
                    ActionStep::Context(self.attack_card, Datum::Text(card.printed_id)),
                ]);
            }
            steps.push(ActionStep::Passive(Passive::AfterAttack));
        }
        if kind == ActionKind::Movement {
            steps.push(ActionStep::Passive(Passive::AfterMovement));
        }
        if kind == ActionKind::Skill
            && !instance.facedown
            && matches!(card.color,Some(c) if c==self.gold || c==self.silver)
        {
            steps.push(ActionStep::Passive(Passive::AfterBasicSkill));
        }
        if card.basic
            && matches!(
                kind,
                ActionKind::Attack | ActionKind::Movement | ActionKind::Skill
            )
        {
            steps.extend([
                ActionStep::Context(self.basic_type, Datum::Text(self.kinds[kind.index()])),
                ActionStep::Context(self.basic_value, Datum::Int(chosen.value)),
            ]);
            if kind == ActionKind::Attack {
                steps.push(ActionStep::Context(
                    self.basic_range,
                    Datum::Int(world.computed_stat_in_context(
                        hero,
                        Stat::Range,
                        range,
                        context,
                    )?),
                ));
            }
            if effect && card.has_effect {
                steps.push(ActionStep::Context(
                    self.basic_effect,
                    Datum::Text(card.effect),
                ));
            }
            steps.extend([
                ActionStep::Context(self.basic_card, Datum::Text(card.printed_id)),
                ActionStep::Passive(Passive::AfterBasicAction),
            ]);
        }
        if primary_effect {
            steps.push(ActionStep::Passive(Passive::AfterPrimaryAction));
        }
        if !copied.is_some_and(|(_, suppress)| suppress) {
            steps.push(ActionStep::Passive(Passive::AfterResolveCard));
        }
        Ok(ActionResolution::Finished(steps))
    }
}

impl ActionMenu {
    /// Boundary-only request encoding, shared by execution and diagnostics.
    pub fn to_json(&self, id: String, symbols: &Symbols) -> serde_json::Value {
        use serde_json::json;
        let mut facts = serde_json::Map::new();
        let options = self.options.iter().map(|o| {
            let name = o.kind.name();
            facts.entry(name.to_owned()).or_insert_with(|| json!({"action_role":if o.primary_effect {"primary"} else {"secondary"},"defense_stat":null,"defense_kind":null}));
            json!({"id":name,"text":format!("{}: {name} ({})",if o.primary_label {"Primary"} else {"Secondary"},o.display.map_or_else(||"-".into(),|v|v.to_string())),"metadata":{"type":name,"value":o.value}})
        }).collect::<Vec<_>>();
        json!({"id":id,"request_type":"CHOOSE_ACTION","player_id":symbols.name(self.hero),"prompt":if self.copied {format!("Choose an action to perform on {}",symbols.name(self.name))} else {format!("Choose action for card {}",symbols.name(self.name))},"options":options,"can_skip":false,"can_rollback":false,"context":{},"decision_context":{"purpose":"action","actor_id":symbols.name(self.hero),"source_card_id":symbols.name(self.card),"choice_facts":facts}})
    }
}
impl ActionStep {
    pub fn template(&self) -> &'static str {
        match self {
            Self::ChoosePiece(_) => "action_piece",
            Self::Passive(_) => "passive",
            Self::PreMovement(_) => "pre_movement",
            Self::PreDiscard(_) => "pre_discard",
            Self::CardText { .. } => "card_text",
            Self::Movement { .. } => "movement_sequence",
            Self::FastTravel { .. } => "fast_travel",
            Self::Attack { .. } => "attack_sequence",
            Self::Clear => "clear_select",
            Self::Hold(_) | Self::Defend(_) | Self::PerformLog(..) => "log",
            Self::Context(..) => "context",
        }
    }
    /// Patch a frozen continuation only on import/export, never in search execution.
    pub fn patch_wire(&self, wire: &mut serde_json::Value, symbols: &Symbols) {
        use serde_json::json;
        match self {
            Self::ChoosePiece(h) | Self::PreMovement(h) | Self::PreDiscard(h) => {
                wire["hero_id"] = json!(symbols.name(*h))
            }
            Self::Passive(trigger) => wire["trigger"] = json!(trigger.name()),
            Self::CardText { hero, card } => {
                wire["hero_id"] = json!(symbols.name(*hero));
                wire["card_id"] = json!(symbols.name(*card));
            }
            Self::Movement { hero, base } => {
                wire["unit_id"] = json!(symbols.name(*hero));
                wire["range_val"] = json!(base);
                wire["range_stat_type"] = json!("MOVEMENT");
            }
            Self::FastTravel { hero, card } => {
                wire["unit_id"] = json!(symbols.name(*hero));
                wire["source_card_id"] = json!(symbols.name(*card));
            }
            Self::Attack { damage, range } => {
                wire["damage"] = json!(damage);
                wire["range_val"] = json!(range);
                wire["damage_stat_type"] = json!("ATTACK");
                wire["range_stat_type"] = json!("RANGE");
            }
            Self::Hold(h) => wire["message"] = json!(format!("{} Holds.", symbols.name(*h))),
            Self::Defend(h) => {
                wire["message"] = json!(format!("{} Defends (Active).", symbols.name(*h)))
            }
            Self::PerformLog(h, kind) => {
                wire["message"] = json!(format!("{} performs {}.", symbols.name(*h), kind.name()));
            }
            Self::Context(key, value) => {
                wire["key"] = json!(symbols.name(*key));
                wire["value"] = value.to_json(symbols);
            }
            Self::Clear => {}
        }
    }
}

impl NativeWorld {
    pub fn can_repeat_action(&self, actor: Option<Symbol>) -> Result<bool, String> {
        if actor.is_none() {
            return Ok(false);
        }
        self.repeat_legality(actor.unwrap())
            .map(|result| result.allowed)
    }
}
