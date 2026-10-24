//! Exact scalar part of the Python stat pipeline. Effect/aura closure is
//! explicit; unsupported bonuses are never silently replaced by zero.
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
        self.computed_stat_for_card(unit, stat, base, self.performing_card(unit, context)?)
    }
    /// Ordinary turn-card context. Nested callers use computed_stat_in_context.
    pub fn computed_stat(&self, unit: Symbol, stat: Stat, base: i64) -> Result<i64, String> {
        self.computed_stat_for_card(unit, stat, base, None)
    }
    pub fn computed_stat_for_card(
        &self,
        unit: Symbol,
        stat: Stat,
        base: i64,
        performing: Option<CardId>,
    ) -> Result<i64, String> {
        let Some(_entity) = self.entity(unit).filter(|entity| {
            matches!(
                entity.kind,
                EntityKind::Hero | EntityKind::Piece | EntityKind::Minion
            )
        }) else {
            return Ok(base);
        };
        if self.has_effects {
            return Err("active-effect stat closure not compiled".into());
        }
        if self.has_uncompiled_auras {
            return Err("card stat aura closure not compiled".into());
        }
        let Some(hero) = self.hero(unit) else {
            return Ok(base);
        };
        if hero.uncompiled_auras {
            return Err("card stat aura closure not compiled".into());
        }
        if performing.is_some_and(|id| self.card(id).is_none()) {
            return Err("unknown performing card instance".into());
        }
        let marker_unit = if stat == Stat::Initiative {
            hero.id
        } else {
            self.resolved_actor(unit)
        };
        let mut total = base
            .checked_add(hero.items[stat.index()])
            .ok_or("stat exceeds i64")?;
        if matches!(stat, Stat::Attack | Stat::Defense | Stat::Initiative) {
            for marker in &self.markers {
                let Some(target) = marker.target else {
                    continue;
                };
                let shared = target == hero.id
                    || target == marker_unit
                    || hero
                        .pieces
                        .iter()
                        .any(|id| *id == target && self.position(*id).is_some());
                if shared && matches!(marker.kind, MarkerKind::Venom | MarkerKind::Poison) {
                    total = total.checked_add(marker.value).ok_or("stat exceeds i64")?;
                }
            }
        }
        total = total
            .checked_add(self.card_aura_bonus(unit, stat, performing.or(hero.current))?)
            .ok_or("stat exceeds i64")?;
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
    pub fn minion_defense(&self, unit: Symbol) -> Result<i64, String> {
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
        if self.has_effects {
            return Err("effect-aware minion defense not compiled".into());
        }
        let mut total = 0;
        for hex in self.tile_hexes() {
            let distance = position.distance(*hex);
            if distance != 1 && distance != 2 {
                continue;
            }
            let Some(minion) = self
                .occupant(*hex)
                .and_then(|id| self.entity(id))
                .filter(|entity| entity.kind == EntityKind::Minion)
            else {
                continue;
            };
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
