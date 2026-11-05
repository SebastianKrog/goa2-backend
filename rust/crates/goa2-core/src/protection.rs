//! Minion saves and limited defeat bounties share the native scope/topology rules.
use crate::{
    cards::CardId,
    combat::CombatEvent,
    data::Symbol,
    world::{EntityKind, NativeWorld},
};
use std::sync::Arc;

#[derive(Clone, Debug, Hash)]
pub(crate) struct ProtectionBinding {
    pub minion: Symbol,
    pub killer: Option<Symbol>,
    pub tried: Arc<[Symbol]>,
}
pub(crate) enum ProtectionDecision {
    Missing,
    Defeat,
    Next(Symbol),
    Sacrifice {
        effect: Symbol,
        hero: Symbol,
        token: Symbol,
    },
    Cards {
        effect: Symbol,
        hero: Symbol,
        cards: Vec<CardId>,
    },
}
impl NativeWorld {
    pub(crate) fn protection_candidate(
        &self,
        minion: Symbol,
        tried: &[Symbol],
    ) -> Result<Option<usize>, String> {
        let Some(unit) = self.entity(minion) else {
            return Ok(None);
        };
        let mut candidate = None;
        for (i, e) in self.effects.instances.iter().enumerate() {
            let b = &e.definition.behavior;
            if b.kind != crate::effect_rules::Kind::MinionProtection
                || tried.contains(&e.id)
                || !self.effect_active(e)
                || !self.effect_in_scope(e, minion, false)?
                || !b.protected_types.is_empty()
                    && unit
                        .minion_kind
                        .is_none_or(|kind| !b.protected_types.contains(&kind))
            {
                continue;
            }
            if candidate.is_none() {
                candidate = Some(i);
            }
            if b.sacrifice {
                return Ok(Some(i));
            }
        }
        Ok(candidate)
    }
    pub(crate) fn protection_decision(
        &self,
        spec: &ProtectionBinding,
    ) -> Result<ProtectionDecision, String> {
        let Some(unit) = self.entity(spec.minion).filter(|e| {
            matches!(
                e.kind,
                EntityKind::Hero | EntityKind::Piece | EntityKind::Minion
            )
        }) else {
            return Ok(ProtectionDecision::Missing);
        };
        if unit.kind != EntityKind::Minion {
            return Err("minion protection requires a minion".into());
        }
        let Some(index) = self.protection_candidate(spec.minion, &spec.tried)? else {
            return Ok(ProtectionDecision::Defeat);
        };
        let e = &self.effects.instances[index];
        let Some(hero) = self.hero(e.source) else {
            return Ok(ProtectionDecision::Next(e.id));
        };
        let b = &e.definition.behavior;
        if b.sacrifice {
            let Some(token) = e.scope_origin.filter(|id| {
                self.entity(*id)
                    .is_some_and(|u| u.kind == EntityKind::Token)
                    && self.position(*id).is_some()
            }) else {
                return Ok(ProtectionDecision::Next(e.id));
            };
            return Ok(ProtectionDecision::Sacrifice {
                effect: e.id,
                hero: hero.id,
                token,
            });
        }
        let cards = hero
            .hand
            .iter()
            .copied()
            .filter(|id| {
                self.card(*id).is_some_and(|c| {
                    self.definitions.cards[c.definition]
                        .color
                        .is_some_and(|color| b.discard_colors.contains(&color))
                })
            })
            .collect::<Vec<_>>();
        if cards.is_empty() {
            Ok(ProtectionDecision::Next(e.id))
        } else {
            Ok(ProtectionDecision::Cards {
                effect: e.id,
                hero: hero.id,
                cards,
            })
        }
    }
    pub(crate) fn award_minion_bounties(
        &mut self,
        minion: Symbol,
    ) -> Result<Vec<CombatEvent>, String> {
        let Some(unit) = self.entity(minion) else {
            return Ok(Vec::new());
        };
        let Some(hex) = self.position(minion) else {
            return Ok(Vec::new());
        };
        let mut payouts = Vec::new();
        // A source list can contain the same effect model more than once. Stage
        // its coin budget by arena handle, while distinct equal-ID models keep
        // independent budgets. Small sparse storage avoids a global alias scan.
        let mut shared = Vec::<(crate::objects::ObjectId, i64)>::new();
        for (index, e) in self.effects.instances.iter().enumerate() {
            let b = &e.definition.behavior;
            if b.kind != crate::effect_rules::Kind::MinionBounty
                || !self.effect_active(e)
                || e.remaining_bounty.unwrap_or(0) <= 0
            {
                continue;
            }
            let remaining = e
                .arena_model
                .and_then(|model| {
                    shared
                        .iter()
                        .find(|(id, _)| *id == model)
                        .map(|(_, value)| *value)
                })
                .unwrap_or_else(|| e.remaining_bounty.unwrap_or(0));
            if remaining <= 0 {
                continue;
            }
            let subject = b.subject.unwrap_or(e.source);
            let Some(hero) = self.hero(subject) else {
                continue;
            };
            // Bounties use the beneficiary model's actual team. The roster
            // binding is used for assist distribution, not this relation test.
            if unit.team == self.entity(hero.id).and_then(|entity| entity.team) {
                continue;
            }
            let Some(origin) = self.resolved_position(subject) else {
                continue;
            };
            if self.effect_connected(origin, hex, &[])?
                && i64::from(origin.distance(hex)) <= b.range
            {
                payouts.push((index, hero.id, subject));
                if let Some(model) = e.arena_model {
                    if let Some((_, value)) = shared.iter_mut().find(|(id, _)| *id == model) {
                        *value -= 1;
                    } else {
                        shared.push((model, remaining - 1));
                    }
                }
            }
        }
        let mut events = Vec::new();
        for (index, owner, subject) in payouts {
            let hero = self.hero_index(owner).ok_or("bounty hero missing")?;
            self.heroes[hero].gold = self.heroes[hero]
                .gold
                .checked_add(1)
                .ok_or("bounty gold outside i64")?;
            if self.effects.instances[index].arena_model.is_none() {
                self.effects.instances[index].remaining_bounty = self.effects.instances[index]
                    .remaining_bounty
                    .map(|n| n - 1);
            }
            events.push(CombatEvent::Bounty {
                hero: subject,
                minion,
            });
        }
        for row in &mut self.effects.instances {
            if let Some(model) = row.arena_model
                && let Some((_, value)) = shared.iter().find(|(id, _)| *id == model)
            {
                row.remaining_bounty = Some(*value);
            }
        }
        Ok(events)
    }
}
