//! Shared typed effect scope, topology and stat-immunity rules.
use crate::{
    data::{Symbol, Symbols},
    effects::EffectInstance,
    geometry::Hex,
    stats::Stat,
    world::{EntityKind, NativeWorld},
};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Other,
    DoubleItems,
    AreaStat,
    BasicBonus,
    AttackImmunity,
    ActionImmunity,
    Split,
    Isolation,
    IllusionMinions,
    PlacementPrevention,
    MovementZone,
    RepeatPrevention,
    StaticBarrier,
    EmptyHexObstacle,
    Petrify,
    MovementAura,
    ControlAction,
    DiscardShield,
    MinionProtection,
    MinionBounty,
    BattleExclusion,
    LosBlocker,
    PreMovement,
    PreDiscard,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    Global,
    Point,
    Adjacent,
    Radius,
    Line,
    Zone,
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum Affects {
    All,
    SelfOnly,
    Friendly,
    FriendlyHeroes,
    SelfFriendlyHeroes,
    Enemy,
    EnemyHeroes,
    Heroes,
    Minions,
}
#[derive(Clone, Debug)]
pub(crate) struct Behavior {
    pub kind: Kind,
    pub shape: Shape,
    pub affects: Affects,
    pub range: i64,
    pub origin_hex: Option<Hex>,
    pub stat: Option<Stat>,
    pub value: i64,
    pub minimum: Option<i64>,
    pub subject: Option<Symbol>,
    pub friendly_actors: bool,
    pub enemy_actors: bool,
    pub self_actors: bool,
    pub displacement: Vec<crate::validation::DisplacementKind>,
    pub restrictions: Vec<crate::world::ActionKind>,
    pub except_colors: Vec<Symbol>,
    pub maximum: Option<i64>,
    pub actions_only: bool,
    pub barrier_radius: i64,
    pub grants_through: bool,
    pub controlled_card: Option<Symbol>,
    pub protected_types: Vec<crate::world::MinionKind>,
    pub discard_colors: Vec<Symbol>,
    pub sacrifice: bool,
    pub discard_or_defeat: bool,
    pub basic_only: bool,
    pub non_basic_only: bool,
    pub except_attackers: Vec<Symbol>,
    pub axis: Option<usize>,
    pub split: i64,
    pub isolated: Option<Hex>,
}
impl Behavior {
    pub(crate) fn bind_primary(
        &mut self,
        mut bind: impl FnMut(&mut crate::data::Datum) -> Result<(), String>,
    ) -> Result<(), String> {
        use crate::data::Datum;
        for number in [
            &mut self.value,
            &mut self.range,
            &mut self.split,
            &mut self.barrier_radius,
        ]
        .into_iter()
        .chain(self.minimum.iter_mut())
        .chain(self.maximum.iter_mut())
        {
            let mut value = Datum::Int(*number);
            bind(&mut value)?;
            *number = match value {
                Datum::Int(value) => value,
                _ => return Err("effect numeric binding changed type".into()),
            };
        }
        for id in self
            .subject
            .iter_mut()
            .chain(self.except_attackers.iter_mut())
            .chain(self.controlled_card.iter_mut())
        {
            let mut value = Datum::Text(*id);
            bind(&mut value)?;
            *id = match value {
                Datum::Text(value) => value,
                _ => return Err("effect identity binding changed type".into()),
            };
        }
        Ok(())
    }
    pub fn compile(v: &Value, s: &mut Symbols) -> Result<Self, String> {
        let kind = match v["effect_type"].as_str() {
            Some("placement_prevention") => Kind::PlacementPrevention,
            Some("movement_zone") => Kind::MovementZone,
            Some("repeat_prevention") => Kind::RepeatPrevention,
            Some("static_barrier") => Kind::StaticBarrier,
            Some("empty_hex_obstacle") => Kind::EmptyHexObstacle,
            Some("petrify") => Kind::Petrify,
            Some("movement_aura_zone") => Kind::MovementAura,
            Some("control_next_action") => Kind::ControlAction,
            Some("discard_shield") => Kind::DiscardShield,
            Some("minion_protection") => Kind::MinionProtection,
            Some("minion_defeat_bounty") => Kind::MinionBounty,
            Some("minion_battle_exclusion") => Kind::BattleExclusion,
            Some("double_items") => Kind::DoubleItems,
            Some("area_stat_modifier") => Kind::AreaStat,
            Some("basic_action_stat_bonus") => Kind::BasicBonus,
            Some("attack_immunity") => Kind::AttackImmunity,
            Some("los_blocker") => Kind::LosBlocker,
            Some("pre_action_movement") => Kind::PreMovement,
            Some("pre_action_discard") => Kind::PreDiscard,
            Some("immunity_enemy_actions") => Kind::ActionImmunity,
            Some("topology_split") => Kind::Split,
            Some("topology_isolation") => Kind::Isolation,
            Some("illusion_minion_equivalence") => Kind::IllusionMinions,
            _ => Kind::Other,
        };
        let scope = &v["scope"];
        let shape = match scope["shape"].as_str() {
            Some("global") => Shape::Global,
            Some("point") => Shape::Point,
            Some("adjacent") => Shape::Adjacent,
            Some("radius") => Shape::Radius,
            Some("line") => Shape::Line,
            Some("zone") => Shape::Zone,
            _ => return Err("invalid effect shape".into()),
        };
        let affects = match scope["affects"].as_str() {
            Some("all_units") => Affects::All,
            Some("self") => Affects::SelfOnly,
            Some("friendly_units") => Affects::Friendly,
            Some("friendly_heroes") => Affects::FriendlyHeroes,
            Some("self_and_friendly_heroes") => Affects::SelfFriendlyHeroes,
            Some("enemy_units") => Affects::Enemy,
            Some("enemy_heroes") => Affects::EnemyHeroes,
            Some("all_heroes") => Affects::Heroes,
            Some("all_minions") => Affects::Minions,
            _ => return Err("invalid effect affects".into()),
        };
        let integer = |key: &str| {
            v[key]
                .as_i64()
                .ok_or_else(|| format!("invalid effect {key}"))
        };
        let flag = |key: &str| {
            v[key]
                .as_bool()
                .ok_or_else(|| format!("invalid effect {key}"))
        };
        let hex = |value: &Value| -> Result<Option<Hex>, String> {
            if value.is_null() {
                Ok(None)
            } else {
                serde_json::from_value(value.clone())
                    .map(Some)
                    .map_err(|e| e.to_string())
            }
        };
        Ok(Self {
            kind,
            shape,
            affects,
            range: scope["range"].as_i64().ok_or("invalid scope range")?,
            origin_hex: hex(&scope["origin_hex"])?,
            stat: v["stat_type"].as_str().map(Stat::parse).transpose()?,
            value: integer("stat_value")?,
            minimum: if v["apply_stat_value_only_if_result_at_least"].is_null() {
                None
            } else {
                Some(integer("apply_stat_value_only_if_result_at_least")?)
            },
            subject: v["subject_id"]
                .as_str()
                .filter(|v| !v.is_empty())
                .map(|v| s.intern(v)),
            restrictions: v["restrictions"]
                .as_array()
                .ok_or("invalid action restrictions")?
                .iter()
                .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
                .collect::<Result<_, String>>()?,
            except_colors: v["except_card_colors"]
                .as_array()
                .ok_or("invalid card color exceptions")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(|v| s.intern(v))
                        .ok_or("invalid exception color".into())
                })
                .collect::<Result<_, String>>()?,
            maximum: if v["max_value"].is_null() {
                None
            } else {
                Some(integer("max_value")?)
            },
            actions_only: flag("limit_actions_only")?,
            barrier_radius: integer("barrier_radius")?,
            grants_through: flag("grants_pass_through_obstacles")?,
            controlled_card: v["controlled_card_id"].as_str().map(|v| s.intern(v)),
            protected_types: serde_json::from_value(v["protected_minion_types"].clone())
                .map_err(|e| e.to_string())?,
            discard_colors: v["allowed_discard_colors"]
                .as_array()
                .ok_or("invalid protection colors")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(|v| s.intern(v))
                        .ok_or("invalid protection color".into())
                })
                .collect::<Result<_, String>>()?,
            sacrifice: flag("sacrifice_origin_token")?,
            discard_or_defeat: flag("discard_or_defeat")?,
            friendly_actors: flag("blocks_friendly_actors")?,
            enemy_actors: flag("blocks_enemy_actors")?,
            self_actors: flag("blocks_self")?,
            displacement: v["displacement_blocks"]
                .as_array()
                .ok_or("invalid displacement restrictions")?
                .iter()
                .map(|v| {
                    crate::validation::DisplacementKind::parse(
                        v.as_str().ok_or("invalid displacement type")?,
                    )
                })
                .collect::<Result<_, _>>()?,
            basic_only: flag("basic_attacks_only")?,
            non_basic_only: flag("non_basic_attacks_only")?,
            except_attackers: v["except_attacker_ids"]
                .as_array()
                .ok_or("invalid except attackers")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(|v| s.intern(v))
                        .ok_or("invalid except attacker".into())
                })
                .collect::<Result<_, String>>()?,
            axis: match v["split_axis"].as_str() {
                Some("q") => Some(0),
                Some("r") => Some(1),
                Some("s") => Some(2),
                _ => None,
            },
            split: integer("split_value")?,
            isolated: hex(&v["isolated_hex"])?,
        })
    }
}
impl NativeWorld {
    pub(crate) fn hero_identity(&self, id: Symbol) -> Symbol {
        self.entity(id)
            .filter(|e| e.kind == EntityKind::Piece)
            .and_then(|e| e.owner)
            .unwrap_or(id)
    }
    fn effect_controller(&self, id: Symbol) -> Symbol {
        self.entity(id)
            .and_then(|e| e.owner)
            .unwrap_or_else(|| self.hero_identity(id))
    }
    pub(crate) fn effect_active(&self, effect: &EffectInstance) -> bool {
        effect.is_active(self.combat.round, self.combat.turn)
    }
    fn effect_affects(&self, e: &EffectInstance, target: Symbol, validation: bool) -> bool {
        let filter = e.definition.behavior.affects;
        if matches!(filter, Affects::All) {
            return true;
        }
        let (Some(source), Some(unit)) = (self.entity(e.source), self.entity(target)) else {
            return false;
        };
        if validation && (source.team.is_none() || unit.team.is_none()) {
            return false;
        }
        let same = source.team == unit.team;
        let own = self.hero_identity(target) == self.hero_identity(e.source);
        let hero = matches!(unit.kind, EntityKind::Hero | EntityKind::Piece);
        match filter {
            Affects::All => true,
            Affects::SelfOnly => own,
            Affects::Friendly => same && (!validation || !own),
            Affects::FriendlyHeroes => same && hero && !own,
            Affects::SelfFriendlyHeroes => same && hero,
            Affects::Enemy => !same,
            Affects::EnemyHeroes => !same && hero,
            Affects::Heroes => hero,
            Affects::Minions => unit.kind == EntityKind::Minion,
        }
    }
    pub(crate) fn effect_in_scope(
        &self,
        e: &EffectInstance,
        target: Symbol,
        validation: bool,
    ) -> Result<bool, String> {
        let Some(hex) = self.position(target) else {
            return Ok(false);
        };
        self.effect_scope_at(e, target, hex, validation)
    }
    pub(crate) fn effect_scope_at(
        &self,
        e: &EffectInstance,
        target: Symbol,
        hex: Hex,
        validation: bool,
    ) -> Result<bool, String> {
        if !self.effect_affects(e, target, validation) {
            return Ok(false);
        }
        let b = &e.definition.behavior;
        let origin = b.origin_hex.or_else(|| {
            self.resolved_position(
                e.scope_origin
                    .filter(|id| *id != self.definitions.effect_empty)
                    .unwrap_or(e.source),
            )
        });
        let Some(origin) = origin.or_else(|| (b.shape == Shape::Global).then_some(hex)) else {
            return Ok(false);
        };
        match b.shape {
            Shape::Point => Ok(origin == hex),
            Shape::Zone => Ok(self
                .zone_at(origin)
                .is_some_and(|z| Some(z) == self.zone_at(hex))),
            Shape::Global => self.effect_connected(origin, hex, &[target]),
            Shape::Adjacent => {
                Ok(origin.distance(hex) == 1 && self.effect_connected(origin, hex, &[target])?)
            }
            Shape::Radius => Ok(self.effect_connected(origin, hex, &[target])?
                && i64::from(origin.distance(hex)) <= b.range),
            Shape::Line => Ok(origin.straight(hex)
                && self.effect_connected(origin, hex, &[target])?
                && i64::from(origin.distance(hex)) <= b.range),
        }
    }
    pub(crate) fn effect_connected(
        &self,
        from: Hex,
        to: Hex,
        units: &[Symbol],
    ) -> Result<bool, String> {
        self.effect_connected_inner(from, to, units, 0)
    }
    fn effect_connected_inner(
        &self,
        from: Hex,
        to: Hex,
        units: &[Symbol],
        depth: usize,
    ) -> Result<bool, String> {
        if depth > 64 {
            return Err("recursive topology/immunity scope exceeds supported depth".into());
        }
        for e in &self.effects.instances {
            let b = &e.definition.behavior;
            if !matches!(b.kind, Kind::Split | Kind::Isolation) {
                continue;
            }
            // Source topology deliberately does not consult duration/is_active.
            let mut ignored = false;
            for id in units
                .iter()
                .copied()
                .chain(self.occupant(from))
                .chain(self.occupant(to))
            {
                if self.effect_ignored_inner(e, id, depth + 1)? {
                    ignored = true;
                    break;
                }
            }
            if ignored {
                continue;
            }
            let region = |h: Hex| {
                let (q, r, s) = h.coordinates();
                b.axis.map(|axis| i64::from([q, r, s][axis]).cmp(&b.split))
            };
            let isolated = self.resolved_position(e.source).or(b.isolated);
            if b.kind == Kind::Isolation && isolated == Some(from) {
                continue;
            }
            let a = region(from);
            let c = region(to);
            if matches!(
                (a, c),
                (
                    Some(std::cmp::Ordering::Less),
                    Some(std::cmp::Ordering::Greater)
                ) | (
                    Some(std::cmp::Ordering::Greater),
                    Some(std::cmp::Ordering::Less)
                )
            ) {
                return Ok(false);
            }
            if b.kind == Kind::Isolation
                && isolated == Some(to)
                && !matches!(a, None | Some(std::cmp::Ordering::Equal))
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(crate) fn effect_ignored(
        &self,
        e: &EffectInstance,
        target: Symbol,
    ) -> Result<bool, String> {
        self.effect_ignored_inner(e, target, 0)
    }
    fn effect_ignored_inner(
        &self,
        e: &EffectInstance,
        target: Symbol,
        depth: usize,
    ) -> Result<bool, String> {
        let Some(_unit) = self.entity(target).filter(|u| {
            matches!(
                u.kind,
                EntityKind::Hero | EntityKind::Piece | EntityKind::Minion
            )
        }) else {
            return Ok(false);
        };
        if self.effect_controller(e.source) == self.effect_controller(target) {
            return Ok(false);
        }
        if e.origin_basic
            .is_some_and(|basic| self.attack_immune(target, Some(e.source), basic))
        {
            return Ok(true);
        }
        self.actor_immune_inner(target, e.source, depth)
    }
    pub(crate) fn attack_immune(&self, target: Symbol, actor: Option<Symbol>, basic: bool) -> bool {
        let actor = actor.filter(|id| *id != self.definitions.effect_empty);
        self.effects.instances.iter().any(|shield| {
            let b = &shield.definition.behavior;
            b.kind == Kind::AttackImmunity
                && shield.active
                && self.effect_active(shield)
                && shield.source == target
                && (!b.basic_only || basic)
                && (!b.non_basic_only || !basic)
                && actor.is_none_or(|actor| !b.except_attackers.contains(&actor))
        })
    }
    pub(crate) fn illusion_minion_team(&self, actor: Option<Symbol>) -> Option<Symbol> {
        let owner = self.hero_identity(actor.filter(|id| *id != self.definitions.effect_empty)?);
        self.effects.instances.iter().find_map(|effect| {
            (effect.definition.behavior.kind == Kind::IllusionMinions
                && effect.active
                && self.hero_identity(effect.source) == owner)
                .then(|| self.hero(effect.source).map(|hero| hero.team))
                .flatten()
        })
    }
    pub(crate) fn equivalent_illusion(&self, id: Symbol, actor: Option<Symbol>) -> bool {
        self.entity(id).is_some_and(|entity| {
            entity.kind == EntityKind::Token
                && entity.token_type == Some(self.definitions.illusion_token)
        }) && self.illusion_minion_team(actor).is_some()
    }
    pub(crate) fn actor_immune(&self, target: Symbol, actor: Symbol) -> Result<bool, String> {
        self.actor_immune_inner(target, actor, 0)
    }
    fn actor_immune_inner(
        &self,
        target: Symbol,
        actor: Symbol,
        depth: usize,
    ) -> Result<bool, String> {
        let Some(unit) = self.entity(target).filter(|u| {
            matches!(
                u.kind,
                EntityKind::Hero | EntityKind::Piece | EntityKind::Minion
            )
        }) else {
            return Ok(false);
        };
        if self.effect_controller(actor) == self.effect_controller(target) {
            return Ok(false);
        }
        if self.ordinary_immune(target, Some(actor)) {
            return Ok(true);
        }
        let actor_team = self
            .entity(actor)
            .and_then(|a| {
                a.team
                    .or_else(|| a.owner.and_then(|owner| self.hero(owner)).map(|h| h.team))
            })
            .or_else(|| self.hero(actor).map(|h| h.team));
        if let (Some(team), Some(actor_team)) = (unit.team, actor_team) {
            for shield in &self.effects.instances {
                let b = &shield.definition.behavior;
                if b.kind != Kind::ActionImmunity
                    || !shield.active
                    || (team == actor_team && !b.friendly_actors)
                {
                    continue;
                }
                if b.shape == Shape::Radius {
                    // Validation's relational filter excludes self from FRIENDLY_UNITS.
                    let Some(hex) = self.resolved_position(target) else {
                        continue;
                    };
                    if !self.effect_affects(shield, target, true) {
                        continue;
                    }
                    let origin = b.origin_hex.or_else(|| {
                        self.resolved_position(
                            shield
                                .scope_origin
                                .filter(|id| *id != self.definitions.effect_empty)
                                .unwrap_or(shield.source),
                        )
                    });
                    if let Some(origin) = origin
                        && self.effect_connected_inner(origin, hex, &[target], depth + 1)?
                        && i64::from(origin.distance(hex)) <= b.range
                    {
                        return Ok(true);
                    }
                } else if b.subject.unwrap_or(shield.source) == target {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}
