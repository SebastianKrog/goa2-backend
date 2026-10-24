//! Attack expansion inputs and turn-scoped defeat tracking. Child execution is
//! typed in the native interpreter; this module never generates JSON steps.
use crate::{
    cards::CardId,
    data::{Datum, Registers, Symbol, Symbols},
    stats::Stat,
    world::NativeWorld,
};
#[derive(Clone, Debug)]
pub struct AttackSpec {
    pub damage: i64,
    pub range: i64,
    pub ranged: bool,
    pub card_ranged: bool,
    pub initiative: bool,
    pub target: Option<Symbol>,
    pub output: Option<Symbol>,
    pub damage_bonus: Option<Symbol>,
    pub range_bonus: Option<Symbol>,
    pub damage_stat: Option<Stat>,
    pub range_stat: Option<Stat>,
}
#[derive(Clone, Copy, Debug)]
pub struct AttackBinding {
    pub key: Symbol,
    pub damage: i64,
    pub range: i64,
}
#[derive(Clone, Copy, Debug)]
pub struct AttackExpansion {
    pub binding: AttackBinding,
    pub select: bool,
}
#[derive(Clone, Debug)]
pub struct AttackRules {
    marks: Symbol,
    log: Symbol,
    default_target: Symbol,
    ranged: Symbol,
    attacker: Symbol,
    damage: Symbol,
    initiative: Symbol,
    basic: Symbol,
    empty: Symbol,
}
impl AttackRules {
    pub fn compile(s: &mut Symbols) -> Self {
        Self {
            marks: s.intern("defeat_marks"),
            log: s.intern("defeat_log"),
            default_target: s.intern("victim_id"),
            ranged: s.intern("attack_is_ranged"),
            attacker: s.intern("attacker_id"),
            damage: s.intern("attack_damage"),
            initiative: s.intern("defense_uses_initiative"),
            basic: s.intern("attack_is_basic"),
            empty: s.intern(""),
        }
    }
    pub fn prepare(
        &self,
        world: &NativeWorld,
        ctx: &mut Registers,
        symbols: &Symbols,
        actor: Option<Symbol>,
        spec: &AttackSpec,
        followup: Option<Symbol>,
    ) -> Result<Option<AttackExpansion>, String> {
        let target = followup.or(spec.target);
        let (key, select) = if let Some(key) =
            target.filter(|key| ctx.get(*key).is_some_and(|v| v.truthy(symbols)))
        {
            (key, false)
        } else if let Some(output) = spec.output.filter(|_| followup.is_none()) {
            (output, true)
        } else if target.is_some() {
            return Ok(None);
        } else {
            (self.default_target, true)
        };
        let mark = match ctx.get(self.log) {
            None => 0,
            Some(Datum::List(log)) => log.len(),
            _ => return Err("defeat log is not a list".into()),
        };
        let mut marks = match ctx.get(self.marks) {
            None => Vec::new(),
            Some(Datum::Object(items)) => items.clone(),
            _ => return Err("defeat marks are not a dictionary".into()),
        };
        let value = Datum::Int(i64::try_from(mark).map_err(|_| "defeat mark outside i64")?);
        if let Some((_, v)) = marks.iter_mut().find(|(k, _)| *k == key) {
            *v = value;
        } else {
            marks.push((key, value));
        }
        ctx.set(self.marks, Datum::Object(marks));
        let mut damage = spec.damage;
        let mut range = spec.range;
        if let Some(actor) = actor {
            let board = world.resolved_actor(actor);
            if let Some(stat) = spec.damage_stat {
                damage = world.computed_stat_in_context(board, stat, damage, ctx)?;
            }
            if let Some(stat) = spec.range_stat {
                range = world.computed_stat_in_context(board, stat, range, ctx)?;
            }
        }
        let bonus = |key: Option<Symbol>| -> Result<i64, String> {
            key.map_or(Ok(0), |key| {
                ctx.get(key)
                    .unwrap_or(&Datum::Int(0))
                    .to_integer(symbols)?
                    .ok_or("invalid attack bonus".into())
            })
        };
        damage = damage
            .checked_add(bonus(spec.damage_bonus)?)
            .ok_or("attack damage outside i64")?;
        range = range
            .checked_add(bonus(spec.range_bonus)?)
            .ok_or("attack range outside i64")?;
        let basic = if let Some(actor) = actor.filter(|a| world.hero(*a).is_some()) {
            let card = world.performing_card(actor, ctx)?;
            card.and_then(|id| world.card(id))
                .is_some_and(|c| world.definitions.cards[c.definition].basic)
        } else {
            false
        };
        ctx.set(self.ranged, Datum::Bool(spec.ranged));
        ctx.set(self.attacker, actor.map_or(Datum::Null, Datum::Text));
        ctx.set(self.damage, Datum::Int(damage));
        ctx.set(self.initiative, Datum::Bool(spec.initiative));
        ctx.set(self.basic, Datum::Bool(basic));
        Ok(Some(AttackExpansion {
            binding: AttackBinding { key, damage, range },
            select,
        }))
    }
    pub fn defeated(
        &self,
        world: &NativeWorld,
        ctx: &Registers,
        symbols: &Symbols,
        actor: Option<Symbol>,
        target: Symbol,
        by_actor: bool,
    ) -> Result<bool, String> {
        let Some(value) = ctx.get(target).filter(|v| v.truthy(symbols)) else {
            return Ok(false);
        };
        let marks = match ctx.get(self.marks) {
            None => return Ok(false),
            Some(Datum::Object(items)) => items,
            _ => return Err("defeat marks are not a dictionary".into()),
        };
        let Some((_, mark)) = marks.iter().find(|(k, _)| *k == target) else {
            return Ok(false);
        };
        if matches!(mark, Datum::Null) {
            return Ok(false);
        }
        let mark = match mark {
            Datum::Int(n) => *n,
            Datum::Bool(b) => i64::from(*b),
            _ => return Err("invalid defeat log slice".into()),
        };
        let log = match ctx.get(self.log) {
            None => return Ok(false),
            Some(Datum::List(log)) => log,
            _ => return Err("defeat log is not a list".into()),
        };
        let start = if mark < 0 {
            log.len().saturating_sub(mark.unsigned_abs() as usize)
        } else {
            usize::try_from(mark).unwrap_or(usize::MAX).min(log.len())
        };
        let owner = |id| world.hero(id).map_or(id, |h| h.id);
        let actor = owner(actor.filter(|a| *a != self.empty).unwrap_or(self.empty));
        for entry in &log[start..] {
            let Datum::List(pair) = entry else {
                return Err("invalid defeat log entry".into());
            };
            if pair.len() != 2 {
                return Err("invalid defeat log pair".into());
            }
            if &pair[0] != value {
                continue;
            }
            if by_actor {
                let Some(killer) = text(Some(&pair[1]), symbols)? else {
                    continue;
                };
                if owner(killer) != actor {
                    continue;
                }
            }
            return Ok(true);
        }
        Ok(false)
    }
}
fn text(value: Option<&Datum>, symbols: &Symbols) -> Result<Option<Symbol>, String> {
    match value.filter(|v| v.truthy(symbols)) {
        None => Ok(None),
        Some(Datum::Text(id)) => Ok(Some(*id)),
        _ => Err("non-string attack identity coercion not compiled".into()),
    }
}
impl NativeWorld {
    pub fn card_for_hero_printed(&self, hero: Symbol, printed: Symbol) -> Option<CardId> {
        let h = self.hero(hero)?;
        h.current
            .iter()
            .chain(h.extra.iter())
            .chain(h.played.iter().flatten())
            .chain(&h.hand)
            .chain(&h.deck)
            .chain(&h.discard)
            .chain(&h.spells)
            .chain(h.ultimate.iter())
            .copied()
            .find(|id| {
                self.card(*id)
                    .is_some_and(|c| self.definitions.cards[c.definition].printed_id == printed)
            })
    }
    pub fn card_by_printed(&self, printed: Symbol) -> Option<CardId> {
        self.heroes.iter().find_map(|h| {
            h.current
                .iter()
                .chain(h.played.iter().flatten())
                .chain(&h.hand)
                .chain(&h.deck)
                .chain(&h.discard)
                .chain(&h.spells)
                .chain(h.ultimate.iter())
                .copied()
                .find(|id| {
                    self.card(*id)
                        .is_some_and(|c| self.definitions.cards[c.definition].printed_id == printed)
                })
        })
    }
}
