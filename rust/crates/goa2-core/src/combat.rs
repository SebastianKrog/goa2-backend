//! Combat and defeat consequences, including effect expiry, minion saves and bounties.
//! Minion defense includes actor-bound illusion equivalence.
use crate::{
    cards::CardError,
    data::{Datum, RegisterWrite, Registers, Symbol, Symbols},
    world::{EntityKind, MinionKind, NativeWorld},
};
use serde_json::{Value, json};
#[derive(Clone, Debug)]
pub enum CombatError {
    Rule(String),
    Unsupported(String),
}
fn rule(s: &str) -> CombatError {
    CombatError::Rule(s.into())
}
fn unsupported(s: &str) -> CombatError {
    CombatError::Unsupported(s.into())
}
type Result<T> = std::result::Result<T, CombatError>;
#[derive(Clone, Copy, Debug)]
pub struct DefeatBinding {
    pub victim: Symbol,
    pub killer: Option<Symbol>,
}
#[derive(Clone, Copy, Debug)]
pub struct GameOutcome {
    pub team: Option<Symbol>,
    pub hero: Option<Symbol>,
    pub condition: Symbol,
}
#[derive(Clone, Debug)]
pub struct TeamCounter {
    key: Symbol,
    life: i64,
}
#[derive(Clone, Debug)]
pub struct CombatState {
    pub(crate) round: i64,
    pub(crate) turn: i64,
    teams: Vec<TeamCounter>,
    pub(crate) unresolved: Vec<Symbol>,
    pub(crate) defeated: Vec<Symbol>,
    pub outcome: Option<GameOutcome>,
}
impl CombatState {
    pub fn compile(state: &Value, symbols: &mut Symbols) -> std::result::Result<Self, String> {
        let teams = state["teams"]
            .as_object()
            .ok_or("teams missing")?
            .iter()
            .map(|(key, team)| {
                Ok(TeamCounter {
                    key: symbols.intern(key),
                    life: team["life_counters"]
                        .as_i64()
                        .ok_or("life counters missing")?,
                })
            })
            .collect::<std::result::Result<Vec<_>, String>>()?;
        let mut ids = |key: &str| -> std::result::Result<Vec<Symbol>, String> {
            state[key]
                .as_array()
                .ok_or_else(|| format!("missing {key}"))?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(|s| symbols.intern(s))
                        .ok_or("non-string hero identity".into())
                })
                .collect()
        };
        Ok(Self {
            turn: state["turn"].as_i64().ok_or("turn missing")?,
            round: state["round"].as_i64().ok_or("round missing")?,
            teams,
            unresolved: ids("unresolved_hero_ids")?,
            defeated: ids("heroes_defeated_this_round")?,
            outcome: None,
        })
    }
    pub(crate) fn team_lives(&self) -> impl Iterator<Item = (Symbol, i64)> + '_ {
        self.teams.iter().map(|row| (row.key, row.life))
    }
    pub(crate) fn set_team_life(
        &mut self,
        team: Symbol,
        life: i64,
    ) -> std::result::Result<(), String> {
        self.teams
            .iter_mut()
            .find(|row| row.key == team)
            .ok_or("combat team missing")?
            .life = life;
        Ok(())
    }
    pub fn export(&self, state: &mut Value, symbols: &Symbols) {
        state["turn"] = json!(self.turn);
        for team in &self.teams {
            state["teams"][symbols.name(team.key)]["life_counters"] = json!(team.life);
        }
        state["unresolved_hero_ids"] = json!(
            self.unresolved
                .iter()
                .map(|id| symbols.name(*id))
                .collect::<Vec<_>>()
        );
        state["heroes_defeated_this_round"] = json!(
            self.defeated
                .iter()
                .map(|id| symbols.name(*id))
                .collect::<Vec<_>>()
        );
        if let Some(outcome) = self.outcome {
            state["winner"] = json!(outcome.team.map(|s| symbols.name(s)));
            state["individual_winner_id"] = json!(outcome.hero.map(|s| symbols.name(s)));
            state["victory_condition"] = json!(symbols.name(outcome.condition));
            state["input_stack"] = json!([]);
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub enum GoldReason {
    Kill,
    Assist,
    MinionKill,
}
#[derive(Clone, Debug)]
pub enum CombatEvent {
    Resolved {
        actor: Option<Symbol>,
        target: Symbol,
        attack: i64,
        defense: Option<Datum>,
        modifier: Datum,
        blocked: bool,
    },
    Defeated {
        actor: Option<Symbol>,
        target: Symbol,
    },
    Gold {
        hero: Symbol,
        amount: i64,
        reason: GoldReason,
    },
    Life {
        team: Symbol,
        change: i64,
        remaining: i64,
    },
    Removed {
        target: Symbol,
        from: Option<crate::geometry::Hex>,
    },
    Bounty {
        hero: Symbol,
        minion: Symbol,
    },
    Protected {
        hero: Symbol,
        minion: Symbol,
        effect: Symbol,
        card: Option<Symbol>,
        token: Option<Symbol>,
    },
    GameOver(GameOutcome),
}
impl CombatEvent {
    /// Diagnostics/transport boundary only.
    pub fn to_json(&self, symbols: &Symbols) -> Value {
        let (kind, actor, target, from, metadata) = match self {
            Self::Resolved {
                actor,
                target,
                attack,
                defense,
                modifier,
                blocked,
            } => (
                "COMBAT_RESOLVED",
                *actor,
                Some(*target),
                None,
                json!({"attack_value":attack,"defense_value":defense.as_ref().map(|v|v.to_json(symbols)),"modifier_value":modifier.to_json(symbols),"outcome":if *blocked {"BLOCKED"}else{"DEFEATED"}}),
            ),
            Self::Defeated { actor, target } => {
                ("UNIT_DEFEATED", *actor, Some(*target), None, json!({}))
            }
            Self::Gold {
                hero,
                amount,
                reason,
            } => (
                "GOLD_GAINED",
                Some(*hero),
                None,
                None,
                json!({"amount":amount,"reason":match reason {GoldReason::Kill=>"kill",GoldReason::Assist=>"assist",GoldReason::MinionKill=>"minion_kill"}}),
            ),
            Self::Life {
                team,
                change,
                remaining,
            } => (
                "LIFE_COUNTER_CHANGED",
                None,
                None,
                None,
                json!({"team":symbols.name(*team),"change":change,"remaining":remaining}),
            ),
            Self::Removed { target, from } => {
                ("UNIT_REMOVED", None, Some(*target), *from, json!({}))
            }
            Self::Bounty { hero, minion } => (
                "GOLD_GAINED",
                Some(*hero),
                Some(*minion),
                None,
                json!({"amount":1,"reason":"minion_defeat_bounty"}),
            ),
            Self::Protected {
                hero,
                minion,
                effect,
                card,
                token,
            } => (
                "MINION_PROTECTED",
                Some(*hero),
                Some(*minion),
                None,
                if let Some(card) = card {
                    json!({"discarded_card_id":symbols.name(*card),"effect_id":symbols.name(*effect)})
                } else {
                    json!({"sacrificed_token_id":token.map(|id| symbols.name(id)),"effect_id":symbols.name(*effect)})
                },
            ),
            Self::GameOver(outcome) => (
                "GAME_OVER",
                None,
                None,
                None,
                json!({"winner":symbols.name(outcome.team.or(outcome.hero).unwrap()),"winner_type":if outcome.team.is_some(){"TEAM"}else{"HERO"},"condition":symbols.name(outcome.condition)}),
            ),
        };
        json!({"event_type":kind,"actor_id":actor.map(|s|symbols.name(s)),"target_id":target.map(|s|symbols.name(s)),"from_hex":from,"to_hex":null,"metadata":metadata})
    }
}
#[derive(Clone, Debug)]
pub struct DefeatResult {
    pub protection: bool,
    pub removals: Vec<Symbol>,
    pub events: Vec<CombatEvent>,
    pub outcome: Option<GameOutcome>,
}
#[derive(Clone, Debug)]
pub struct CombatRules {
    block: Symbol,
    last: Symbol,
    defense: Symbol,
    invalid: Symbol,
    auto: Symbol,
    ignore: Symbol,
    modifier: Symbol,
    initiative: Symbol,
    bonus: Symbol,
    log: Symbol,
    last_minion: Symbol,
    empty: Symbol,
    bounty: Symbol,
    red: Symbol,
    blue: Symbol,
    annihilation: Symbol,
}
impl CombatRules {
    pub fn compile(symbols: &mut Symbols) -> Self {
        Self {
            block: symbols.intern("block_succeeded"),
            last: symbols.intern("last_combat_target"),
            defense: symbols.intern("defense_value"),
            invalid: symbols.intern("defense_invalid"),
            auto: symbols.intern("auto_block"),
            ignore: symbols.intern("ignore_minion_defense"),
            modifier: symbols.intern("minion_defense_modifier"),
            initiative: symbols.intern("defense_uses_initiative"),
            bonus: symbols.intern("defense_bonus"),
            log: symbols.intern("defeat_log"),
            last_minion: symbols.intern("last_defeated_minion_id"),
            empty: symbols.intern(""),
            bounty: symbols.intern("bounty"),
            red: symbols.intern("RED"),
            blue: symbols.intern("BLUE"),
            annihilation: symbols.intern("ANNIHILATION"),
        }
    }
    pub fn resolve(
        &self,
        world: &NativeWorld,
        context: &mut impl RegisterWrite,
        symbols: &Symbols,
        actor: Option<Symbol>,
        target_key: Symbol,
        damage: i64,
    ) -> Result<(Option<CombatEvent>, Option<DefeatBinding>)> {
        let truth = |key| context.get(key).is_some_and(|v| v.truthy(symbols));
        let target = context
            .get(target_key)
            .filter(|v| v.truthy(symbols))
            .cloned();
        let Some(target) = target else {
            context.set(self.block, Datum::Bool(false));
            return Ok((None, None));
        };
        let Datum::Text(id) = target else {
            return Err(unsupported(
                "non-string combat target coercion not compiled",
            ));
        };
        if world.position(id).is_none() {
            context.set(self.block, Datum::Bool(false));
            return Ok((None, None));
        }
        let invalid = truth(self.invalid);
        let auto = truth(self.auto);
        let ignore = truth(self.ignore);
        let initiative = truth(self.initiative);
        context.set(self.last, Datum::Text(id));
        let defense = context
            .get(self.defense)
            .filter(|v| !matches!(v, Datum::Null))
            .cloned();
        let (blocked, shown_defense, shown_modifier) = if invalid {
            (false, None, Datum::Int(0))
        } else if auto {
            (true, None, Datum::Int(0))
        } else {
            // Source calculates the modifier before testing for absent defense.
            let modifier = if ignore {
                Datum::Int(0)
            } else {
                match context
                    .get(self.modifier)
                    .filter(|v| !matches!(v, Datum::Null))
                    .cloned()
                {
                    Some(value) => value,
                    None => Datum::Int(
                        world
                            .minion_defense(id, actor)
                            .map_err(|e| unsupported(&e))?,
                    ),
                }
            };
            if let Some(defense) = defense {
                let bonus = if initiative {
                    0
                } else {
                    context
                        .get(self.bonus)
                        .unwrap_or(&Datum::Int(0))
                        .to_integer(symbols)
                        .map_err(|e| unsupported(&e))?
                        .ok_or_else(|| rule("invalid defense bonus"))?
                };
                let total = numeric(&defense)?
                    .add(CombatNumber::Int(i128::from(bonus)))
                    .add(numeric(&modifier)?);
                (total.ge_i64(damage), Some(defense), modifier)
            } else {
                (false, None, Datum::Int(0))
            }
        };
        context.set(self.block, Datum::Bool(blocked));
        let event = CombatEvent::Resolved {
            actor: actor.filter(|id| *id != self.empty),
            target: id,
            attack: damage,
            defense: shown_defense,
            modifier: shown_modifier,
            blocked,
        };
        Ok((
            Some(event),
            (!blocked).then_some(DefeatBinding {
                victim: id,
                killer: actor,
            }),
        ))
    }
    pub fn defeat(
        &self,
        world: &mut NativeWorld,
        context: &mut Registers,
        victim_id: Symbol,
        killer: Option<Symbol>,
        assist_multiplier: i64,
    ) -> Result<DefeatResult> {
        let victim = world
            .entity(victim_id)
            .filter(|e| {
                matches!(
                    e.kind,
                    EntityKind::Hero | EntityKind::Piece | EntityKind::Minion
                )
            })
            .ok_or_else(|| rule("cannot defeat unknown unit"))?
            .clone();
        let mut victim_id = victim_id;
        let mut removals = vec![victim_id];
        let empty = || DefeatResult {
            protection: false,
            removals: Vec::new(),
            events: Vec::new(),
            outcome: None,
        };
        if victim.kind == EntityKind::Piece && world.position(victim_id).is_none()
            || victim.kind == EntityKind::Hero
                && victim.multi_piece
                && !world.has_board_presence(victim_id)
        {
            return Ok(empty());
        }
        if victim.kind == EntityKind::Piece {
            let hero = world
                .hero(victim_id)
                .ok_or_else(|| rule("piece has no owner hero"))?;
            victim_id = hero.id;
            removals = world.piece_ids(hero.id).collect();
        }
        if victim.kind == EntityKind::Minion
            && world
                .protection_candidate(victim_id, &[])
                .map_err(|e| unsupported(&e))?
                .is_some()
        {
            return Ok(DefeatResult {
                protection: true,
                removals: Vec::new(),
                events: Vec::new(),
                outcome: None,
            });
        }
        let hero_index = world.heroes.iter().position(|h| h.id == victim_id);
        let mut victims = vec![victim_id];
        for id in &removals {
            if !victims.contains(id) {
                victims.push(*id);
            }
        }
        let mut log = match context.get(self.log) {
            None => Vec::new(),
            Some(Datum::List(items)) => items.clone(),
            _ => return Err(rule("defeat log is not a list")),
        };
        log.extend(victims.iter().map(|id| {
            Datum::List(vec![
                Datum::Text(*id),
                killer.map_or(Datum::Null, Datum::Text),
            ])
        }));
        context.set(self.log, Datum::List(log));
        let mut events = vec![CombatEvent::Defeated {
            actor: killer,
            target: victim_id,
        }];
        let owner = |id| world.hero(id).map_or(id, |h| h.id);
        let bounty = world
            .markers
            .iter()
            .find(|m| m.key == self.bounty)
            .and_then(|m| m.target)
            .is_some_and(|target| owner(target) == victim_id);
        let clear = world
            .markers
            .iter()
            .map(|m| {
                m.target
                    .is_some_and(|target| target != self.empty && owner(target) == victim_id)
            })
            .collect::<Vec<_>>();
        for (marker, clear) in world.markers.iter_mut().zip(clear) {
            if clear {
                marker.target = None;
                marker.value = 0;
                marker.source = None;
            }
        }
        world.expire_effects_by_source(victim_id);
        if let Some(index) = hero_index {
            world
                .resolve_current_card(victim_id, Some(world.combat.turn))
                .map_err(|e| match e {
                    CardError::Illegal(s) => rule(s),
                    CardError::Unsupported(s) => unsupported(s),
                })?;
            if let Some(i) = world.combat.unresolved.iter().position(|h| *h == victim_id) {
                world.combat.unresolved.remove(i);
            }
            if !world.combat.defeated.contains(&victim_id) {
                world.combat.defeated.push(victim_id);
            }
            let level = world.heroes[index].level;
            let (kill, assist, penalty): (i64, i64, i64) = match level {
                1 => (1, 1, 1),
                2 => (2, 1, 1),
                3 => (3, 1, 1),
                4 => (4, 2, 2),
                5 => (5, 2, 2),
                6 => (6, 2, 2),
                7 => (7, 3, 3),
                8 => (8, 3, 3),
                n => (n, 1, 1),
            };
            if let Some(killer) = killer
                .and_then(|id| world.entity(id))
                .filter(|e| e.kind == EntityKind::Hero)
                .map(|e| e.id)
            {
                gain(world, killer, kill)?;
                events.push(CombatEvent::Gold {
                    hero: killer,
                    amount: kill,
                    reason: GoldReason::Kill,
                });
            }
            let killer_unit = killer.and_then(|id| world.entity(id)).filter(|e| {
                matches!(
                    e.kind,
                    EntityKind::Hero | EntityKind::Minion | EntityKind::Piece
                )
            });
            if let Some(killer_unit) = killer_unit {
                let killer_id = killer_unit.id;
                if let Some(team) = killer_unit
                    .team
                    .filter(|team| world.combat.teams.iter().any(|t| t.key == *team))
                {
                    let amount = assist
                        .checked_mul(assist_multiplier)
                        .ok_or_else(|| unsupported("assist reward outside i64"))?;
                    let allies = world
                        .heroes
                        .iter()
                        .filter(|h| h.team == team && h.id != killer_id)
                        .map(|h| h.id)
                        .collect::<Vec<_>>();
                    for ally in allies {
                        gain(world, ally, amount)?;
                        events.push(CombatEvent::Gold {
                            hero: ally,
                            amount,
                            reason: GoldReason::Assist,
                        });
                    }
                }
            }
            let actual_team = world.entity(victim_id).and_then(|e| e.team);
            if let Some(team) =
                actual_team.and_then(|color| world.combat.teams.iter_mut().find(|t| t.key == color))
            {
                let penalty = penalty + i64::from(bounty);
                team.life = team.life.saturating_sub(penalty).max(0);
                events.push(CombatEvent::Life {
                    team: team.key,
                    change: -penalty,
                    remaining: team.life,
                });
                if team.life == 0 {
                    return Ok(DefeatResult {
                        protection: false,
                        removals,
                        events,
                        outcome: Some(GameOutcome {
                            team: Some(if team.key == self.red {
                                self.blue
                            } else {
                                self.red
                            }),
                            hero: None,
                            condition: self.annihilation,
                        }),
                    });
                }
            }
        } else {
            context.set(self.last_minion, Datum::Text(victim_id));
            if let Some(killer) = killer
                .and_then(|id| world.entity(id))
                .filter(|e| e.kind == EntityKind::Hero)
                .map(|e| e.id)
            {
                let amount = if victim.minion_kind == Some(MinionKind::Heavy) {
                    4
                } else {
                    2
                };
                gain(world, killer, amount)?;
                events.push(CombatEvent::Gold {
                    hero: killer,
                    amount,
                    reason: GoldReason::MinionKill,
                });
            }
        }
        if hero_index.is_none() {
            events.extend(
                world
                    .award_minion_bounties(victim_id)
                    .map_err(|e| unsupported(&e))?,
            );
        }
        Ok(DefeatResult {
            protection: false,
            removals,
            events,
            outcome: None,
        })
    }
    pub(crate) fn protection_defeat(
        &self,
        world: &mut NativeWorld,
        context: &mut Registers,
        minion: Symbol,
        killer: Option<Symbol>,
        bounty: bool,
    ) -> Result<Vec<CombatEvent>> {
        context.set(self.last_minion, Datum::Text(minion));
        let mut log = match context.get(self.log) {
            None => Vec::new(),
            Some(Datum::List(v)) => v.clone(),
            _ => return Err(rule("defeat log is not a list")),
        };
        log.push(Datum::List(vec![
            Datum::Text(minion),
            killer.map_or(Datum::Null, Datum::Text),
        ]));
        context.set(self.log, Datum::List(log));
        let mut events = vec![CombatEvent::Defeated {
            actor: killer,
            target: minion,
        }];
        if let Some(killer) = killer.filter(|id| {
            world
                .entity(*id)
                .is_some_and(|e| e.kind == EntityKind::Hero)
        }) {
            let reward = if world
                .entity(minion)
                .is_some_and(|e| e.minion_kind == Some(MinionKind::Heavy))
            {
                4
            } else {
                2
            };
            gain(world, killer, reward)?;
            events.push(CombatEvent::Gold {
                hero: killer,
                amount: reward,
                reason: GoldReason::MinionKill,
            });
        }
        if bounty {
            events.extend(
                world
                    .award_minion_bounties(minion)
                    .map_err(|e| unsupported(&e))?,
            );
        }
        Ok(events)
    }
}
enum CombatNumber {
    Int(i128),
    Float(f64),
}
fn numeric(value: &Datum) -> Result<CombatNumber> {
    match value {
        Datum::Int(value) => Ok(CombatNumber::Int(i128::from(*value))),
        Datum::Bool(value) => Ok(CombatNumber::Int(i128::from(*value))),
        Datum::Float(value) => Ok(CombatNumber::Float(*value)),
        _ => Err(unsupported("combat value requires numeric data")),
    }
}
impl CombatNumber {
    fn add(self, other: Self) -> Self {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => Self::Int(a + b),
            (Self::Float(a), Self::Int(b)) => Self::Float(a + b as f64),
            (Self::Int(a), Self::Float(b)) => Self::Float(a as f64 + b),
            (Self::Float(a), Self::Float(b)) => Self::Float(a + b),
        }
    }
    fn ge_i64(self, target: i64) -> bool {
        match self {
            Self::Int(value) => value >= i128::from(target),
            Self::Float(value) => {
                // Internal float overflow is a valid comparison. Do not round
                // the i64 target through f64 above its 53-bit precision.
                if value.is_nan() {
                    false
                } else if value >= 9223372036854775808.0 {
                    true
                } else if value < -9223372036854775808.0 {
                    false
                } else {
                    let whole = value as i64;
                    whole > target || whole == target && value >= whole as f64
                }
            }
        }
    }
}
fn gain(world: &mut NativeWorld, hero: Symbol, amount: i64) -> Result<()> {
    let index = world
        .heroes
        .iter()
        .position(|h| h.id == hero)
        .ok_or_else(|| rule("reward hero missing"))?;
    world.heroes[index].gold = world.heroes[index]
        .gold
        .checked_add(amount)
        .ok_or_else(|| unsupported("gold outside i64"))?;
    Ok(())
}
impl NativeWorld {
    pub fn export_combat(&self, state: &mut Value, symbols: &Symbols) {
        self.combat.export(state, symbols);
        self.history.export(state, symbols);
        for marker in &self.markers {
            let row = &mut state["markers"][symbols.name(marker.key)];
            row["type"] = serde_json::to_value(marker.kind).expect("marker enum serializes");
            row["target_id"] = json!(marker.target.map(|id| symbols.name(id)));
            row["value"] = json!(marker.value);
            row["source_id"] = json!(marker.source.map(|id| symbols.name(id)));
        }
    }
}
