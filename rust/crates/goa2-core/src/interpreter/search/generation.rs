//! Legal joint planning candidates and positional root ordering.

use super::{
    DecisionMove, EngineError, HeroCardScores, JointPlanOrder, JointPlans, NativeSession, Phase,
    PlanningCommitment, Result, Symbol,
};
use super::{greedy_card_cost, ranked_reply_indices};
use crate::{data::Datum, geometry::Hex, interpreter::datum_hex, world::EntityKind};

impl NativeSession {
    /// Generate up to cap joint plans. The requesting hero's choices vary
    /// first so a bounded friendly set covers their options before teammates'.
    pub fn joint_plans(&self, player: Symbol, friendly: bool, cap: usize) -> Result<JointPlans> {
        self.joint_plans_partition(player, friendly, cap, None)
    }

    pub(in crate::interpreter) fn joint_plans_partition(
        &self,
        player: Symbol,
        friendly: bool,
        cap: usize,
        controlled: Option<&[Symbol]>,
    ) -> Result<JointPlans> {
        self.joint_plans_with_reply_order(
            player,
            friendly,
            cap,
            controlled,
            JointPlanOrder::PlayerFirst,
            &[],
        )
    }

    pub(super) fn joint_plans_with_reply_order(
        &self,
        player: Symbol,
        friendly: bool,
        cap: usize,
        controlled: Option<&[Symbol]>,
        order: JointPlanOrder,
        greedy_scores: &[(Symbol, HeroCardScores)],
    ) -> Result<JointPlans> {
        let invalid = |reason: &str| EngineError::InvalidInput(reason.into());
        if self.planning.phase != Phase::Planning || !(1..=4096).contains(&cap) {
            return Err(invalid(
                "joint plan generation requires planning and a cap in 1..4096",
            ));
        }
        let team = self
            .world
            .hero(player)
            .filter(|hero| hero.id == player)
            .ok_or_else(|| invalid("search player is not a hero"))?
            .team;
        let mut dimensions = Vec::new();
        let mut strides = Vec::new();
        let mut costs = Vec::new();
        let mut total = 1usize;
        let mut heroes = self
            .world
            .heroes
            .iter()
            .filter(|hero| {
                controlled.map_or(hero.team == team, |ids| ids.contains(&hero.id)) == friendly
            })
            .collect::<Vec<_>>();
        heroes.sort_by_key(|hero| hero.id != player);
        for hero in heroes {
            let mut choices = self.hero_planning_commitments(hero.id)?;
            if choices.is_empty() {
                continue;
            }
            if let JointPlanOrder::OpponentReplies(seed) = order
                && hero.team != team
            {
                let preferred = self.greedy_opponent_commitment(hero.id, seed)?;
                let mut ranks = std::collections::HashMap::new();
                for card in choices.iter().filter_map(|choice| choice.first) {
                    if let std::collections::hash_map::Entry::Vacant(entry) = ranks.entry(card) {
                        entry.insert(self.greedy_defense_rank(hero.id, card)?);
                    }
                }
                choices.sort_by_key(|choice| {
                    (
                        Some(*choice) != preferred,
                        choice.first.map_or((true, i64::MAX), |card| ranks[&card]),
                        choice.second.is_some(),
                    )
                });
            }
            if let Some((_, scores)) = greedy_scores.iter().find(|(id, _)| *id == hero.id) {
                let best = scores.iter().map(|(_, score)| *score).max().unwrap();
                let cost = |choice: &PlanningCommitment| {
                    let score = scores
                        .iter()
                        .find_map(|(card, score)| (*card == choice.first).then_some(*score));
                    greedy_card_cost(best, score)
                };
                choices.sort_by_key(cost);
                costs.push(Some(choices.iter().map(cost).collect()));
            } else {
                costs.push(None);
            }
            strides.push(total);
            total = total
                .checked_mul(choices.len())
                .ok_or_else(|| invalid("joint plan count overflow"))?;
            dimensions.push(choices);
        }
        let indices = if !matches!(order, JointPlanOrder::PlayerFirst) || !greedy_scores.is_empty()
        {
            ranked_reply_indices(&dimensions, &strides, &costs, total.min(cap))
        } else {
            (0..total.min(cap)).collect()
        };
        let plans = indices
            .into_iter()
            .map(|mut index| {
                dimensions
                    .iter()
                    .map(|choices| {
                        let choice = choices[index % choices.len()];
                        index /= choices.len();
                        choice
                    })
                    .collect()
            })
            .collect();
        Ok(JointPlans { plans, total })
    }

    pub(in crate::interpreter) fn hero_planning_commitments(
        &self,
        hero: Symbol,
    ) -> Result<Vec<PlanningCommitment>> {
        let invalid = |reason: &str| EngineError::InvalidInput(reason.into());
        let moves = self.planning_choices(hero)?;
        let cards = moves
            .iter()
            .filter_map(|decision| match decision {
                DecisionMove::Card { card, .. } => Some(*card),
                _ => None,
            })
            .collect::<Vec<_>>();
        let second_slot = moves.contains(&DecisionMove::FinishPlanning { hero });
        let mut choices = Vec::new();
        for decision in moves {
            let first = match decision {
                DecisionMove::Card { card, .. } => Some(card),
                DecisionMove::Pass { .. } | DecisionMove::FinishPlanning { .. } => None,
                _ => return Err(invalid("unexpected joint planning choice")),
            };
            choices.push(PlanningCommitment {
                hero,
                first,
                second: None,
            });
        }
        if !second_slot && self.planning.two_cards(&self.world, hero) {
            // Put single-card plans first so capped sets still see every
            // possible first card before considering second-card variants.
            for &first in &cards {
                for &second in &cards {
                    if first != second {
                        choices.push(PlanningCommitment {
                            hero,
                            first: Some(first),
                            second: Some(second),
                        });
                    }
                }
            }
        }
        Ok(choices)
    }

    /// Search generated plans on one clean sampled world. The returned
    /// commitment belongs to the requesting hero and is valid on the live root.

    pub(in crate::interpreter) fn order_movement_roots(
        &self,
        player: Symbol,
        moves: &mut [DecisionMove],
    ) {
        if moves.len() < 3 {
            return;
        }
        let Some(actor) = self.movement_root_actor() else {
            return;
        };
        let Some(hero) = self.world.hero(actor).filter(|hero| hero.id == player) else {
            return;
        };
        let mut enemy_heroes = Vec::new();
        let mut enemy_minions = Vec::new();
        for &(id, hex) in &self.world.locations {
            let Some(entity) = self.world.entity(id) else {
                continue;
            };
            if entity.team.is_some_and(|team| team != hero.team) {
                match entity.kind {
                    EntityKind::Hero | EntityKind::Piece => enemy_heroes.push(hex),
                    EntityKind::Minion => enemy_minions.push(hex),
                    _ => {}
                }
            }
        }
        if enemy_heroes.is_empty() && enemy_minions.is_empty() {
            return;
        }
        let destination = |decision: &DecisionMove| match decision {
            DecisionMove::Input {
                selection: selection @ Datum::Object(_),
                ..
            } => datum_hex(selection, &self.symbols, self.coordinates).ok(),
            _ => None,
        };
        let nearest =
            |hex: Hex, enemies: &[Hex]| enemies.iter().map(|&enemy| hex.distance(enemy)).min();
        let distances = moves
            .iter()
            .filter_map(|decision| {
                let hex = destination(decision)?;
                Some((
                    hex,
                    nearest(hex, &enemy_minions),
                    nearest(hex, &enemy_heroes),
                ))
            })
            .collect::<Vec<_>>();
        let mut approach_minion = Vec::new();
        let mut approach_hero = Vec::new();
        let mut retreat = Vec::new();
        for (hex, minion, hero) in distances {
            if let Some(distance) = minion {
                approach_minion.push((hex, distance));
            }
            if let Some(distance) = hero {
                approach_hero.push((hex, distance));
            }
            if let Some(distance) = minion.into_iter().chain(hero).min() {
                retreat.push((hex, distance));
            }
        }
        approach_minion.sort_by_key(|&(_, distance)| distance);
        approach_hero.sort_by_key(|&(_, distance)| distance);
        retreat.sort_by_key(|&(_, distance)| std::cmp::Reverse(distance));
        let mut priority = std::collections::HashMap::with_capacity(retreat.len());
        for rank in 0..retreat.len() {
            for lane in [&approach_minion, &approach_hero, &retreat] {
                if let Some(&(hex, _)) = lane.get(rank) {
                    let next = priority.len();
                    priority.entry(hex).or_insert(next);
                }
            }
        }
        moves.sort_by_cached_key(|decision| {
            destination(decision)
                .and_then(|hex| priority.get(&hex).copied())
                .unwrap_or(usize::MAX)
        });
    }
}
