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
        match self {
            Self::AdjacentTags(tags) => {
                let actor = context.actor.and_then(|id| context.world.entity(id));
                for (hex, value) in candidates.iter().zip(accepted.iter_mut()) {
                    *value &= actor.is_some_and(|actor| {
                        hex.neighbors()
                            .iter()
                            .filter(|n| context.world.on_map(**n))
                            .filter_map(|n| {
                                context
                                    .world
                                    .occupant(*n)
                                    .and_then(|id| context.world.entity(id))
                            })
                            .any(|unit| {
                                tags.iter().all(|tag| match tag {
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
                                })
                            })
                    });
                }
            }
            Self::Spawn {
                wanted,
                minion_only,
            } => {
                for (hex, value) in candidates.iter().zip(accepted.iter_mut()) {
                    *value &= context.world.on_map(*hex)
                        && context.world.spawn_at(*hex, *minion_only) == *wanted;
                }
            }
            Self::AdjacentSpawn {
                empty,
                forbidden,
                battle_only,
                minion_only,
            } => {
                if context.world.has_effects || *battle_only && context.world.has_uncompiled_auras {
                    return Err("spawn adjacency topology/aura closure not compiled".into());
                }
                for (hex, value) in candidates.iter().zip(accepted.iter_mut()) {
                    let found = hex.neighbors().iter().any(|n| {
                        context.world.on_map(*n)
                            && context.world.spawn_at(*n, *minion_only)
                            && (!*battle_only || context.world.tile_in_battle_zone(*n))
                            && (!*empty || !context.world.obstacle(*n))
                    });
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
                        || context.world.obstacle(hex) == *wanted;
                }
            }
            Self::Movement {
                origin,
                range,
                through,
            } => {
                context.world.ensure_move_closure(false)?;
                let start = context
                    .origin(origin, true)?
                    .and_then(|id| context.world.position(id));
                let reachable = start.map(|start| {
                    context.world.reachable(
                        start,
                        context.range_override.unwrap_or(*range),
                        context.through_override.unwrap_or(*through),
                    )
                });
                for (i, value) in accepted.iter_mut().enumerate() {
                    *value &= reachable.as_ref().is_some_and(|values| values[i]);
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
            } => {
                let from_hex = hex_key
                    .and_then(|key| context.registers.get(key))
                    .filter(|v| matches!(v, Datum::Object(_)))
                    .map(|v| v.hex(context.coordinates, context.symbols))
                    .transpose()?;
                let from = if from_hex.is_some() {
                    from_hex
                } else {
                    context
                        .origin(origin, true)?
                        .and_then(|id| context.world.resolved_position(id))
                };
                let frame_range = if *frame_range {
                    context.range_override.map(i64::from)
                } else {
                    None
                };
                let min = context.int_override(*min_key, frame_range.or(*min));
                let max = context.int_override(*max_key, frame_range.or(*max));
                for (i, &hex) in candidates.iter().enumerate() {
                    accepted[i] &= from.is_some_and(|from| {
                        let distance = i64::from(from.distance(hex));
                        min.is_none_or(|min| distance >= min)
                            && max.is_none_or(|max| distance <= max)
                    });
                }
            }
            Self::Straight { origin, invert } => {
                let from = context
                    .origin(origin, true)?
                    .and_then(|id| context.world.resolved_position(id));
                for (i, &hex) in candidates.iter().enumerate() {
                    accepted[i] &= from.is_some_and(|from| from.straight(hex) != *invert);
                }
            }
            Self::ClearLine { origin, through } => {
                let from = context
                    .origin(origin, true)?
                    .and_then(|id| context.world.resolved_position(id));
                for (i, &hex) in candidates.iter().enumerate() {
                    if !accepted[i] {
                        continue;
                    }
                    accepted[i] &= from.and_then(|from| from.line_to(hex)).is_some_and(|path| {
                        path.iter().take(path.len().saturating_sub(1)).all(|hex| {
                            context.world.on_map(*hex)
                                && (context.through_override.unwrap_or(*through)
                                    || !context.world.obstacle(*hex))
                        })
                    });
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
