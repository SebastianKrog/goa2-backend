//! Compiled spatial predicates. Each movement filter prepares one traversal
//! for the entire ordered candidate scan; nothing is cached across mutations.
use crate::{
    data::{Datum, Registers, Symbol, Symbols},
    geometry::Hex,
    world::{EntityKind, NativeWorld},
};
use serde_json::Value;

#[derive(Clone, Debug, Default)]
pub struct Origin {
    pub literal: Option<Symbol>,
    pub key: Option<Symbol>,
    pub frame: bool,
}
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdjacencyTag {
    Friendly,
    Enemy,
    Hero,
    Minion,
}
#[derive(Clone, Debug)]
pub enum HexFilter {
    AdjacentTags(Vec<AdjacencyTag>),
    Spawn {
        wanted: bool,
        minion_only: bool,
    },
    AdjacentSpawn {
        empty: bool,
        forbidden: bool,
        battle_only: bool,
        minion_only: bool,
    },
    FastTravel {
        origin: Origin,
        change: bool,
    },
    Obstacle {
        wanted: bool,
        exclude: Origin,
    },
    Range {
        origin: Origin,
        hex_key: Option<Symbol>,
        min: Option<i64>,
        max: Option<i64>,
        min_key: Option<Symbol>,
        max_key: Option<Symbol>,
        frame_range: bool,
        geometric: bool,
    },
    Movement {
        origin: Origin,
        range: i32,
        through: bool,
    },
    Straight {
        origin: Origin,
        invert: bool,
    },
    ClearLine {
        origin: Origin,
        through: bool,
    },
}
pub struct SpatialContext<'a> {
    pub world: &'a NativeWorld,
    pub registers: &'a Registers,
    pub symbols: &'a Symbols,
    pub actor: Option<Symbol>,
    pub frame: Option<Option<Symbol>>,
    pub coordinates: [Symbol; 3],
    pub range_override: Option<i32>,
    pub through_override: Option<bool>,
}
impl SpatialContext<'_> {
    pub fn origin(&self, origin: &Origin, fallback: bool) -> Result<Option<Symbol>, String> {
        let literal = if origin.frame {
            self.frame.flatten()
        } else {
            origin.literal
        };
        if literal.is_some() {
            return Ok(literal);
        }
        if let Some(key) = origin.key {
            match self.registers.get(key) {
                Some(Datum::Text(id)) if !self.symbols.name(*id).is_empty() => {
                    return Ok(Some(*id));
                }
                Some(value) if value.truthy(self.symbols) => {
                    return Err("non-string spatial origin not compiled".into());
                }
                _ => {}
            }
        }
        Ok(if fallback { self.actor } else { None })
    }
    fn int_override(&self, key: Option<Symbol>, base: Option<i64>) -> Option<i64> {
        match key.and_then(|key| self.registers.get(key)) {
            Some(Datum::Int(n)) => Some(*n),
            Some(Datum::Bool(v)) => Some(i64::from(*v)),
            _ => base,
        }
    }
}
impl HexFilter {
    pub fn apply_scan(
        &self,
        context: &SpatialContext<'_>,
        candidates: &[Hex],
        accepted: &mut [bool],
    ) -> Result<(), String> {
        self.apply_scan_with_unit(context, candidates, accepted, None)
    }
    pub(crate) fn accept_unit(
        &self,
        context: &SpatialContext<'_>,
        unit: Symbol,
    ) -> Result<bool, String> {
        let Some(hex) = context.world.position(unit) else {
            return Ok(false);
        };
        let mut accepted = [true];
        self.apply_scan_with_unit(context, &[hex], &mut accepted, Some(unit))?;
        Ok(accepted[0])
    }
    fn apply_scan_with_unit(
        &self,
        context: &SpatialContext<'_>,
        candidates: &[Hex],
        accepted: &mut [bool],
        target_unit: Option<Symbol>,
    ) -> Result<(), String> {
        match self {
            Self::AdjacentTags(tags) => {
                for (hex, value) in candidates.iter().zip(accepted.iter_mut()) {
                    if !*value {
                        continue;
                    }
                    let mut found = false;
                    if let Some(actor) = context.actor.and_then(|id| context.world.entity(id)) {
                        for neighbor in hex.neighbors() {
                            if !context.world.on_map(neighbor)
                                || !context.world.effect_connected(*hex, neighbor, &[])?
                            {
                                continue;
                            }
                            let Some(unit) = context
                                .world
                                .occupant(neighbor)
                                .and_then(|id| context.world.entity(id))
                            else {
                                continue;
                            };
                            if tags.iter().all(|tag| match tag {
                                AdjacencyTag::Friendly => {
                                    unit.team.is_some()
                                        && actor.team == unit.team
                                        && unit.id != actor.id
                                }
                                AdjacencyTag::Enemy => {
                                    unit.team.is_some()
                                        && actor.team.is_some()
                                        && actor.team != unit.team
                                }
                                AdjacencyTag::Hero => {
                                    matches!(unit.kind, EntityKind::Hero | EntityKind::Piece)
                                }
                                AdjacencyTag::Minion => unit.kind == EntityKind::Minion,
                            }) {
                                found = true;
                                break;
                            }
                        }
                    }
                    *value &= found;
                }
            }
            Self::Spawn {
                wanted,
                minion_only,
            } => {
                for (hex, value) in candidates.iter().zip(accepted.iter_mut()) {
                    *value &= context.world.spawn_at(*hex, *minion_only) == *wanted;
                }
            }
            Self::AdjacentSpawn {
                empty,
                forbidden,
                battle_only,
                minion_only,
            } => {
                let all_zones = context.world.tide_of_darkness(context.actor);
                for (hex, value) in candidates.iter().zip(accepted.iter_mut()) {
                    if !*value {
                        continue;
                    }
                    let mut found = false;
                    for neighbor in hex.neighbors() {
                        if !context.world.effect_connected(*hex, neighbor, &[])?
                            || !context.world.spawn_at(neighbor, *minion_only)
                        {
                            continue;
                        }
                        if *battle_only
                            && !all_zones
                            && !context.world.tile_in_battle_zone(neighbor)
                        {
                            continue;
                        }
                        if *empty && context.world.obstacle_for_actor(neighbor, context.actor)? {
                            continue;
                        }
                        found = true;
                        break;
                    }
                    *value &= found != *forbidden;
                }
            }
            Self::FastTravel { origin, change } => {
                let unit = context.origin(origin, true)?;
                let values = context
                    .world
                    .fast_travel_destinations(unit, *change, candidates)?;
                for (target, value) in accepted.iter_mut().zip(values) {
                    *target &= value;
                }
            }
            Self::Obstacle { wanted, exclude } => {
                let except = context.origin(exclude, false)?;
                for (i, &hex) in candidates.iter().enumerate() {
                    accepted[i] &= except.is_some_and(|id| context.world.occupant(hex) == Some(id))
                        || context.world.obstacle_for_actor(hex, context.actor)? == *wanted;
                }
            }
            Self::Movement {
                origin,
                range,
                through,
            } => {
                let unit = context.origin(origin, true)?;
                let start = unit.and_then(|id| context.world.position(id));
                let reachable = if let (Some(unit), Some(start)) = (unit, start) {
                    context.world.movement_reachable(
                        &crate::geometry::MovementQuery {
                            start,
                            ends: candidates.to_vec(),
                            max_steps: context.range_override.unwrap_or(*range),
                            ignore_obstacles: false,
                            pass_through_obstacles: context.through_override.unwrap_or(*through),
                        },
                        context.actor,
                        Some(&[unit]),
                        &[unit],
                    )?
                } else {
                    vec![false; candidates.len()]
                };
                for (value, reachable) in accepted.iter_mut().zip(reachable) {
                    *value &= reachable;
                }
            }
            Self::Range {
                origin,
                hex_key,
                min,
                max,
                min_key,
                max_key,
                frame_range,
                geometric,
            } => {
                let from_hex = hex_key
                    .and_then(|key| context.registers.get(key))
                    .filter(|v| matches!(v, Datum::Object(_)))
                    .map(|v| v.hex(context.coordinates, context.symbols))
                    .transpose()?;
                let origin_unit = if from_hex.is_none() {
                    context.origin(origin, true)?
                } else {
                    None
                };
                let from = from_hex
                    .or_else(|| origin_unit.and_then(|id| context.world.resolved_position(id)));
                let mut units = [context.world.definitions.effect_empty; 2];
                let mut unit_count = 0;
                for id in [origin_unit, target_unit].into_iter().flatten() {
                    units[unit_count] = id;
                    unit_count += 1;
                }
                let frame_range = if *frame_range {
                    context.range_override.map(i64::from)
                } else {
                    None
                };
                let min = context.int_override(*min_key, frame_range.or(*min));
                let max = context.int_override(*max_key, frame_range.or(*max));
                for (i, &hex) in candidates.iter().enumerate() {
                    if !accepted[i] {
                        continue;
                    }
                    accepted[i] &= if let Some(from) = from {
                        let distance = i64::from(from.distance(hex));
                        (*geometric
                            || context
                                .world
                                .effect_connected(from, hex, &units[..unit_count])?)
                            && min.is_none_or(|min| distance >= min)
                            && max.is_none_or(|max| distance <= max)
                    } else {
                        false
                    };
                }
            }
            Self::Straight { origin, invert } => {
                let from = context
                    .origin(origin, true)?
                    .and_then(|id| context.world.resolved_position(id));
                for (i, &hex) in candidates.iter().enumerate() {
                    if !accepted[i] {
                        continue;
                    }
                    accepted[i] &= if let Some(from) = from {
                        (from.straight(hex) && context.world.effect_connected(from, hex, &[])?)
                            != *invert
                    } else {
                        false
                    };
                }
            }
            Self::ClearLine { origin, through } => {
                let unit = context.origin(origin, true)?;
                let from = unit.and_then(|id| context.world.resolved_position(id));
                for (i, &hex) in candidates.iter().enumerate() {
                    if !accepted[i] {
                        continue;
                    }
                    let mut clear = false;
                    if let Some(path) = from.and_then(|from| from.line_to(hex)) {
                        clear = true;
                        for point in path.iter().take(path.len().saturating_sub(1)) {
                            if !context.world.on_map(*point)
                                || (!context.through_override.unwrap_or(*through)
                                    && context.world.obstacle_for_actor(*point, unit)?
                                    && !context.world.passable_token(*point, unit.as_slice()))
                            {
                                clear = false;
                                break;
                            }
                        }
                    }
                    accepted[i] &= clear;
                }
            }
        }
        Ok(())
    }
}
pub fn compile_filter(raw: &Value, symbols: &mut Symbols) -> Result<HexFilter, String> {
    let origin = |symbols: &mut Symbols, literal: &str, key: &str| -> Result<Origin, String> {
        Ok(Origin {
            literal: optional(raw, literal, symbols)?,
            key: optional(raw, key, symbols)?,
            frame: false,
        })
    };
    match raw["type"].as_str().ok_or("filter type missing")? {
        "adjacency_filter" => Ok(HexFilter::AdjacentTags(
            serde_json::from_value(raw["target_tags"].clone()).map_err(|e| e.to_string())?,
        )),
        "spawn_point_filter" => Ok(HexFilter::Spawn {
            wanted: raw["has_spawn_point"].as_bool().unwrap_or(false),
            minion_only: raw["minion_only"].as_bool().unwrap_or(false),
        }),
        "adjacent_spawn_point_filter" => Ok(HexFilter::AdjacentSpawn {
            empty: raw["is_empty"].as_bool().unwrap_or(true),
            forbidden: raw["must_not_have"].as_bool().unwrap_or(true),
            battle_only: raw["battle_zone_only"].as_bool().unwrap_or(false),
            minion_only: raw["minion_only"].as_bool().unwrap_or(false),
        }),
        "fast_travel_destination_filter" => {
            let _source = optional(raw, "source_card_id", symbols)?;
            Ok(HexFilter::FastTravel {
                origin: Origin {
                    literal: optional(raw, "unit_id", symbols)?,
                    ..Default::default()
                },
                change: raw["require_zone_change"].as_bool().unwrap_or(false),
            })
        }
        "occupied_filter" => Ok(HexFilter::Obstacle {
            wanted: raw["is_obstacle"].as_bool().unwrap_or(false),
            exclude: origin(symbols, "exclude_id", "exclude_id_key")?,
        }),
        "range_filter" => Ok(HexFilter::Range {
            origin: origin(symbols, "origin_id", "origin_key")?,
            hex_key: optional(raw, "origin_hex_key", symbols)?,
            min: raw.get("min_range").map_or(Some(0), Value::as_i64),
            max: raw.get("max_range").and_then(Value::as_i64),
            min_key: optional(raw, "min_range_key", symbols)?,
            max_key: optional(raw, "max_range_key", symbols)?,
            frame_range: false,
            geometric: match raw["distance_mode"].as_str().unwrap_or("topology") {
                "topology" => false,
                "geometric" => true,
                _ => return Err("invalid range distance mode".into()),
            },
        }),
        "movement_path_filter" => Ok(HexFilter::Movement {
            origin: origin(symbols, "unit_id", "unit_key")?,
            range: i32::try_from(
                raw["range_val"]
                    .as_i64()
                    .ok_or("movement filter range missing")?,
            )
            .map_err(|_| "movement range outside i32")?,
            through: raw["pass_through_obstacles"].as_bool().unwrap_or(false),
        }),
        "in_straight_line_filter" | "not_in_straight_line_filter" => Ok(HexFilter::Straight {
            origin: origin(symbols, "origin_id", "origin_key")?,
            invert: raw["type"] == "not_in_straight_line_filter",
        }),
        "straight_line_path_filter" => Ok(HexFilter::ClearLine {
            origin: origin(symbols, "origin_id", "origin_key")?,
            through: raw["pass_through_obstacles"].as_bool().unwrap_or(false),
        }),
        other => Err(format!("hex filter not compiled: {other}")),
    }
}
fn optional(raw: &Value, key: &str, symbols: &mut Symbols) -> Result<Option<Symbol>, String> {
    match raw.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Ok((!v.is_empty()).then(|| symbols.intern(v))),
        _ => Err(format!("invalid filter origin/key: {key}")),
    }
}
