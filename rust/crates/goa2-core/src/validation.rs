//! Typed displacement, action, repeat and movement legality shared by execution and previews.
use crate::{
    data::{Symbol, Symbols},
    geometry::Hex,
    world::NativeWorld,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DisplacementKind {
    Move,
    Push,
    Swap,
    Place,
}
impl DisplacementKind {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "MOVE" => Ok(Self::Move),
            "PUSH" => Ok(Self::Push),
            "SWAP" => Ok(Self::Swap),
            "PLACE" => Ok(Self::Place),
            _ => Err("unknown displacement type".into()),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Move => "MOVE",
            Self::Push => "PUSH",
            Self::Swap => "SWAP",
            Self::Place => "PLACE",
        }
    }
}
#[derive(Clone, Debug)]
enum Failure {
    MissingEntity,
    Occupied,
    Restricted(DisplacementKind),
    Action(Symbol),
    Repeat,
    NotOnBoard,
    Movement { maximum: i64, distance: i64 },
    WitchingHour,
    InvalidTarget,
    TargetOffBoard,
    LineOfSight,
}
#[derive(Clone, Debug)]
pub struct Verdict {
    pub allowed: bool,
    failure: Option<Failure>,
    blocking: Option<(Symbol, Symbol)>,
    card_source: Option<Symbol>,
}
impl Verdict {
    fn allow() -> Self {
        Self {
            allowed: true,
            failure: None,
            blocking: None,
            card_source: None,
        }
    }
    fn deny(failure: Failure, blocking: Option<(Symbol, Symbol)>) -> Self {
        Self {
            allowed: false,
            failure: Some(failure),
            blocking,
            card_source: None,
        }
    }
    /// Outer diagnostic boundary only. Native legality never serializes.
    pub fn export(&self, s: &Symbols) -> Value {
        let reason = match self.failure {
            None => String::new(),
            Some(Failure::MissingEntity) => "Entity not found".into(),
            Some(Failure::Occupied) => "Destination occupied".into(),
            Some(Failure::Restricted(kind)) => format!("{} prevented by area effect", kind.name()),
            Some(Failure::Action(kind)) => format!("Action prevented by effect: {}", s.name(kind)),
            Some(Failure::Repeat) => "Action repeat prevented".into(),
            Some(Failure::NotOnBoard) => "Unit not on board".into(),
            Some(Failure::Movement { maximum, distance }) => {
                format!("Movement limited to {maximum} (attempted {distance})")
            }
            Some(Failure::WitchingHour) => "Witching Hour: Attack value is below zero".into(),
            Some(Failure::InvalidTarget) => "Invalid actor or target".into(),
            Some(Failure::TargetOffBoard) => "Actor or target not on board".into(),
            Some(Failure::LineOfSight) => "Line of sight blocked".into(),
        };
        json!({"allowed":self.allowed,"reason":reason,"blocking_effect_ids":self.blocking.map(|(id,_)|vec![s.name(id)]).unwrap_or_default(),"blocking_modifier_ids":[],"blocked_by_source":self.blocking.map(|(_,source)|source).or(self.card_source).map(|source|s.name(source))})
    }
}
impl NativeWorld {
    pub fn can_target(
        &self,
        actor: Option<Symbol>,
        target: Option<Symbol>,
    ) -> Result<Verdict, String> {
        let (Some(actor), Some(target)) = (actor, target) else {
            return Ok(Verdict::deny(Failure::InvalidTarget, None));
        };
        if actor == self.definitions.effect_empty || target == self.definitions.effect_empty {
            return Ok(Verdict::deny(Failure::InvalidTarget, None));
        }
        let Some((from, to)) = self
            .resolved_position(actor)
            .zip(self.resolved_position(target))
        else {
            return Ok(Verdict::deny(Failure::TargetOffBoard, None));
        };
        // Smoke checks only strictly interior points of an axis-aligned segment;
        // scope/range/affects and the target's immunity are deliberately not used.
        if !from.straight(to) {
            return Ok(Verdict::allow());
        }
        for e in &self.effects.instances {
            let b = &e.definition.behavior;
            if b.kind != crate::effect_rules::Kind::LosBlocker || !self.effect_active(e) {
                continue;
            }
            let origin = b.origin_hex.or_else(|| {
                self.resolved_position(
                    e.scope_origin
                        .filter(|id| *id != self.definitions.effect_empty)
                        .unwrap_or(e.source),
                )
            });
            let Some(hex) = origin else {
                continue;
            };
            if hex != from
                && hex != to
                && u64::from(from.distance(hex)) + u64::from(hex.distance(to))
                    == u64::from(from.distance(to))
                && !self.effect_ignored(e, actor)?
                && self.restriction_actor_blocks(e, Some(actor), false)
            {
                return Ok(Verdict::deny(Failure::LineOfSight, Some((e.id, e.source))));
            }
        }
        Ok(Verdict::allow())
    }
    pub fn can_displace(
        &self,
        unit: Symbol,
        actor: Option<Symbol>,
        kind: DisplacementKind,
        destination: Option<Hex>,
    ) -> Result<Verdict, String> {
        use crate::effect_rules::Kind;
        if self.entity(unit).is_none() {
            return Ok(Verdict::deny(Failure::MissingEntity, None));
        }
        if kind == DisplacementKind::Place
            && destination.is_some_and(|to| self.occupant(to).is_some())
        {
            return Ok(Verdict::deny(Failure::Occupied, None));
        }
        if self.position(unit).is_none() {
            return Ok(Verdict::allow());
        }
        for e in &self.effects.instances {
            let b = &e.definition.behavior;
            if b.kind != Kind::PlacementPrevention
                || !self.effect_active(e)
                || !self.effect_in_scope(e, unit, true)?
                || self.effect_ignored(e, unit)?
            {
                continue;
            }
            if !b.displacement.contains(&kind) {
                continue;
            }
            if self.restriction_actor_blocks(e, actor, false) {
                return Ok(Verdict::deny(
                    Failure::Restricted(kind),
                    Some((e.id, e.source)),
                ));
            }
        }
        Ok(Verdict::allow())
    }
}

impl NativeWorld {
    fn restriction_actor_blocks(
        &self,
        e: &crate::effects::EffectInstance,
        actor: Option<Symbol>,
        unit_only: bool,
    ) -> bool {
        use crate::world::EntityKind;
        let Some((source, actor)) =
            self.entity(e.source)
                .zip(actor.and_then(|id| self.entity(id)).filter(|actor| {
                    !unit_only
                        || matches!(
                            actor.kind,
                            EntityKind::Hero | EntityKind::Piece | EntityKind::Minion
                        )
                }))
        else {
            return false;
        };
        let Some((source_team, actor_team)) = source.team.zip(actor.team) else {
            return false;
        };
        let own = self.hero_identity(actor.id) == self.hero_identity(e.source);
        let enemy = source_team != actor_team;
        let b = &e.definition.behavior;
        (b.self_actors && own) || (b.enemy_actors && enemy) || (b.friendly_actors && !enemy && !own)
    }
    fn actor_verdict(
        &self,
        actor: Symbol,
        mut check: impl FnMut(Symbol, Hex) -> Result<Verdict, String>,
    ) -> Result<Verdict, String> {
        if let Some(hex) = self.resolved_position(actor) {
            return check(actor, hex);
        }
        if self.entity(actor).is_some_and(|e| e.multi_piece) {
            let mut last = Verdict::allow();
            for piece in self.piece_ids(actor) {
                last = check(piece, self.position(piece).ok_or("piece position missing")?)?;
                if last.allowed {
                    return Ok(last);
                }
            }
            return Ok(last);
        }
        Ok(Verdict::allow())
    }
    pub fn repeat_legality(&self, actor: Symbol) -> Result<Verdict, String> {
        self.actor_verdict(actor, |unit, hex| {
            for e in &self.effects.instances {
                if e.definition.behavior.kind == crate::effect_rules::Kind::RepeatPrevention
                    && self.effect_active(e)
                    && self.effect_scope_at(e, unit, hex, true)?
                    && !self.effect_ignored(e, unit)?
                    && self.restriction_actor_blocks(e, Some(unit), true)
                {
                    return Ok(Verdict::deny(Failure::Repeat, Some((e.id, e.source))));
                }
            }
            Ok(Verdict::allow())
        })
    }
    pub fn action_legality(
        &self,
        actor: Symbol,
        action: crate::world::ActionKind,
        card: Option<crate::cards::CardId>,
        context: &crate::data::Registers,
    ) -> Result<Verdict, String> {
        use crate::world::ActionKind;
        let prerequisites = match action {
            ActionKind::FastTravel => Some(ActionKind::Movement),
            ActionKind::Clear => Some(ActionKind::Attack),
            _ => None,
        };
        if card.is_some_and(|id| self.card(id).is_none()) {
            return Err("unknown action card".into());
        }
        self.actor_verdict(actor, |unit, hex| {
            for required in prerequisites.into_iter().chain(std::iter::once(action)) {
                for e in &self.effects.instances {
                    let b = &e.definition.behavior;
                    if !self.effect_active(e) || !b.restrictions.contains(&required) {
                        continue;
                    }
                    if !b.except_colors.is_empty()
                        && let Some(card) = card.and_then(|id| self.card(id))
                    {
                        let def = &self.definitions.cards[card.definition];
                        if def.spell {
                            return Err("spell face action color not compiled".into());
                        }
                        if !card.facedown
                            && def
                                .color
                                .is_some_and(|color| b.except_colors.contains(&color))
                        {
                            continue;
                        }
                    }
                    if self.effect_scope_at(e, unit, hex, true)?
                        && !self.effect_ignored(e, unit)?
                        && self.restriction_actor_blocks(e, Some(unit), true)
                    {
                        return Ok(Verdict::deny(
                            Failure::Action(e.definition.kind),
                            Some((e.id, e.source)),
                        ));
                    }
                }
            }
            for required in prerequisites.into_iter().chain(std::iter::once(action)) {
                if let Some(source) =
                    self.card_action_blocker(unit, hex, required, card, context)?
                {
                    let mut result = Verdict::deny(Failure::WitchingHour, None);
                    result.card_source = Some(source);
                    return Ok(result);
                }
            }
            Ok(Verdict::allow())
        })
    }
    fn card_action_blocker(
        &self,
        actor: Symbol,
        actor_hex: Hex,
        action: crate::world::ActionKind,
        card: Option<crate::cards::CardId>,
        context: &crate::data::Registers,
    ) -> Result<Option<Symbol>, String> {
        use crate::world::{ActionKind, EntityKind};
        if action != ActionKind::Attack || card.is_none() {
            return Ok(None);
        }
        let Some(target) = self.entity(actor).filter(|e| {
            matches!(
                e.kind,
                EntityKind::Hero | EntityKind::Piece | EntityKind::Minion
            )
        }) else {
            return Ok(None);
        };
        let card = card.unwrap();
        for hero in &self.heroes {
            for source_card in self.active_aura_cards(hero) {
                let source_def = &self.definitions.cards[self
                    .card(source_card)
                    .ok_or("aura card missing")?
                    .definition];
                if !source_def.action_prevention {
                    continue;
                }
                if source_def.aura != crate::auras::CardAura::WitchingHour {
                    return Err("card action prevention hook not compiled".into());
                }
                let Some(source_hex) = self.resolved_position(hero.id) else {
                    continue;
                };
                if target.team == Some(hero.team) || self.actor_immune(actor, hero.id)? {
                    continue;
                }
                let radius = self.card_stats(hero.id, source_card)?.radius.unwrap_or(0);
                if !self.effect_connected(source_hex, actor_hex, &[actor])?
                    || i64::from(source_hex.distance(actor_hex)) > radius
                {
                    continue;
                }
                let instance = self.card(card).ok_or("action card missing")?;
                let def = &self.definitions.cards[instance.definition];
                if def.spell {
                    return Err("spell face attack stats not compiled".into());
                }
                let base = if !instance.facedown && def.primary == Some(ActionKind::Attack) {
                    def.primary_value.unwrap_or(0)
                } else if !instance.facedown
                    && let Some((_, base)) = def
                        .secondary
                        .iter()
                        .find(|(kind, _)| *kind == ActionKind::Attack)
                {
                    *base
                } else {
                    continue;
                };
                if self.computed_stat_for_card_in_context(
                    actor,
                    crate::stats::Stat::Attack,
                    base,
                    Some(card),
                    context,
                )? < 0
                {
                    return Ok(Some(hero.id));
                }
            }
        }
        Ok(None)
    }
    pub fn movement_legality(
        &self,
        unit: Symbol,
        distance: i64,
        is_action: bool,
        card: Option<crate::cards::CardId>,
        context: &crate::data::Registers,
    ) -> Result<Verdict, String> {
        if is_action {
            let action =
                self.action_legality(unit, crate::world::ActionKind::Movement, card, context)?;
            if !action.allowed {
                return Ok(action);
            }
        }
        if self.position(unit).is_none() {
            return Ok(Verdict::deny(Failure::NotOnBoard, None));
        }
        let mut cap = None;
        for e in &self.effects.instances {
            let b = &e.definition.behavior;
            if b.kind != crate::effect_rules::Kind::MovementZone
                || !self.effect_active(e)
                || !self.effect_in_scope(e, unit, true)?
                || self.effect_ignored(e, unit)?
            {
                continue;
            }
            if b.actions_only && !is_action {
                continue;
            }
            if let Some(maximum) = b.maximum
                && cap.is_none_or(|(current, _, _)| maximum < current)
            {
                cap = Some((maximum, e.id, e.source));
            }
        }
        if let Some((maximum, id, source)) = cap
            && distance > maximum
        {
            return Ok(Verdict::deny(
                Failure::Movement { maximum, distance },
                Some((id, source)),
            ));
        }
        Ok(Verdict::allow())
    }
}
