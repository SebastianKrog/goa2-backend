//! Minion battles, simultaneous lane pushes and blocked spawn displacement.
//! Rules operate on native state; wire records exist only at import/export.
use crate::{
    combat::GameOutcome,
    data::{Datum, Symbol, Symbols},
    geometry::Hex,
    world::{EntityKind, MinionKind, NativeWorld},
};
use serde_json::{Value, json};
use std::collections::HashSet;
#[derive(Clone, Debug)]
pub enum BattleError {
    Rule(String),
    Unsupported(&'static str),
}
type Result<T> = std::result::Result<T, BattleError>;
#[derive(Clone, Debug)]
pub struct PushSpec {
    pub lane: Symbol,
    pub loser: Symbol,
    pub target: Option<Symbol>,
    pub skip_wave: bool,
}
#[derive(Clone, Debug)]
pub struct RemovalSpec {
    pub loser: Symbol,
    pub remaining: i64,
    pub zone: Symbol,
}
#[derive(Clone, Debug)]
pub enum BattleStep {
    Check(Option<Symbol>),
    Push(PushSpec),
    Minions(Option<Symbol>),
    Casualties(RemovalSpec),
    Displace(Vec<(Symbol, Hex)>),
}
impl BattleStep {
    pub fn patch_wire(&self, wire: &mut Value, symbols: &Symbols) {
        match self {
            Self::Check(lane) | Self::Minions(lane) => {
                wire["lane_id"] = json!(lane.map(|id| symbols.name(id)))
            }
            Self::Push(p) => {
                wire["lane_id"] = json!(symbols.name(p.lane));
                wire["losing_team"] = json!(symbols.name(p.loser));
                wire["target_zone_id"] = json!(p.target.map(|id| symbols.name(id)));
                wire["skip_wave_counter"] = json!(p.skip_wave);
            }
            Self::Casualties(p) => {
                wire["losing_team"] = json!(symbols.name(p.loser));
                wire["remaining_to_remove"] = json!(p.remaining);
                wire["zone_id"] = json!(symbols.name(p.zone));
            }
            Self::Displace(d) => {
                wire["displacements"] = json!(
                    d.iter()
                        .map(|(u, h)| (symbols.name(*u), h))
                        .collect::<Vec<_>>()
                )
            }
        }
    }
}
#[derive(Clone, Debug)]
pub enum BattleOp {
    Step(BattleStep),
    Remove(Symbol),
    Place(Symbol, Hex),
    GameOver(GameOutcome),
}
#[derive(Clone, Debug)]
pub enum Choices {
    Units(Vec<Symbol>),
    Hexes(Vec<Hex>),
}
#[derive(Clone, Debug)]
pub struct Prompt {
    pub owner: Symbol,
    pub prompt: Symbol,
    pub choices: Choices,
    pub unit: Option<Symbol>,
    pub clear_pending: bool,
}
#[derive(Clone, Debug)]
pub enum Resolution {
    Done(Vec<BattleOp>),
    Prompt(Prompt),
}
#[derive(Clone, Debug)]
pub struct BattleRules {
    red: Symbol,
    blue: Symbol,
    last_push: Symbol,
    lane_push: Symbol,
    one_man: Symbol,
    owners: [Symbol; 2],
    blocked: [Symbol; 2],
    duplicated: [Symbol; 2],
    removal: [Vec<Symbol>; 2],
    displacement: Vec<(Symbol, Symbol)>,
}
impl BattleRules {
    pub fn compile(world: &NativeWorld, s: &mut Symbols) -> Self {
        let red = s.intern("RED");
        let blue = s.intern("BLUE");
        let teams = [red, blue];
        let count = world
            .definitions
            .entities
            .iter()
            .filter(|e| e.kind == EntityKind::Minion)
            .count();
        Self {
            red,
            blue,
            last_push: s.intern("LAST_PUSH"),
            lane_push: s.intern("LANE_PUSH"),
            one_man: s.intern("one_man_army"),
            owners: teams.map(|t| s.intern(&format!("team:{}", s.name(t)))),
            blocked: teams.map(|t| {
                s.intern(&format!(
                    "Team {}, choose which blocked spawn point to resolve first.",
                    s.name(t)
                ))
            }),
            duplicated: teams.map(|t| {
                s.intern(&format!(
                    "Team {}, choose which displaced unit to place first.",
                    s.name(t)
                ))
            }),
            removal: teams.map(|t| {
                (0..=count)
                    .map(|n| {
                        s.intern(&format!(
                            "Team {}, choose a minion to remove ({n} remaining).",
                            s.name(t)
                        ))
                    })
                    .collect()
            }),
            displacement: world
                .definitions
                .entities
                .iter()
                .filter(|e| {
                    matches!(
                        e.kind,
                        EntityKind::Hero | EntityKind::Minion | EntityKind::Piece
                    )
                })
                .filter_map(|e| e.team.map(|team| (e.id, team, e.name)))
                .map(|(id, team, name)| {
                    (
                        id,
                        s.intern(&format!(
                            "Team {}, choose displacement for {}.",
                            s.name(team),
                            s.name(name)
                        )),
                    )
                })
                .collect(),
        }
    }
    fn team(&self, t: Symbol) -> Result<usize> {
        if t == self.red {
            Ok(0)
        } else if t == self.blue {
            Ok(1)
        } else {
            Err(BattleError::Rule("invalid minion battle team".into()))
        }
    }
    fn zone(world: &NativeWorld, lane: Symbol) -> Option<Symbol> {
        world
            .battle_zones
            .iter()
            .find(|(l, _)| *l == lane)
            .map(|(_, z)| *z)
            .filter(|z| *z != world.definitions.zones.empty)
    }
    fn lanes(world: &NativeWorld, lane: Option<Symbol>) -> Vec<Symbol> {
        lane.map_or_else(
            || world.battle_zones.iter().map(|(l, _)| *l).collect(),
            |l| vec![l],
        )
    }
    fn target(&self, world: &NativeWorld, lane: Symbol, loser: Symbol) -> (Option<Symbol>, bool) {
        let Some(zone) = Self::zone(world, lane) else {
            return (None, false);
        };
        let Some((_, zones)) = world
            .definitions
            .zones
            .lanes
            .iter()
            .find(|(l, _)| *l == lane)
        else {
            return (None, false);
        };
        let Some(index) = zones.iter().position(|z| *z == zone) else {
            return (None, false);
        };
        if loser == self.red {
            if index <= 1 {
                (None, true)
            } else {
                (Some(zones[index - 1]), false)
            }
        } else if index + 1 >= zones.len().saturating_sub(1) {
            (None, true)
        } else {
            (Some(zones[index + 1]), false)
        }
    }
    fn flip(world: &mut NativeWorld, lane: Symbol, clamp: bool) -> Result<i64> {
        let index = world.waves.iter().position(|(l, _)| *l == lane);
        let count = index
            .map_or(0, |i| world.waves[i].1)
            .checked_sub(1)
            .ok_or_else(|| BattleError::Rule("wave counter overflow".into()))?;
        let count = if clamp { count.max(0) } else { count };
        if let Some(i) = index {
            world.waves[i].1 = count
        } else {
            world.waves.push((lane, count));
        }
        Ok(count)
    }
    fn victory(&self, loser: Symbol, condition: Symbol) -> BattleOp {
        BattleOp::GameOver(GameOutcome {
            team: Some(if loser == self.red {
                self.blue
            } else {
                self.red
            }),
            hero: None,
            condition,
        })
    }
    pub fn resolve(
        &self,
        world: &mut NativeWorld,
        s: &Symbols,
        step: BattleStep,
        selected: Option<&Datum>,
        favored: Symbol,
    ) -> Result<Resolution> {
        if world.has_effects {
            return Err(BattleError::Unsupported(
                "battle effect/exclusion closure not compiled",
            ));
        }
        match step {
            BattleStep::Check(lane) => self.check(world, lane),
            BattleStep::Push(spec) => self.push(world, spec),
            BattleStep::Minions(lane) => self.battle(world, lane),
            BattleStep::Casualties(spec) => self.casualties(world, spec, selected),
            BattleStep::Displace(items) => self.displace(world, s, items, selected, favored),
        }
    }
    fn check(&self, world: &mut NativeWorld, lane: Option<Symbol>) -> Result<Resolution> {
        let mut triggered = Vec::new();
        for lane in Self::lanes(world, lane) {
            let Some(zid) = Self::zone(world, lane) else {
                continue;
            };
            let Some(zone) = world.definitions.zones.zones.iter().find(|z| z.id == zid) else {
                continue;
            };
            let mut counts = [0, 0];
            // Push thresholds use roster membership, not each minion's team field.
            for e in world
                .definitions
                .entities
                .iter()
                .filter(|e| e.kind == EntityKind::Minion)
            {
                if world
                    .position(e.id)
                    .is_some_and(|h| zone.hexes.contains(&h))
                {
                    if e.roster == Some(self.red) {
                        counts[0] += 1
                    } else if e.roster == Some(self.blue) {
                        counts[1] += 1
                    }
                }
            }
            let loser = if counts[0] == 0 && counts[1] > 0 {
                self.red
            } else if counts[1] == 0 && counts[0] > 0 {
                self.blue
            } else {
                continue;
            };
            triggered.push((lane, loser));
        }
        if world.definitions.zones.lanes.len() <= 1 {
            return Ok(Resolution::Done(
                triggered
                    .into_iter()
                    .map(|(lane, loser)| {
                        BattleOp::Step(BattleStep::Push(PushSpec {
                            lane,
                            loser,
                            target: None,
                            skip_wave: false,
                        }))
                    })
                    .collect(),
            ));
        }
        if triggered.is_empty() {
            return Ok(Resolution::Done(vec![]));
        }
        let outcomes = triggered
            .into_iter()
            .map(|(l, t)| {
                let (z, throne) = self.target(world, l, t);
                (l, t, z, throne)
            })
            .collect::<Vec<_>>();
        for (lane, _, _, _) in &outcomes {
            Self::flip(world, *lane, true)?;
        }
        let red = outcomes
            .iter()
            .any(|(_, loser, _, throne)| *throne && *loser == self.blue);
        let blue = outcomes
            .iter()
            .any(|(_, loser, _, throne)| *throne && *loser == self.red);
        if red != blue {
            return Ok(Resolution::Done(vec![
                self.victory(if red { self.blue } else { self.red }, self.lane_push),
            ]));
        }
        let mut remedy = red && blue;
        if !remedy && world.waves.iter().any(|(_, n)| *n <= 0) {
            let mut totals = [0usize, 0usize];
            for (lane, current) in &world.battle_zones {
                let zone = outcomes
                    .iter()
                    .find(|(l, _, _, _)| l == lane)
                    .and_then(|(_, _, z, _)| *z)
                    .unwrap_or(*current);
                if let Some((_, zones)) = world
                    .definitions
                    .zones
                    .lanes
                    .iter()
                    .find(|(l, _)| l == lane)
                    && let Some(index) = zones.iter().position(|z| *z == zone)
                {
                    totals[0] += index.saturating_sub(1);
                    totals[1] += zones.len().saturating_sub(2 + index);
                }
            }
            if totals[0] != totals[1] {
                return Ok(Resolution::Done(vec![self.victory(
                    if totals[0] > totals[1] {
                        self.blue
                    } else {
                        self.red
                    },
                    self.last_push,
                )]));
            }
            remedy = true;
        }
        if remedy {
            let mut pending = Vec::new();
            for (lane, _, _, _) in outcomes {
                if let Some(zone) = Self::zone(world, lane) {
                    Self::wipe(world, zone)?;
                    pending.extend(Self::spawn(world, lane, zone)?);
                }
            }
            return Ok(Resolution::Done(if pending.is_empty() {
                vec![]
            } else {
                vec![BattleOp::Step(BattleStep::Displace(pending))]
            }));
        }
        Ok(Resolution::Done(
            outcomes
                .into_iter()
                .filter_map(|(lane, loser, target, _)| {
                    target.map(|t| {
                        BattleOp::Step(BattleStep::Push(PushSpec {
                            lane,
                            loser,
                            target: Some(t),
                            skip_wave: true,
                        }))
                    })
                })
                .collect(),
        ))
    }
    fn push(&self, world: &mut NativeWorld, p: PushSpec) -> Result<Resolution> {
        if !p.skip_wave && Self::flip(world, p.lane, false)? <= 0 {
            return Ok(Resolution::Done(vec![
                self.victory(p.loser, self.last_push),
            ]));
        }
        let target = if p.target.is_some() {
            p.target
        } else {
            let (t, throne) = self.target(world, p.lane, p.loser);
            if throne {
                return Ok(Resolution::Done(vec![
                    self.victory(p.loser, self.lane_push),
                ]));
            }
            t
        };
        let Some(target) = target.filter(|id| *id != world.definitions.zones.empty) else {
            return Ok(Resolution::Done(vec![]));
        };
        let Some(current) = Self::zone(world, p.lane) else {
            return Ok(Resolution::Done(vec![]));
        };
        Self::wipe(world, current)?;
        world
            .battle_zones
            .iter_mut()
            .find(|(l, _)| *l == p.lane)
            .unwrap()
            .1 = target;
        let pending = Self::spawn(world, p.lane, target)?;
        Ok(Resolution::Done(if pending.is_empty() {
            vec![]
        } else {
            vec![BattleOp::Step(BattleStep::Displace(pending))]
        }))
    }
    fn wipe(world: &mut NativeWorld, zone: Symbol) -> Result<()> {
        let Some(z) = world.definitions.zones.zones.iter().find(|z| z.id == zone) else {
            return Ok(());
        };
        let ids = world
            .locations
            .iter()
            .filter_map(|(id, h)| {
                (z.hexes.contains(h)
                    && world
                        .entity(*id)
                        .is_some_and(|e| e.kind == EntityKind::Minion))
                .then_some(*id)
            })
            .collect::<Vec<_>>();
        for id in ids {
            world.remove_entity(id).map_err(BattleError::Rule)?;
        }
        Ok(())
    }
    fn candidate(
        world: &NativeWorld,
        lane: Symbol,
        sp: &crate::zones::Spawn,
        reserved: &HashSet<Symbol>,
    ) -> Option<Symbol> {
        world
            .definitions
            .entities
            .iter()
            .find(|e| {
                e.kind == EntityKind::Minion
                    && e.roster == Some(sp.team)
                    && e.lane == Some(lane)
                    && e.minion_kind == Some(sp.kind)
                    && world.position(e.id).is_none()
                    && !reserved.contains(&e.id)
            })
            .map(|e| e.id)
    }
    fn spawn(world: &mut NativeWorld, lane: Symbol, zone: Symbol) -> Result<Vec<(Symbol, Hex)>> {
        let Some(z) = world.definitions.zones.zones.iter().find(|z| z.id == zone) else {
            return Ok(vec![]);
        };
        let spawns = z.spawns.clone();
        let mut blocked = Vec::new();
        let mut pending = Vec::new();
        let mut reserved = HashSet::new();
        for sp in spawns {
            let occupant = world.occupant(sp.hex);
            let token = occupant.filter(|id| {
                world
                    .entity(*id)
                    .is_some_and(|e| e.kind == EntityKind::Token)
            });
            if (world.on_map(sp.hex) && occupant.is_none()) || token.is_some() {
                if let Some(id) = Self::candidate(world, lane, &sp, &reserved) {
                    if let Some(token) = token {
                        world.remove_entity(token).map_err(BattleError::Rule)?;
                    }
                    world.place_entity(id, sp.hex).map_err(BattleError::Rule)?;
                }
            } else {
                blocked.push(sp);
            }
        }
        for sp in blocked {
            if let Some(id) = Self::candidate(world, lane, &sp, &reserved) {
                reserved.insert(id);
                pending.push((id, sp.hex));
            }
        }
        Ok(pending)
    }
    fn battle(&self, world: &NativeWorld, lane: Option<Symbol>) -> Result<Resolution> {
        let mut ops = Vec::new();
        for lane in Self::lanes(world, lane) {
            let Some(id) = Self::zone(world, lane) else {
                continue;
            };
            let Some(zone) = world.definitions.zones.zones.iter().find(|z| z.id == id) else {
                continue;
            };
            let mut count = [0i64, 0i64];
            for (uid, h) in &world.locations {
                if zone.hexes.contains(h)
                    && let Some(e) = world.entity(*uid).filter(|e| e.kind == EntityKind::Minion)
                    && let Some(team) = e.team
                {
                    count[self.team(team)?] += 1;
                }
            }
            for hero in &world.heroes {
                if hero.level >= 8
                    && hero
                        .ultimate
                        .and_then(|id| world.card(id))
                        .is_some_and(|c| {
                            world.definitions.cards[c.definition].effect == self.one_man
                        })
                    && world
                        .position(world.resolved_actor(hero.id))
                        .is_some_and(|h| zone.hexes.contains(&h))
                    && let Some(team) = world.entity(hero.id).and_then(|e| e.team)
                {
                    count[self.team(team)?] += 1;
                }
            }
            if count[0] != count[1] {
                ops.push(BattleOp::Step(BattleStep::Casualties(RemovalSpec {
                    loser: if count[0] < count[1] {
                        self.red
                    } else {
                        self.blue
                    },
                    remaining: (count[0] - count[1]).abs(),
                    zone: id,
                })));
            }
        }
        Ok(Resolution::Done(ops))
    }
    fn casualties(
        &self,
        world: &NativeWorld,
        p: RemovalSpec,
        selected: Option<&Datum>,
    ) -> Result<Resolution> {
        if p.remaining <= 0 {
            return Ok(Resolution::Done(vec![]));
        }
        let Some(zone) = world
            .definitions
            .zones
            .zones
            .iter()
            .find(|z| z.id == p.zone)
        else {
            return Ok(Resolution::Done(vec![]));
        };
        let team = self.team(p.loser)?;
        let minions = world
            .locations
            .iter()
            .filter_map(|(id, h)| {
                world.entity(*id).filter(|e| {
                    zone.hexes.contains(h)
                        && e.kind == EntityKind::Minion
                        && e.team == Some(p.loser)
                })
            })
            .collect::<Vec<_>>();
        if minions.is_empty() {
            return Ok(Resolution::Done(vec![]));
        }
        let ordinary = minions
            .iter()
            .any(|m| m.minion_kind != Some(MinionKind::Heavy));
        let valid = minions
            .into_iter()
            .filter(|m| !ordinary || m.minion_kind != Some(MinionKind::Heavy))
            .map(|m| m.id)
            .collect::<Vec<_>>();
        let mut ops = Vec::new();
        if valid.len() == 1 || p.remaining >= valid.len() as i64 {
            let n = usize::try_from(p.remaining)
                .unwrap_or(usize::MAX)
                .min(valid.len());
            ops.extend(valid[..n].iter().copied().map(BattleOp::Remove));
            let left = p.remaining - n as i64;
            if left > 0 {
                ops.push(BattleOp::Step(BattleStep::Casualties(RemovalSpec {
                    remaining: left,
                    ..p
                })));
            }
            return Ok(Resolution::Done(ops));
        }
        if let Some(Datum::Text(id)) = selected
            && valid.contains(id)
        {
            return Ok(Resolution::Done(vec![
                BattleOp::Remove(*id),
                BattleOp::Step(BattleStep::Casualties(RemovalSpec {
                    remaining: p.remaining - 1,
                    ..p
                })),
            ]));
        }
        let prompt = *self.removal[team]
            .get(p.remaining as usize)
            .ok_or(BattleError::Unsupported("minion removal prompt capacity"))?;
        Ok(Resolution::Prompt(Prompt {
            owner: self.owners[team],
            prompt,
            choices: Choices::Units(valid),
            unit: None,
            clear_pending: true,
        }))
    }
    fn displace(
        &self,
        world: &NativeWorld,
        s: &Symbols,
        items: Vec<(Symbol, Hex)>,
        selected: Option<&Datum>,
        favored: Symbol,
    ) -> Result<Resolution> {
        let mut groups = [Vec::new(), Vec::new()];
        for (id, h) in items {
            if let Some(e) = world.entity(id).filter(|e| {
                matches!(
                    e.kind,
                    EntityKind::Hero | EntityKind::Minion | EntityKind::Piece
                )
            }) {
                if e.team == Some(self.red) {
                    groups[0].push((id, h));
                } else if e.team == Some(self.blue) {
                    groups[1].push((id, h));
                }
            }
        }
        let favored = if favored == self.red { 0 } else { 1 };
        let team = if groups[favored].is_empty() {
            1 - favored
        } else {
            favored
        };
        let active = &groups[team];
        if active.is_empty() {
            return Ok(Resolution::Done(vec![]));
        }
        let selected_hex = selected.and_then(|v| {
            v.hex(
                [
                    s.lookup("q").unwrap(),
                    s.lookup("r").unwrap(),
                    s.lookup("s").unwrap(),
                ],
                s,
            )
            .ok()
        });
        if active.len() > 1 {
            if let Some(index) = active
                .iter()
                .position(|(id, h)| selected_hex == Some(*h) || selected == Some(&Datum::Text(*id)))
            {
                let chosen = active[index];
                let rest = active
                    .iter()
                    .enumerate()
                    .filter_map(|(i, x)| (i != index).then_some(*x))
                    .chain(groups[1 - team].iter().copied())
                    .collect();
                return Ok(Resolution::Done(vec![
                    BattleOp::Step(BattleStep::Displace(vec![chosen])),
                    BattleOp::Step(BattleStep::Displace(rest)),
                ]));
            }
            let origins = active.iter().map(|(_, h)| *h).collect::<Vec<_>>();
            let unique = origins.iter().copied().collect::<HashSet<_>>().len() == origins.len();
            return Ok(Resolution::Prompt(Prompt {
                owner: self.owners[team],
                prompt: if unique {
                    self.blocked[team]
                } else {
                    self.duplicated[team]
                },
                choices: if unique {
                    Choices::Hexes(origins)
                } else {
                    Choices::Units(active.iter().map(|(id, _)| *id).collect())
                },
                unit: None,
                clear_pending: false,
            }));
        }
        let (unit, origin) = active[0];
        let rest = groups[1 - team].clone();
        let Some(zone) = world
            .entity(unit)
            .and_then(|e| e.lane)
            .and_then(|l| Self::zone(world, l))
        else {
            return Ok(Resolution::Done(vec![]));
        };
        let candidates = world
            .definitions
            .zones
            .zones
            .iter()
            .find(|z| z.id == zone)
            .map_or_else(Vec::new, |z| {
                crate::minion_return::nearest(world, origin, &z.hexes, false).0
            });
        let destination = selected_hex
            .filter(|h| candidates.contains(h))
            .or_else(|| (candidates.len() == 1).then(|| candidates[0]));
        if candidates.is_empty() || destination.is_some() {
            let mut ops = Vec::new();
            if let Some(hex) = destination {
                ops.push(BattleOp::Place(unit, hex));
            }
            ops.push(BattleOp::Step(BattleStep::Displace(rest)));
            return Ok(Resolution::Done(ops));
        }
        let prompt = self
            .displacement
            .iter()
            .find(|(id, _)| *id == unit)
            .map(|(_, p)| *p)
            .ok_or(BattleError::Unsupported("displacement prompt missing"))?;
        Ok(Resolution::Prompt(Prompt {
            owner: self.owners[team],
            prompt,
            choices: Choices::Hexes(candidates),
            unit: Some(unit),
            clear_pending: true,
        }))
    }
}
