//! Native item, active-effect, marker and supported-card-aura stat pipeline.
//! Unknown card aura hooks remain explicit unsupported closures.
use crate::{
    cards::CardId,
    data::{Datum, RegisterRead, Symbol, Symbols},
    world::{ActionKind, EntityKind, MinionKind, NativeWorld},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
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
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum MarkerKind {
    Venom,
    Poison,
    Bounty,
}
impl MarkerKind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Venom => "venom",
            Self::Poison => "poison",
            Self::Bounty => "bounty",
        }
    }
}

#[derive(Clone, Debug, Hash)]
pub struct NativeMarker {
    pub key: Symbol,
    pub source: Option<Symbol>,
    pub kind: MarkerKind,
    pub target: Option<Symbol>,
    pub value: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Hash)]
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
#[derive(Default)]
pub(crate) struct MinionDefense {
    friendly: [i64; 3],
    enemy: [i64; 3],
}

impl MinionDefense {
    const KINDS: [&str; 3] = ["MELEE", "RANGED", "HEAVY"];

    fn add(&mut self, kind: MinionKind, friendly: bool) {
        let index = match kind {
            MinionKind::Melee => 0,
            MinionKind::Ranged => 1,
            MinionKind::Heavy => 2,
        };
        let side = if friendly {
            &mut self.friendly
        } else {
            &mut self.enemy
        };
        side[index] += 1;
    }

    fn modifier(&self) -> i64 {
        self.friendly.iter().sum::<i64>() - self.enemy.iter().sum::<i64>()
    }

    pub(crate) fn counts(&self, friendly: bool) -> impl Iterator<Item = (&'static str, i64)> {
        let side = if friendly { self.friendly } else { self.enemy };
        Self::KINDS
            .into_iter()
            .zip(side)
            .filter(|(_, count)| *count != 0)
    }
}

impl NativeWorld {
    /// Resolve the current nested action from live registers, never from a cache.
    pub fn performing_card(
        &self,
        unit: Symbol,
        context: &dyn RegisterRead,
    ) -> Result<Option<CardId>, String> {
        let Some(hero) = self.hero(unit) else {
            return Ok(None);
        };
        Ok(self.nested_performing_card(context)?.or(hero.current))
    }

    pub(crate) fn nested_performing_card(
        &self,
        context: &dyn RegisterRead,
    ) -> Result<Option<CardId>, String> {
        let keys = &self.definitions.stat_context;
        let Some(printed) = keys.text(context.get(keys.card))? else {
            return Ok(None);
        };
        let owner = keys.text(context.get(keys.owner))?;
        Ok(owner
            .and_then(|owner| self.card_for_hero_printed(owner, printed))
            .or_else(|| self.card_by_printed(printed)))
    }
    pub fn computed_stat_in_context(
        &self,
        unit: Symbol,
        stat: Stat,
        base: i64,
        context: &dyn RegisterRead,
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
        context: &dyn RegisterRead,
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
        Ok(self.minion_defense_contributions(unit, actor)?.modifier())
    }

    pub(crate) fn minion_defense_contributions(
        &self,
        unit: Symbol,
        actor: Option<Symbol>,
    ) -> Result<MinionDefense, String> {
        let mut contributions = MinionDefense::default();
        let Some(target) = self.entity(unit).filter(|entity| {
            matches!(
                entity.kind,
                EntityKind::Hero | EntityKind::Piece | EntityKind::Minion
            )
        }) else {
            return Ok(contributions);
        };
        let Some(position) = self.position(unit) else {
            return Ok(contributions);
        };
        let equivalence = self.illusion_minion_team(actor);
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
                    contributions.add(MinionKind::Melee, Some(team) == target.team);
                }
                continue;
            }
            let kind = entity.minion_kind.ok_or("minion kind is missing")?;
            let friendly = entity.team == target.team;
            let supports = match (distance, friendly) {
                (1, true) => matches!(kind, MinionKind::Melee | MinionKind::Heavy),
                (1, false) => true,
                (2, false) => kind == MinionKind::Ranged,
                _ => false,
            };
            if supports {
                contributions.add(kind, friendly);
            }
        }
        Ok(contributions)
    }
}
