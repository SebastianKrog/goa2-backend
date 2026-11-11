use super::{NativeSession, SearchEvaluation, Symbol};
use crate::world::EntityKind;

impl NativeSession {
    pub(super) fn stall_pressure(&self, evaluation: SearchEvaluation, team: Symbol) -> Option<f64> {
        use SearchEvaluation::*;
        Some(match evaluation {
            StallLifePressure => {
                self.world
                    .combat
                    .team_lives()
                    .map(|(owner, lives)| side_sign(owner, team) * lives as f64)
                    .sum::<f64>()
                    * 0.55
            }
            StallGoldDiscount => -0.065 * self.team_gold_delta(team, true) as f64,
            StallHeroThreat | StallMinionApproach | StallCrossfire | StallAttackReady => {
                self.stall_engagement(evaluation, team)
            }
            StallLaneAdvance | StallWaveClear | StallNextWave | StallLanePresence => {
                self.stall_lanes(evaluation, team)
            }
            _ => return None,
        })
    }

    fn stall_engagement(&self, evaluation: SearchEvaluation, team: Symbol) -> f64 {
        use SearchEvaluation::*;
        let targets: Vec<_> = self
            .world
            .locations
            .iter()
            .filter_map(|&(id, hex)| {
                let entity = self.world.entity(id)?;
                let eligible = if matches!(evaluation, StallHeroThreat) {
                    matches!(entity.kind, EntityKind::Hero | EntityKind::Piece)
                } else {
                    entity.kind == EntityKind::Minion
                        && self.world.battle_zones.iter().any(|(_, active)| {
                            self.world
                                .definitions
                                .zones
                                .zones
                                .iter()
                                .any(|zone| zone.id == *active && zone.hexes.contains(&hex))
                        })
                };
                eligible.then_some((entity.team?, hex))
            })
            .collect();

        if matches!(evaluation, StallCrossfire) {
            // Use the second-closest attacker on the same target. A lone
            // pursuer cannot collect the team coordination bonus.
            let mut best = [0.0_f64; 2];
            for &(owner, target) in &targets {
                let mut closest = [0.0_f64; 2];
                for hero in self.world.heroes.iter().filter(|hero| hero.team != owner) {
                    let distance = self
                        .world
                        .positions(hero.id)
                        .map(|hex| hex.distance(target))
                        .min();
                    let value = proximity(distance, 1);
                    if value >= closest[0] {
                        closest = [value, closest[0]];
                    } else {
                        closest[1] = closest[1].max(value);
                    }
                }
                let side = usize::from(owner == team);
                best[side] = best[side].max(closest[1]);
            }
            return 0.65 * (best[0] - best[1]);
        }

        self.world
            .heroes
            .iter()
            .map(|hero| {
                let range = match evaluation {
                    StallAttackReady => {
                        let Some(range) = self.attack_ready_range(hero) else {
                            return 0.0;
                        };
                        range.max(1) as u32
                    }
                    StallHeroThreat => self.attack_ready_range(hero).unwrap_or(1).max(1) as u32,
                    _ => 1,
                };
                let distance = self
                    .world
                    .positions(hero.id)
                    .flat_map(|hex| {
                        targets
                            .iter()
                            .filter(|(owner, _)| *owner != hero.team)
                            .map(move |(_, target)| hex.distance(*target))
                    })
                    .min();
                let value = if matches!(evaluation, StallAttackReady) {
                    f64::from(u8::from(distance.is_some_and(|distance| distance <= range))) * 0.48
                } else {
                    proximity(distance, range) * 0.34
                };
                side_sign(hero.team, team) * value
            })
            .sum()
    }

    fn stall_lanes(&self, evaluation: SearchEvaluation, team: Symbol) -> f64 {
        use SearchEvaluation::*;
        let graph = &self.world.definitions.zones;
        let root_is_red = self.symbols.name(team) == "RED";
        self.world
            .battle_zones
            .iter()
            .filter_map(|(lane, active)| {
                let ordered = &graph.lanes.iter().find(|(id, _)| id == lane)?.1;
                let index = ordered.iter().position(|id| id == active)?;
                let zone = graph.zones.iter().find(|zone| zone.id == *active)?;
                if matches!(evaluation, StallLaneAdvance) {
                    let red = index.saturating_sub(1);
                    let blue = ordered.len().saturating_sub(2 + index);
                    let depth = (red as f64 - blue as f64) / 2.0;
                    return Some(0.50 * if root_is_red { depth } else { -depth });
                }
                let mut counts = [0.0_f64; 2];
                for &(id, hex) in &self.world.locations {
                    if zone.hexes.contains(&hex)
                        && let Some(entity) = self.world.entity(id)
                        && entity.kind == EntityKind::Minion
                        && let Some(owner) = entity.team
                    {
                        counts[usize::from(owner != team)] += 1.0;
                    }
                }
                let value = |side: usize| {
                    let friendly = counts[side];
                    let opposing = counts[1 - side];
                    let heroes = self
                        .world
                        .heroes
                        .iter()
                        .filter(|hero| usize::from(hero.team != team) == side);
                    match evaluation {
                        StallWaveClear => 0.65 * (friendly / (friendly + 2.0)) / (1.0 + opposing),
                        StallLanePresence if opposing > 0.0 => {
                            let present = heroes
                                .filter(|hero| {
                                    self.world
                                        .positions(hero.id)
                                        .any(|hex| zone.hexes.contains(&hex))
                                })
                                .count() as f64;
                            0.17 * present * present.min(2.0)
                        }
                        StallNextWave if friendly > opposing => {
                            let advancing_red = root_is_red == (side == 0);
                            let next = if advancing_red {
                                index.checked_add(1)
                            } else {
                                index.checked_sub(1)
                            };
                            let Some(next_zone) = next
                                .and_then(|next| ordered.get(next))
                                .and_then(|id| graph.zones.iter().find(|zone| zone.id == *id))
                            else {
                                return 0.0;
                            };
                            let distance = heroes
                                .flat_map(|hero| {
                                    self.world.positions(hero.id).flat_map(move |hex| {
                                        next_zone
                                            .spawns
                                            .iter()
                                            .filter(move |spawn| spawn.team != hero.team)
                                            .map(move |spawn| hex.distance(spawn.hex))
                                    })
                                })
                                .min();
                            0.55 * proximity(distance, 1) * (friendly - opposing) / (1.0 + friendly)
                        }
                        _ => 0.0,
                    }
                };
                Some(value(0) - value(1))
            })
            .sum()
    }
}

fn side_sign(owner: Symbol, team: Symbol) -> f64 {
    if owner == team { 1.0 } else { -1.0 }
}

fn proximity(distance: Option<u32>, range: u32) -> f64 {
    distance.map_or(0.0, |distance| {
        1.0 / (1.0 + f64::from(distance.saturating_sub(range)))
    })
}
