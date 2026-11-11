//! Position, material and lane scoring for search leaves.

use super::{EngineError, NativeSession, Result, SearchEvaluation, WIN};
use crate::{
    cards::HeroRecord,
    data::Symbol,
    world::{ActionKind, EntityKind, MinionKind},
};

impl NativeSession {
    /// The Python lab's lab-attack-range-v1 leaf evaluation.
    pub fn evaluate_lab_attack_range(&self, player: Symbol) -> Result<f64> {
        self.evaluate_attack_range(player, false)
    }

    fn evaluate_attack_range(&self, player: Symbol, accumulated_gold: bool) -> Result<f64> {
        let hero = self
            .world
            .hero(player)
            .filter(|hero| hero.id == player)
            .ok_or_else(|| EngineError::InvalidInput("evaluation player is not a hero".into()))?;
        if let Some(winner) = self.world.combat.outcome.and_then(|outcome| outcome.team) {
            return Ok(if winner == hero.team { 1.0 } else { -1.0 });
        }

        let sign = |team| if team == hero.team { 1 } else { -1 };
        let life = self
            .world
            .combat
            .team_lives()
            .map(|(team, life)| sign(team) * life)
            .sum::<i64>();
        let gold = self.team_gold_delta(hero.team, accumulated_gold);
        let mut minions = 0;
        let mut near_minion = false;
        let mut near_hero = false;
        let mut cover = false;
        let mut exposure = false;
        let range = self.attack_ready_range(hero);
        let positions = self.world.positions(player).collect::<Vec<_>>();
        for &(id, target) in &self.world.locations {
            let Some(entity) = self.world.entity(id) else {
                continue;
            };
            let Some(team) = entity.team else { continue };
            let distance = positions
                .iter()
                .map(|position| i64::from(position.distance(target)))
                .min();
            match entity.kind {
                EntityKind::Minion => {
                    let heavy = entity.minion_kind == Some(MinionKind::Heavy);
                    minions += sign(team) * if heavy { 4 } else { 2 };
                    if team == hero.team {
                        cover |= distance == Some(1)
                            && matches!(
                                entity.minion_kind,
                                Some(MinionKind::Melee | MinionKind::Heavy)
                            );
                    } else {
                        near_minion |= distance.is_some_and(|d| d <= range.unwrap_or(1));
                        exposure |= entity.minion_kind == Some(MinionKind::Ranged)
                            && distance.is_some_and(|d| d <= 2);
                    }
                }
                EntityKind::Hero | EntityKind::Piece if team != hero.team => {
                    near_hero |= distance.is_some_and(|d| d <= range.unwrap_or(1));
                }
                _ => {}
            }
        }
        let setup = if near_minion && range.is_some() {
            0.12
        } else if near_hero {
            0.08
        } else if near_minion {
            0.04
        } else {
            0.0
        };
        let bonus =
            setup + 0.005 * f64::from(u8::from(cover)) - 0.01 * f64::from(u8::from(exposure));
        let material = life as f64 + 0.1 * gold as f64 + 0.25 * minions as f64;
        let value = (material / 5.0).tanh().clamp(-1.0 + 1e-6, 1.0 - 1e-6);
        Ok((value.atanh() + bonus / 5.0).tanh())
    }

    pub(super) fn team_gold_delta(&self, team: Symbol, accumulated_gold: bool) -> i64 {
        self.world
            .heroes
            .iter()
            .map(|row| {
                let spent = if accumulated_gold {
                    // The upgrade from level L to L+1 costs L gold.
                    let level = row.level.clamp(1, 8);
                    level * (level - 1) / 2
                } else {
                    0
                };
                let sign = if row.team == team { 1 } else { -1 };
                sign * (row.gold + spent)
            })
            .sum()
    }

    pub(super) fn attack_ready_range(&self, hero: &HeroRecord) -> Option<i64> {
        hero.hand
            .iter()
            .filter_map(|id| {
                let card = self.world.card(*id)?;
                let def = &self.world.definitions.cards[card.definition];
                let color = def.color.map(|color| self.symbols.name(color));
                (!card.played_this_round && matches!(color, Some("RED" | "GOLD")))
                    .then_some(def.range.filter(|range| *range != 0).unwrap_or(1))
            })
            .max()
    }

    pub(in crate::interpreter) fn search_score(
        &self,
        evaluation: SearchEvaluation,
        team: Symbol,
        player: Symbol,
    ) -> Result<i64> {
        if matches!(evaluation, SearchEvaluation::MaterialPosition)
            || self.world.combat.outcome.is_some()
        {
            return Ok(self.evaluate_resolution(team, player));
        }
        let stall_pressure = self.stall_pressure(evaluation, team);
        let evaluation = if stall_pressure.is_some() {
            SearchEvaluation::LanePressureV3AccumulatedGold
        } else {
            evaluation
        };
        let mut score = self.evaluate_attack_range(
            player,
            matches!(evaluation, SearchEvaluation::LanePressureV3AccumulatedGold),
        )?;
        if !matches!(evaluation, SearchEvaluation::LabAttackRangeV1) {
            let lane_pressure = self.evaluate_lane_pressure(team, evaluation);
            score = (score.clamp(-1.0 + 1e-6, 1.0 - 1e-6).atanh()
                + (lane_pressure + stall_pressure.unwrap_or(0.0)) / 5.0)
                .tanh();
        }
        Ok((score * (WIN - 1) as f64).round() as i64)
    }

    pub(super) fn evaluate_lane_pressure(&self, team: Symbol, evaluation: SearchEvaluation) -> f64 {
        let graph = &self.world.definitions.zones;
        let direction = if self.symbols.name(team) == "RED" {
            1.0
        } else {
            -1.0
        };
        let last_wave = self
            .world
            .waves
            .iter()
            .any(|(_, remaining)| *remaining <= 1);
        let depth_weight = if last_wave { 0.65 } else { 0.35 };
        let timed_push = matches!(
            evaluation,
            SearchEvaluation::LanePressureV4 | SearchEvaluation::LanePressureV5
        );
        let lane_coverage =
            graph.lanes.len() > 1 && matches!(evaluation, SearchEvaluation::LanePressureV5);
        let any_lane_exhausted = timed_push
            && self
                .world
                .waves
                .iter()
                .any(|(_, remaining)| *remaining <= 0);
        let mut score = 0.0;
        for (lane, active) in &self.world.battle_zones {
            let Some((_, ordered)) = graph.lanes.iter().find(|(id, _)| id == lane) else {
                continue;
            };
            let Some(index) = ordered.iter().position(|id| id == active) else {
                continue;
            };
            let Some(zone) = graph.zones.iter().find(|zone| zone.id == *active) else {
                continue;
            };
            let red_depth = index.saturating_sub(1);
            let blue_depth = ordered.len().saturating_sub(2 + index);
            // Each ordinary lane shift changes the depth differential by two.
            let depth = direction * (red_depth as f64 - blue_depth as f64) / 2.0;
            if !timed_push {
                score += depth_weight * direction * (red_depth as f64 - blue_depth as f64) / 2.0;
            }
            let mut closest_readiness = [0.0_f64; 2];
            let mut advancing_readiness = 0.0;
            let mut advancing_heroes = 0;

            let mut minions = [0.0, 0.0];
            for &(id, hex) in &self.world.locations {
                if zone.hexes.contains(&hex)
                    && let Some(entity) = self.world.entity(id)
                    && entity.kind == EntityKind::Minion
                    && let Some(owner) = entity.team
                {
                    minions[usize::from(owner != team)] += 1.0;
                }
            }
            let [friendly, enemy] = minions;
            score += if matches!(evaluation, SearchEvaluation::WaveBreakthrough) {
                // Clearing the last blocker matters more when a friendly wave survives.
                let chance = |own, opposing| (own / (own + 2.0)) / (1.0 + opposing);
                0.60 * (chance(friendly, enemy) - chance(enemy, friendly))
            } else {
                0.30 * (friendly - enemy) / (friendly + enemy + 1.0)
            };
            for hero in &self.world.heroes {
                let advances = timed_push && depth != 0.0 && ((hero.team == team) == (depth > 0.0));
                advancing_heroes += usize::from(advances);
                let positions = self.world.positions(hero.id);
                let distance = if matches!(evaluation, SearchEvaluation::LanePressureV1) {
                    positions
                        .into_iter()
                        .any(|hex| zone.hexes.contains(&hex))
                        .then_some(0)
                } else {
                    positions
                        .filter_map(|hex| {
                            if (matches!(
                                evaluation,
                                SearchEvaluation::LanePressureV3
                                    | SearchEvaluation::LanePressureV3AccumulatedGold
                                    | SearchEvaluation::WaveBreakthrough
                            ) || timed_push)
                                && let Some(distance) = zone
                                    .spawns
                                    .iter()
                                    .filter(|spawn| spawn.team != hero.team)
                                    .map(|spawn| hex.distance(spawn.hex))
                                    .min()
                            {
                                return Some(distance);
                            }
                            if zone.hexes.contains(&hex) {
                                Some(0)
                            } else {
                                zone.hexes.iter().map(|target| hex.distance(*target)).min()
                            }
                        })
                        .min()
                };
                if let Some(distance) = distance {
                    let readiness = 0.20 / (1.0 + f64::from(distance));
                    if lane_coverage {
                        let side = usize::from(hero.team != team);
                        closest_readiness[side] = closest_readiness[side].max(readiness);
                    } else {
                        score += if hero.team == team {
                            readiness
                        } else {
                            -readiness
                        };
                    }
                    if advances {
                        advancing_readiness += 1.0 / (1.0 + f64::from(distance.saturating_sub(1)));
                    }
                }
            }
            if lane_coverage {
                score += closest_readiness[0] - closest_readiness[1];
            }
            if timed_push {
                // Once any lane is exhausted, a push in either lane can
                // trigger the last-push comparison.
                let lane_last_wave = any_lane_exhausted
                    || self
                        .world
                        .waves
                        .iter()
                        .any(|(id, remaining)| id == lane && *remaining <= 1);
                let weight = if lane_last_wave { 0.65 } else { 0.35 };
                let readiness = advancing_readiness / advancing_heroes.max(1) as f64;
                score += weight * depth * (0.25 + 0.75 * readiness);
            }
        }
        score
    }

    pub(super) fn evaluate_resolution(&self, root_team: Symbol, player: Symbol) -> i64 {
        if let Some(outcome) = self.world.combat.outcome {
            let winner = outcome.team.or_else(|| {
                outcome
                    .hero
                    .and_then(|hero| self.world.hero(hero).map(|row| row.team))
            });
            return match winner {
                Some(team) if team == root_team => WIN,
                Some(_) => -WIN,
                None => 0,
            };
        }
        let sign = |team| if team == root_team { 1i128 } else { -1 };
        let life = self
            .world
            .combat
            .team_lives()
            .map(|(team, life)| sign(team) * i128::from(life) * 100)
            .sum::<i128>();
        let heroes = self
            .world
            .heroes
            .iter()
            .map(|hero| {
                sign(hero.team)
                    * (i128::from(hero.level) * 15
                        + i128::from(hero.gold) * 3
                        + hero.hand.len() as i128)
            })
            .sum::<i128>();
        let position = i128::from(self.team_position_score(root_team, player)).clamp(-31, 31);
        ((life + heroes) * 64 + position).clamp(i128::from(-WIN + 1), i128::from(WIN - 1)) as i64
    }

    pub(in crate::interpreter) fn team_position_score(&self, team: Symbol, player: Symbol) -> i64 {
        let red = self.symbols.lookup("RED");
        let gold = self.symbols.lookup("GOLD");
        self.world
            .heroes
            .iter()
            .map(|hero| {
                let sign = if hero.team == team { 1 } else { -1 };
                sign * self.hero_position_score(hero, hero.id == player, red, gold)
            })
            .sum()
    }

    pub(in crate::interpreter) fn hero_position_score(
        &self,
        hero: &HeroRecord,
        inspect_hand: bool,
        red: Option<Symbol>,
        gold: Option<Symbol>,
    ) -> i64 {
        let mut range = 1;
        let mut has_gold_or_red = false;
        for id in hero.hand.iter().filter(|_| inspect_hand) {
            let Some(card) = self.world.card(*id) else {
                continue;
            };
            let definition = &self.world.definitions.cards[card.definition];
            has_gold_or_red |=
                definition.color.is_some() && (definition.color == red || definition.color == gold);
            if definition.ranged
                && (definition.primary == Some(ActionKind::Attack)
                    || definition
                        .secondary
                        .iter()
                        .any(|(kind, _)| *kind == ActionKind::Attack))
            {
                range = range.max(definition.range.unwrap_or(1));
            }
        }
        self.world
            .positions(hero.id)
            .map(|position| {
                let mut pressure = 0;
                let mut support = false;
                let mut exposed = false;
                for &(id, target) in &self.world.locations {
                    let Some(entity) = self.world.entity(id) else {
                        continue;
                    };
                    let distance = i64::from(position.distance(target));
                    let Some(team) = entity.team else {
                        continue;
                    };
                    match entity.kind {
                        EntityKind::Minion if team == hero.team => {
                            support |= distance == 1
                                && matches!(
                                    entity.minion_kind,
                                    Some(MinionKind::Melee | MinionKind::Heavy)
                                );
                        }
                        EntityKind::Minion => {
                            exposed |=
                                distance <= 2 && entity.minion_kind == Some(MinionKind::Ranged);
                            if distance <= range {
                                pressure = pressure.max(if has_gold_or_red { 12 } else { 4 });
                            }
                        }
                        EntityKind::Hero | EntityKind::Piece
                            if team != hero.team && distance <= range =>
                        {
                            pressure = pressure.max(8);
                        }
                        _ => {}
                    }
                }
                pressure + i64::from(support) - i64::from(exposed)
            })
            .max()
            .unwrap_or(0)
    }
}
