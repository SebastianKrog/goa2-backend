//! Native item, active-effect, marker and supported-card-aura stat pipeline.
//! Unknown card aura hooks remain explicit unsupported closures.
use crate::{
    cards::CardId,
    data::{Datum, Registers, Symbol, Symbols},
    world::{ActionKind, EntityKind, MinionKind, NativeWorld},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Stat {
    Attack,
    Defense,
    Movement,
    Initiative,
    Range,
    Radius,
}
impl Stat {
    pub const ALL: [Self; 6] = [
        Self::Attack,
        Self::Defense,
        Self::Movement,
        Self::Initiative,
        Self::Range,
        Self::Radius,
    ];
    pub fn name(self) -> &'static str {
        [
            "ATTACK",
            "DEFENSE",
            "MOVEMENT",
            "INITIATIVE",
            "RANGE",
            "RADIUS",
        ][self.index()]
    }
    pub fn index(self) -> usize {
        match self {
            Self::Attack => 0,
            Self::Defense => 1,
            Self::Movement => 2,
            Self::Initiative => 3,
            Self::Range => 4,
            Self::Radius => 5,
        }
    }
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "ATTACK" => Ok(Self::Attack),
            "DEFENSE" => Ok(Self::Defense),
            "MOVEMENT" => Ok(Self::Movement),
            "INITIATIVE" => Ok(Self::Initiative),
            "RANGE" => Ok(Self::Range),
            "RADIUS" => Ok(Self::Radius),
            _ => Err("unknown stat".into()),
        }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkerKind {
    Venom,
    Poison,
    Bounty,
}
#[derive(Clone, Debug)]
pub struct NativeMarker {
    pub key: Symbol,
    pub source: Option<Symbol>,
    pub kind: MarkerKind,
    pub target: Option<Symbol>,
    pub value: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CardStats {
    pub primary_value: i64,
    pub range: i64,
    pub radius: Option<i64>,
}
#[derive(Clone, Debug)]
pub(crate) struct PerformingCardKeys {
    card: Symbol,
    owner: Symbol,
    empty: Symbol,
}
impl PerformingCardKeys {
    pub(crate) fn compile(s: &mut Symbols) -> Self {
        Self {
            card: s.intern("performing_card_id"),
            owner: s.intern("performing_card_owner_id"),
            empty: s.intern(""),
        }
    }
    fn text(&self, value: Option<&Datum>) -> Result<Option<Symbol>, String> {
        match value {
            None | Some(Datum::Null | Datum::Bool(false) | Datum::Int(0)) => Ok(None),
            Some(Datum::Text(id)) if *id == self.empty => Ok(None),
            Some(Datum::Text(id)) => Ok(Some(*id)),
            Some(Datum::Float(v)) if *v == 0.0 => Ok(None),
            Some(Datum::List(v)) if v.is_empty() => Ok(None),
            Some(Datum::Object(v)) if v.is_empty() => Ok(None),
            _ => Err("non-string performing-card identity coercion not compiled".into()),
        }
    }
}
impl NativeWorld {
    /// Resolve the current nested action from live registers, never from a cache.
    pub fn performing_card(
        &self,
        unit: Symbol,
        context: &Registers,
    ) -> Result<Option<CardId>, String> {
        let Some(hero) = self.hero(unit) else {
            return Ok(None);
        };
        let keys = &self.definitions.stat_context;
        if let Some(printed) = keys.text(context.get(keys.card))? {
            let owner = keys.text(context.get(keys.owner))?;
            if let Some(card) = owner
                .and_then(|o| self.card_for_hero_printed(o, printed))
                .or_else(|| self.card_by_printed(printed))
            {
                return Ok(Some(card));
            }
        }
        Ok(hero.current)
    }
    pub fn computed_stat_in_context(
        &self,
        unit: Symbol,
        stat: Stat,
        base: i64,
        context: &Registers,
    ) -> Result<i64, String> {
        self.computed_stat_inner(unit, stat, base, None, self.performing_card(unit, context)?)
    }
    /// Ordinary turn-card context. Nested callers use computed_stat_in_context.
    pub fn computed_stat(&self, unit: Symbol, stat: Stat, base: i64) -> Result<i64, String> {
        self.computed_stat_for_card(unit, stat, base, None)
    }
    pub(crate) fn computed_stat_for_card_in_context(
        &self,
        unit: Symbol,
        stat: Stat,
        base: i64,
        performing: Option<CardId>,
        context: &Registers,
    ) -> Result<i64, String> {
        self.computed_stat_inner(
            unit,
            stat,
            base,
            performing,
            self.performing_card(unit, context)?,
        )
    }
    pub fn computed_stat_for_card(
        &self,
        unit: Symbol,
        stat: Stat,
        base: i64,
        performing: Option<CardId>,
    ) -> Result<i64, String> {
        self.computed_stat_inner(
            unit,
            stat,
            base,
            performing,
            self.hero(unit).and_then(|h| h.current),
        )
    }
    fn computed_stat_inner(
        &self,
        unit: Symbol,
        stat: Stat,
        base: i64,
        performing: Option<CardId>,
        fallback: Option<CardId>,
    ) -> Result<i64, String> {
        use crate::effect_rules::Kind;
        let Some(entity) = self.entity(unit).filter(|e| {
            matches!(
                e.kind,
                EntityKind::Hero | EntityKind::Piece | EntityKind::Minion
            )
        }) else {
            return Ok(base);
        };
        // Reference owner/piece recursion drops an explicit performed card.
        if stat == Stat::Initiative && entity.kind == EntityKind::Piece {
            return self.computed_stat_inner(
                entity.owner.ok_or("piece owner missing")?,
                stat,
                base,
                None,
                fallback,
            );
        }
        if entity.multi_piece && stat != Stat::Initiative && self.resolved_actor(unit) != unit {
            return self.computed_stat_inner(self.resolved_actor(unit), stat, base, None, fallback);
        }
        if self.has_uncompiled_auras || self.hero(unit).is_some_and(|h| h.uncompiled_auras) {
            return Err("card stat aura closure not compiled".into());
        }
        if performing.is_some_and(|id| self.card(id).is_none()) {
            return Err("unknown performing card instance".into());
        }
        let hero = self.hero(unit);
        let scope_ids = if entity.multi_piece {
            let pieces = self.piece_ids(unit).collect::<Vec<_>>();
            if pieces.is_empty() {
                vec![unit]
            } else {
                pieces
            }
        } else {
            vec![unit]
        };
        let mut total = base;
        if let Some(hero) = hero {
            let mut item = hero.items[stat.index()];
            if item > 0
                && self.effects.instances.iter().any(|e| {
                    e.definition.behavior.kind == Kind::DoubleItems
                        && e.source == hero.id
                        && self.effect_active(e)
                })
            {
                item = item.checked_mul(2).ok_or("item bonus exceeds i64")?;
            }
            total = total.checked_add(item).ok_or("stat exceeds i64")?;
        }
        let mut conditional = Vec::new();
        for e in &self.effects.instances {
            let b = &e.definition.behavior;
            if b.kind != Kind::AreaStat || b.stat != Some(stat) || !self.effect_active(e) {
                continue;
            }
            let mut covered = false;
            for id in &scope_ids {
                if !self.effect_ignored(e, *id)? && self.effect_in_scope(e, *id, false)? {
                    covered = true;
                    break;
                }
            }
            if !covered {
                continue;
            }
            if let Some(minimum) = b.minimum {
                conditional.push((b.value, minimum));
            } else {
                total = total.checked_add(b.value).ok_or("stat exceeds i64")?;
            }
        }
        let performing = performing.or(fallback);
        if hero.is_some()
            && matches!(
                stat,
                Stat::Movement | Stat::Initiative | Stat::Attack | Stat::Defense
            )
            && performing
                .and_then(|id| self.card(id))
                .is_some_and(|c| self.definitions.cards[c.definition].basic)
        {
            for e in &self.effects.instances {
                if e.definition.behavior.kind != Kind::BasicBonus || !self.effect_active(e) {
                    continue;
                }
                for id in &scope_ids {
                    if self.effect_in_scope(e, *id, false)? {
                        total = total
                            .checked_add(e.definition.behavior.value)
                            .ok_or("stat exceeds i64")?;
                        break;
                    }
                }
            }
        }
        if let Some(hero) = hero {
            total = total
                .checked_add(self.card_aura_bonus(unit, stat, performing)?)
                .ok_or("stat exceeds i64")?;
            if matches!(stat, Stat::Attack | Stat::Defense | Stat::Initiative) {
                for marker in &self.markers {
                    let shared = marker.target.is_some_and(|target| {
                        target == hero.id
                            || target == unit
                            || hero
                                .pieces
                                .iter()
                                .any(|id| *id == target && self.position(*id).is_some())
                    });
                    if shared && matches!(marker.kind, MarkerKind::Venom | MarkerKind::Poison) {
                        total = total.checked_add(marker.value).ok_or("stat exceeds i64")?;
                    }
                }
            }
        }
        for (value, minimum) in conditional {
            let candidate = total
                .checked_add(value)
                .ok_or("conditional stat exceeds i64")?;
            if candidate >= minimum {
                total = candidate;
            }
        }
        Ok(total)
    }
    pub fn card_stats(&self, unit: Symbol, id: CardId) -> Result<CardStats, String> {
        let card = self.card(id).ok_or("unknown physical card")?;
        let definition = &self.definitions.cards[card.definition];
        let primary = if card.facedown {
            None
        } else {
            definition.primary
        };
        let stat = match primary {
            Some(ActionKind::Attack) => Some(Stat::Attack),
            Some(ActionKind::Movement) => Some(Stat::Movement),
            Some(ActionKind::Defense | ActionKind::DefenseSkill) => Some(Stat::Defense),
            _ => None,
        };
        let primary_value = if let Some(stat) = stat {
            self.computed_stat_for_card(
                unit,
                stat,
                definition.primary_value.unwrap_or(0),
                Some(id),
            )?
        } else {
            0
        };
        let range = if definition.ranged {
            if let Some(base) = definition.range {
                self.computed_stat_for_card(unit, Stat::Range, base, Some(id))?
            } else {
                1
            }
        } else {
            1
        };
        let radius = definition
            .radius
            .map(|base| self.computed_stat_for_card(unit, Stat::Radius, base, Some(id)))
            .transpose()?;
        Ok(CardStats {
            primary_value,
            range,
            radius,
        })
    }
    pub fn minion_defense(&self, unit: Symbol, actor: Option<Symbol>) -> Result<i64, String> {
        let Some(target) = self.entity(unit).filter(|entity| {
            matches!(
                entity.kind,
                EntityKind::Hero | EntityKind::Piece | EntityKind::Minion
            )
        }) else {
            return Ok(0);
        };
        let Some(position) = self.position(unit) else {
            return Ok(0);
        };
        let equivalence = self.illusion_minion_team(actor);
        let mut total = 0;
        for hex in self.tile_hexes() {
            let distance = position.distance(*hex);
            if (distance != 1 && distance != 2) || !self.effect_connected(position, *hex, &[])? {
                continue;
            }
            let Some(entity) = self.occupant(*hex).and_then(|id| self.entity(id)) else {
                continue;
            };
            if entity.kind != EntityKind::Minion {
                if distance == 1
                    && entity.kind == EntityKind::Token
                    && entity.token_type == Some(self.definitions.illusion_token)
                    && let Some(team) = equivalence
                {
                    total += if Some(team) == target.team { 1 } else { -1 };
                }
                continue;
            }
            let minion = entity;
            if distance == 1 {
                if minion.team != target.team {
                    total -= 1;
                } else if matches!(
                    minion.minion_kind,
                    Some(MinionKind::Melee | MinionKind::Heavy)
                ) {
                    total += 1;
                }
            } else if minion.team != target.team && minion.minion_kind == Some(MinionKind::Ranged) {
                total -= 1;
            }
        }
        Ok(total)
    }
}
