//! Native defense windows and discard orchestration. Passive provider execution
//! lives in `passives`; remaining effect bodies reject explicitly.
use crate::{
    cards::CardId,
    data::{Datum, RegisterRead, Registers, Symbol, Symbols},
    stats::Stat,
    world::{ActionKind, CardLifecycle, NativeWorld},
};
use serde_json::{Value, json};
#[derive(Clone, Debug, Hash, PartialEq)]
pub struct DefenseCard {
    pub physical: CardId,
    pub id: Symbol,
    pub name: Symbol,
    pub base: i64,
    pub total: i64,
    pub primary: bool,
    pub color: Option<Symbol>,
}
#[derive(Clone, Debug, Hash, PartialEq)]
pub struct DefenseMenu {
    pub owner: Symbol,
    pub target: Symbol,
    pub cards: Vec<DefenseCard>,
    pub attack: Option<Datum>,
    pub modifier: i64,
    pub needed: Option<Datum>,
    pub initiative: bool,
}
impl DefenseMenu {
    pub fn to_json(&self, id: String, symbols: &Symbols) -> Value {
        let owner = symbols.name(self.owner);
        let label = if self.initiative { "Init" } else { "Def" };
        let prompt = if let Some(attack) = &self.attack {
            format!(
                "Player {owner}, select a Defense card. Attack: {}, Defense needed: {} (minion mod: {:+})",
                attack.python_scalar_text(symbols).unwrap(),
                self.needed
                    .as_ref()
                    .unwrap()
                    .python_scalar_text(symbols)
                    .unwrap(),
                self.modifier
            )
        } else {
            format!("Player {owner}, select a Defense card.")
        };
        let mut options=self.cards.iter().map(|c|json!({"id":symbols.name(c.id),"text":format!("{} ({label}: {})",symbols.name(c.name),c.total),"metadata":{"defense_value":c.total,"base_defense":c.base}})).collect::<Vec<_>>();
        options.push(json!({"id":"PASS","text":"PASS","metadata":{}}));
        let facts:serde_json::Map<String,Value>=self.cards.iter().map(|c|(symbols.name(c.id).into(),json!({"action_role":null,"defense_stat":if self.initiative{"INITIATIVE"}else{"DEFENSE"},"defense_kind":if c.primary{"primary"}else{"secondary"}}))).collect();
        json!({"id":id,"request_type":"SELECT_CARD_OR_PASS","player_id":owner,"prompt":prompt,"options":options,"can_skip":false,"can_rollback":false,"context":{"attack_value":self.attack.as_ref().map(|v|v.to_json(symbols)),"minion_modifier":self.modifier,"defense_needed":self.needed.as_ref().map(|v|v.to_json(symbols))},"decision_context":{"purpose":"defense","actor_id":owner,"source_card_id":null,"choice_facts":facts}})
    }
}
#[derive(Clone, Debug)]
pub enum DefenseOp {
    Flag { key: Symbol, value: Datum },
    PlaceBounty { target: Symbol },
    RemoveBounty,
    ProjectileTarget { deflect: bool, range: i64 },
    ProjectileDiscard { deflect: bool },
}
#[derive(Clone, Debug)]
pub struct DefensePlan {
    pub card: Symbol,
    pub operations: Vec<DefenseOp>,
}
pub enum Window {
    Done,
    Selected { owner: Symbol, card: Symbol },
    Wait(DefenseMenu),
}
#[derive(Clone, Debug)]
pub struct ReactionRules {
    reset: Vec<Symbol>,
    defense: Symbol,
    card: Symbol,
    color: Symbol,
    defender: Symbol,
    primary: Symbol,
    block_primary: Symbol,
    initiative: Symbol,
    attack: Symbol,
    modifier: Symbol,
    pass: Symbol,
    action_stack: Symbol,
    current: Symbol,
    defense_action: Symbol,
    discarded: Symbol,
    discard_owner: Symbol,
    source: Symbol,
    hand: Symbol,
    played: Symbol,
    current_turn: Symbol,
    registered: Vec<Symbol>,
    custom_on_block: Vec<Symbol>,
    bounty: Symbol,
    ranged: Symbol,
    auto: Symbol,
    invalid: Symbol,
    ignore: Symbol,
    block: Symbol,
}
impl ReactionRules {
    pub fn registered_effect(&self, effect: Symbol) -> bool {
        self.registered.contains(&effect)
    }
    pub fn compile(symbols: &mut Symbols) -> Result<Self, String> {
        let source = crate::templates::instructions()?;
        let registered = source["registered_effects"]
            .as_array()
            .ok_or("missing effect registry")?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(|s| symbols.intern(s))
                    .ok_or("invalid effect identity".into())
            })
            .collect::<Result<Vec<_>, String>>()?;
        let custom_on_block = source["custom_on_block_effects"]
            .as_array()
            .ok_or("missing on-block capabilities")?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(|s| symbols.intern(s))
                    .ok_or("invalid on-block identity".into())
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            custom_on_block,
            bounty: symbols.intern("bounty"),
            ranged: symbols.intern("attack_is_ranged"),
            auto: symbols.intern("auto_block"),
            invalid: symbols.intern("defense_invalid"),
            ignore: symbols.intern("ignore_minion_defense"),
            registered,
            reset: [
                "auto_block",
                "defense_invalid",
                "defense_bonus",
                "ignore_minion_defense",
                "snorri_ult_rune_defense",
            ]
            .map(|s| symbols.intern(s))
            .to_vec(),
            defense: symbols.intern("defense_value"),
            card: symbols.intern("defense_card_id"),
            color: symbols.intern("defense_card_color"),
            defender: symbols.intern("defender_id"),
            primary: symbols.intern("is_primary_defense"),
            block_primary: symbols.intern("block_primary_defense"),
            initiative: symbols.intern("defense_uses_initiative"),
            attack: symbols.intern("attack_damage"),
            modifier: symbols.intern("minion_defense_modifier"),
            pass: symbols.intern("PASS"),
            action_stack: symbols.intern("action_type_stack"),
            current: symbols.intern("current_action_type"),
            defense_action: symbols.intern("DEFENSE"),
            discarded: symbols.intern("discarded_card_id"),
            discard_owner: symbols.intern("discarded_card_owner_id"),
            source: symbols.intern("discard_source"),
            hand: symbols.intern("HAND"),
            played: symbols.intern("PLAYED"),
            current_turn: symbols.intern("current_turn"),
            block: symbols.intern("block_succeeded"),
        })
    }
    fn pass(&self, ctx: &mut Registers, target: Symbol) {
        ctx.set(self.defense, Datum::Null);
        ctx.set(self.card, Datum::Null);
        ctx.set(self.color, Datum::Null);
        ctx.set(self.defender, Datum::Text(target));
        ctx.set(self.primary, Datum::Bool(false));
    }
    pub fn window(
        &self,
        world: &NativeWorld,
        ctx: &mut Registers,
        symbols: &Symbols,
        target: Symbol,
        actor: Option<Symbol>,
        selected: Option<&Datum>,
    ) -> Result<Window, String> {
        for key in &self.reset {
            ctx.remove(*key);
        }
        let Some(target) = text(ctx.get(target), symbols)? else {
            return Ok(Window::Done);
        };
        let Some(hero) = world.hero(target) else {
            self.pass(ctx, target);
            return Ok(Window::Done);
        };
        let (cards, initiative) = self.cards(world, ctx, symbols, target)?;
        if let Some(Datum::Text(id)) = selected {
            if *id == self.pass {
                self.pass(ctx, target);
                return Ok(Window::Done);
            }
            if let Some(card) = cards.iter().find(|c| c.id == *id) {
                ctx.set(self.defense, Datum::Int(card.total));
                ctx.set(self.card, Datum::Text(card.id));
                ctx.set(self.color, card.color.map_or(Datum::Null, Datum::Text));
                ctx.set(self.defender, Datum::Text(target));
                ctx.set(self.primary, Datum::Bool(card.primary));
                let mut stack = match ctx.get(self.action_stack) {
                    None => Vec::new(),
                    Some(Datum::List(v)) => v.clone(),
                    _ => return Err("invalid action type stack".into()),
                };
                if let Some(current) = ctx.get(self.current).filter(|v| v.truthy(symbols)) {
                    stack.push(current.clone());
                }
                ctx.set(self.action_stack, Datum::List(stack));
                ctx.set(self.current, Datum::Text(self.defense_action));
                return Ok(Window::Selected {
                    owner: hero.id,
                    card: card.id,
                });
            }
        }
        let menu = self.menu(world, ctx, target, actor, cards, initiative)?;
        ctx.set(self.modifier, Datum::Int(menu.modifier));
        Ok(Window::Wait(menu))
    }
    /// Shared ordered card/stat calculation. Callers own context/list mutation.
    pub fn cards(
        &self,
        world: &NativeWorld,
        ctx: &dyn RegisterRead,
        symbols: &Symbols,
        target: Symbol,
    ) -> Result<(Vec<DefenseCard>, bool), String> {
        let hero = world.hero(target).ok_or("reaction hero missing")?;
        let truth = |key| ctx.get(key).is_some_and(|v| v.truthy(symbols));
        let block_primary = truth(self.block_primary);
        let initiative = truth(self.initiative);
        let stat = if initiative {
            Stat::Initiative
        } else {
            Stat::Defense
        };
        let eligible = |id: CardId| -> Result<bool, String> {
            let card = world.card(id).ok_or("reaction card missing")?;
            if card.facedown {
                return Ok(false);
            }
            let def = &world.definitions.cards[card.definition];
            let primary = matches!(
                def.primary,
                Some(ActionKind::Defense | ActionKind::DefenseSkill)
            );
            let secondary = def
                .secondary
                .iter()
                .any(|(action, _)| *action == ActionKind::Defense);
            Ok((primary || secondary) && (!block_primary || !primary || secondary))
        };
        let mut candidates = Vec::new();
        for id in &hero.hand {
            if eligible(*id)? {
                candidates.push(*id);
            }
        }
        for id in world.active_shield_cards(hero.id) {
            if eligible(id)? {
                let mut duplicate = false;
                for existing in &candidates {
                    if world.card_model_equal(*existing, id)? {
                        duplicate = true;
                        break;
                    }
                }
                if !duplicate {
                    candidates.push(id);
                }
            }
        }
        // Source deduplicates shields before computing option values. Avoid
        // recomputing the same model's stats for duplicate shield references.
        let mut cards = Vec::with_capacity(candidates.len());
        for id in candidates {
            let card = world.card(id).ok_or("reaction card missing")?;
            let def = &world.definitions.cards[card.definition];
            let primary = matches!(
                def.primary,
                Some(ActionKind::Defense | ActionKind::DefenseSkill)
            );
            let secondary = def
                .secondary
                .iter()
                .find(|(action, _)| *action == ActionKind::Defense)
                .map(|(_, value)| *value);
            let base = if initiative {
                card.initiative_override.unwrap_or(def.initiative)
            } else if primary {
                def.primary_value.unwrap_or(0)
            } else {
                secondary.unwrap_or(0)
            };
            let total =
                world.computed_stat_for_card_in_context(target, stat, base, Some(id), ctx)?;
            cards.push(DefenseCard {
                physical: id,
                id: def.printed_id,
                name: def.name,
                base,
                total,
                primary: primary && !block_primary,
                color: def.color,
            });
        }
        Ok((cards, initiative))
    }
    pub fn menu(
        &self,
        world: &NativeWorld,
        ctx: &dyn RegisterRead,
        target: Symbol,
        actor: Option<Symbol>,
        cards: Vec<DefenseCard>,
        initiative: bool,
    ) -> Result<DefenseMenu, String> {
        let modifier = world.minion_defense(target, actor)?;
        let attack = match ctx.get(self.attack) {
            None | Some(Datum::Null) => None,
            Some(value @ (Datum::Int(_) | Datum::Bool(_) | Datum::Float(_))) => Some(value.clone()),
            _ => return Err("nonnumeric attack context not compiled".into()),
        };
        let needed = attack
            .as_ref()
            .map(|value| match value {
                Datum::Float(value) => {
                    let result = *value - modifier as f64;
                    if !result.is_finite() {
                        return Err("nonfinite defense requirement not compiled".into());
                    }
                    Ok(Datum::Float(result))
                }
                Datum::Int(value) => value
                    .checked_sub(modifier)
                    .map(Datum::Int)
                    .ok_or_else(|| "defense requirement exceeds i64".to_owned()),
                Datum::Bool(value) => i64::from(*value)
                    .checked_sub(modifier)
                    .map(Datum::Int)
                    .ok_or_else(|| "defense requirement exceeds i64".to_owned()),
                _ => unreachable!(),
            })
            .transpose()?;
        Ok(DefenseMenu {
            owner: world.hero(target).ok_or("reaction owner missing")?.id,
            target,
            cards,
            attack,
            modifier,
            needed,
            initiative,
        })
    }
    pub fn discard(
        &self,
        world: &mut NativeWorld,
        ctx: &mut Registers,
        hero: Symbol,
        printed: Symbol,
        played_first: bool,
    ) -> Result<bool, String> {
        let Some(h) = world.hero(hero) else {
            return Ok(false);
        };
        let owner = h.id;
        let matches = |id: &CardId| {
            world
                .card(*id)
                .is_some_and(|c| world.definitions.cards[c.definition].printed_id == printed)
        };
        let hand = h.hand.iter().copied().find(matches);
        let played = h
            .played
            .iter()
            .flatten()
            .chain(h.current.iter())
            .copied()
            .find(matches);
        let found = if played_first {
            played.map(|c| (c, false)).or(hand.map(|c| (c, true)))
        } else {
            hand.map(|c| (c, true)).or(played.map(|c| (c, false)))
        };
        let Some((id, hand)) = found else {
            return Ok(false);
        };
        let source = if hand {
            self.hand
        } else if world.card(id).unwrap().lifecycle == CardLifecycle::Resolved {
            self.played
        } else {
            self.current_turn
        };
        world
            .discard_card(owner, id, hand)
            .map_err(|e| e.to_string())?;
        world
            .expire_card_effects(printed)
            .map_err(|e| e.to_string())?;
        world.history.discard(owner, printed);
        ctx.set(self.discarded, Datum::Text(printed));
        ctx.set(self.discard_owner, Datum::Text(owner));
        ctx.set(self.source, Datum::Text(source));
        Ok(true)
    }
    pub fn defense_text(
        &self,
        world: &NativeWorld,
        ctx: &Registers,
        symbols: &Symbols,
        on_block: bool,
    ) -> Result<Option<DefensePlan>, String> {
        if on_block && !ctx.get(self.block).is_some_and(|v| v.truthy(symbols)) {
            return Ok(None);
        }
        if !ctx.get(self.primary).is_some_and(|v| v.truthy(symbols)) {
            return Ok(None);
        }
        let (Some(card), Some(defender)) = (
            text(ctx.get(self.card), symbols)?,
            text(ctx.get(self.defender), symbols)?,
        ) else {
            return Ok(None);
        };
        let Some(hero) = world.hero(defender) else {
            return Ok(None);
        };
        if !on_block && world.position(defender).is_none() && !world.has_board_presence(defender) {
            return Ok(None);
        }
        let Some(id) = hero.hand.iter().chain(&hero.discard).copied().find(|id| {
            world
                .card(*id)
                .is_some_and(|c| world.definitions.cards[c.definition].printed_id == card)
        }) else {
            return Ok(None);
        };
        let instance = world.card(id).ok_or("defense card missing")?;
        let def = &world.definitions.cards[instance.definition];
        if instance.facedown || !def.has_effect || !self.registered.contains(&def.effect) {
            return Ok(None);
        }
        // Python computes these before invoking either effect hook, even when
        // that hook's default implementation returns no steps.
        let stats = world.card_stats(defender, id)?;
        if on_block {
            if matches!(
                symbols.name(def.effect),
                "reflect_projectiles" | "deflect_projectiles"
            ) {
                let deflect = symbols.name(def.effect) == "deflect_projectiles";
                return Ok(Some(DefensePlan {
                    card,
                    operations: vec![
                        DefenseOp::ProjectileTarget {
                            deflect,
                            range: if stats.range == 0 { 3 } else { stats.range },
                        },
                        DefenseOp::ProjectileDiscard { deflect },
                    ],
                }));
            }
            return if self.custom_on_block.contains(&def.effect) {
                Err("on-block effect execution not compiled".into())
            } else {
                Ok(None)
            };
        }
        let flag = |key| DefenseOp::Flag {
            key,
            value: Datum::Bool(true),
        };
        let operations = match symbols.name(def.effect) {
            "aspiring_duelist" => vec![flag(self.ignore)],
            "stop_projectiles" | "reflect_projectiles" | "deflect_projectiles" => vec![flag(
                if ctx.get(self.ranged).is_some_and(|v| v.truthy(symbols)) {
                    self.auto
                } else {
                    self.invalid
                },
            )],
            "perfect_getaway" | "close_call" | "narrow_escape" => {
                if world
                    .markers
                    .iter()
                    .any(|m| m.key == self.bounty && m.target.is_some())
                {
                    let mut ops = vec![flag(self.auto)];
                    match symbols.name(def.effect) {
                        "close_call" => ops.push(DefenseOp::PlaceBounty { target: hero.id }),
                        "narrow_escape" => ops.push(DefenseOp::RemoveBounty),
                        _ => {}
                    }
                    ops
                } else {
                    vec![flag(self.invalid)]
                }
            }
            _ => return Err("primary defense effect execution not compiled".into()),
        };
        Ok(Some(DefensePlan { card, operations }))
    }
    pub fn restore(&self, ctx: &mut Registers) -> Result<(), String> {
        let Some(value) = ctx.get(self.action_stack) else {
            return Ok(());
        };
        let Datum::List(mut stack) = value.clone() else {
            return Err("invalid action type stack".into());
        };
        if let Some(previous) = stack.pop() {
            ctx.set(self.action_stack, Datum::List(stack));
            ctx.set(self.current, previous);
        }
        Ok(())
    }
}
fn text(value: Option<&Datum>, symbols: &Symbols) -> Result<Option<Symbol>, String> {
    match value.filter(|v| v.truthy(symbols)) {
        None => Ok(None),
        Some(Datum::Text(id)) => Ok(Some(*id)),
        _ => Err("non-string reaction identity coercion not compiled".into()),
    }
}
