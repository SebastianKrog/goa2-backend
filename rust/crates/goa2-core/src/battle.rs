//! Minion battles, simultaneous lane pushes and blocked spawn displacement.
//! Rules operate on native state; wire records exist only at import/export.
use crate::{
    combat::GameOutcome,
    data::{Datum, Symbol, Symbols},
    geometry::Hex,
    world::{EntityKind, MinionKind, NativeWorld},
};
use serde_json::{Value, json};
use std::collections::{HashSet, VecDeque};
#[derive(Clone, Debug)]
pub enum BattleError {
    Rule(String),
    Unsupported(&'static str),
}
type Result<T> = std::result::Result<T, BattleError>;
#[derive(Clone, Debug, Hash)]
pub struct PushSpec {
    pub lane: Symbol,
    pub loser: Symbol,
    pub target: Option<Symbol>,
    pub skip_wave: bool,
}
#[derive(Clone, Debug, Hash)]
pub struct RemovalSpec {
    pub loser: Symbol,
    pub remaining: i64,
    pub zone: Symbol,
}
#[derive(Clone, Debug, Hash)]
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
/// Ordered direct writes and source-origin bindings, distinct from generated
/// GameStep continuations. Arena execution preserves each write's source
/// container and identity semantics.
#[derive(Clone, Debug)]
pub enum BattleMutation {
    ClearPending,
    Wave(Symbol, i64),
    Zone(Symbol, Symbol),
    Remove(Symbol),
    RemoveToken(Symbol),
    Spawn {
        unit: Symbol,
        zone: Symbol,
        source_index: usize,
        hex: Hex,
    },
    BlockedSpawn {
        unit: Symbol,
        zone: Symbol,
        source_index: usize,
        hex: Hex,
    },
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
    fn flip(
        world: &mut NativeWorld,
        lane: Symbol,
        clamp: bool,
        writes: &mut Vec<BattleMutation>,
    ) -> Result<i64> {
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
        writes.push(BattleMutation::Wave(lane, count));
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
        self.resolve_traced(world, s, step, selected, favored)
            .map(|(result, _)| result)
    }
    pub fn resolve_traced(
        &self,
        world: &mut NativeWorld,
        s: &Symbols,
        step: BattleStep,
        selected: Option<&Datum>,
        favored: Symbol,
    ) -> Result<(Resolution, Vec<BattleMutation>)> {
        let mut writes = Vec::new();
        let result = match step {
            BattleStep::Check(lane) => self.check(world, lane, &mut writes),
            BattleStep::Push(spec) => self.push(world, spec, &mut writes),
            BattleStep::Minions(lane) => self.battle(world, lane),
            BattleStep::Casualties(spec) => self.casualties(world, s, spec, selected, &mut writes),
            BattleStep::Displace(items) => {
                self.displace(world, s, items, selected, favored, &mut writes)
            }
        }?;
        Ok((result, writes))
    }
    fn check(
        &self,
        world: &mut NativeWorld,
        lane: Option<Symbol>,
        writes: &mut Vec<BattleMutation>,
    ) -> Result<Resolution> {
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
            Self::flip(world, *lane, true, writes)?;
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
                    Self::wipe(world, zone, writes)?;
                    pending.extend(Self::spawn(world, lane, zone, writes)?);
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
    fn push(
        &self,
        world: &mut NativeWorld,
        p: PushSpec,
        writes: &mut Vec<BattleMutation>,
    ) -> Result<Resolution> {
        if !p.skip_wave && Self::flip(world, p.lane, false, writes)? <= 0 {
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
        Self::wipe(world, current, writes)?;
        world
            .battle_zones
            .iter_mut()
            .find(|(l, _)| *l == p.lane)
            .unwrap()
            .1 = target;
        writes.push(BattleMutation::Zone(p.lane, target));
        let pending = Self::spawn(world, p.lane, target, writes)?;
        Ok(Resolution::Done(if pending.is_empty() {
            vec![]
        } else {
            vec![BattleOp::Step(BattleStep::Displace(pending))]
        }))
    }
    fn wipe(world: &mut NativeWorld, zone: Symbol, writes: &mut Vec<BattleMutation>) -> Result<()> {
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
            writes.push(BattleMutation::Remove(id));
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
    fn spawn(
        world: &mut NativeWorld,
        lane: Symbol,
        zone: Symbol,
        writes: &mut Vec<BattleMutation>,
    ) -> Result<Vec<(Symbol, Hex)>> {
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
                    if let Some(token) = token
                        && world
                            .remove_entity(token)
                            .map_err(BattleError::Rule)?
                            .is_some()
                    {
                        world.remove_token_effects(token);
                        writes.push(BattleMutation::RemoveToken(token));
                    }
                    world.place_entity(id, sp.hex).map_err(BattleError::Rule)?;
                    writes.push(BattleMutation::Spawn {
                        unit: id,
                        zone,
                        source_index: sp.source_index,
                        hex: sp.hex,
                    });
                }
            } else {
                blocked.push(sp);
            }
        }
        for sp in blocked {
            if let Some(id) = Self::candidate(world, lane, &sp, &reserved) {
                reserved.insert(id);
                pending.push((id, sp.hex));
                writes.push(BattleMutation::BlockedSpawn {
                    unit: id,
                    zone,
                    source_index: sp.source_index,
                    hex: sp.hex,
                });
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
            let excluded = self.exclusions(world, &zone.hexes)?;
            count[0] -= excluded[0];
            count[1] -= excluded[1];
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
    // Source order matters when several origins overlap: sum caps by raw source
    // identity first, then reserve adjacent minions for each enemy team. This
    // consumer uses the stored active flag, not duration/scope/immunity rules.
    fn exclusions(&self, world: &NativeWorld, zone: &HashSet<Hex>) -> Result<[i64; 2]> {
        let mut caps: Vec<(Symbol, i64)> = Vec::new();
        for effect in &world.effects.instances {
            if effect.definition.behavior.kind != crate::effect_rules::Kind::BattleExclusion
                || !effect.active
            {
                continue;
            }
            let cap = effect.definition.behavior.maximum.unwrap_or(0);
            if let Some((_, total)) = caps.iter_mut().find(|(id, _)| *id == effect.source) {
                *total = total
                    .checked_add(cap)
                    .ok_or_else(|| BattleError::Rule("battle exclusion cap overflow".into()))?;
            } else {
                caps.push((effect.source, cap));
            }
        }
        let mut excluded: [HashSet<Symbol>; 2] = [HashSet::new(), HashSet::new()];
        let mut reductions = [0, 0];
        for (origin, cap) in caps {
            if cap <= 0 {
                continue;
            }
            let Some((team, position)) = world
                .entity(origin)
                .and_then(|e| e.team)
                .zip(world.position(origin))
            else {
                continue;
            };
            let enemy = if team == self.red { 1 } else { 0 };
            let enemy_team = if enemy == 0 { self.red } else { self.blue };
            let mut adjacent = Vec::new();
            for hex in position.neighbors() {
                if !zone.contains(&hex)
                    || !world
                        .effect_connected(position, hex, &[])
                        .map_err(BattleError::Rule)?
                {
                    continue;
                }
                if let Some(unit) = world.occupant(hex).and_then(|id| world.entity(id))
                    && unit.kind == EntityKind::Minion
                    && unit.team == Some(enemy_team)
                    && !excluded[enemy].contains(&unit.id)
                {
                    adjacent.push(unit.id);
                }
            }
            let take = usize::try_from(cap)
                .unwrap_or(usize::MAX)
                .min(adjacent.len());
            excluded[enemy].extend(adjacent.into_iter().take(take));
            reductions[enemy] += take as i64;
        }
        Ok(reductions)
    }
    fn casualties(
        &self,
        world: &NativeWorld,
        symbols: &Symbols,
        p: RemovalSpec,
        selected: Option<&Datum>,
        writes: &mut Vec<BattleMutation>,
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
        writes.push(BattleMutation::ClearPending);
        let selected_id = selected
            .filter(|value| value.truthy(symbols))
            .and_then(|value| value.python_scalar_text(symbols))
            .and_then(|text| valid.iter().copied().find(|id| symbols.name(*id) == text));
        if let Some(id) = selected_id {
            return Ok(Resolution::Done(vec![
                BattleOp::Remove(id),
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
        writes: &mut Vec<BattleMutation>,
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
            .and_then(|e| {
                e.lane.or_else(|| {
                    matches!(e.kind, EntityKind::Hero | EntityKind::Piece)
                        .then(|| s.lookup("lane_1"))
                        .flatten()
                })
            })
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
            .map(|z| Self::displacement_destinations(world, origin, &z.hexes))
            .transpose()?
            .unwrap_or_default();
        let selected_destination = selected_hex.filter(|h| candidates.contains(h));
        if !candidates.is_empty() && selected_destination.is_none() {
            writes.push(BattleMutation::ClearPending);
        }
        let destination =
            selected_destination.or_else(|| (candidates.len() == 1).then(|| candidates[0]));
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
    // Spawn displacement searches connected neighbors without an actor or an
    // origin-to-each-space constraint. The generated Place step separately
    // validates placement; do not prefilter or alter its mandatory semantics.
    fn displacement_destinations(
        world: &NativeWorld,
        start: Hex,
        zone: &HashSet<Hex>,
    ) -> Result<Vec<Hex>> {
        let mut queue = VecDeque::from([(start, 0)]);
        let mut visited = HashSet::from([start]);
        let mut found = None;
        let mut candidates = Vec::new();
        while let Some((hex, distance)) = queue.pop_front() {
            if found.is_some_and(|d| distance > d) {
                break;
            }
            if distance > 0 && zone.contains(&hex) && world.on_map(hex) && !world.obstacle(hex) {
                candidates.push(hex);
                found = Some(distance);
            }
            if found.is_none() {
                for next in hex.neighbors() {
                    if world.on_map(next)
                        && !visited.contains(&next)
                        && world
                            .effect_connected(hex, next, &[])
                            .map_err(BattleError::Rule)?
                    {
                        visited.insert(next);
                        queue.push_back((next, distance + 1));
                    }
                }
            }
        }
        Ok(candidates)
    }
}
