//! Immutable zone membership and ordered Fast Travel connections.
use crate::{
    data::{Symbol, Symbols},
    geometry::Hex,
    world::NativeWorld,
};
use serde_json::Value;
use std::collections::HashSet;
#[derive(Clone, Debug)]
pub struct Zone {
    pub id: Symbol,
    pub actual_id: Symbol,
    pub hexes: HashSet<Hex>,
    pub neighbors: Vec<Symbol>,
    pub spawns: Vec<Spawn>,
}
#[derive(Clone, Debug)]
pub struct Spawn {
    pub source_index: usize,
    pub hex: Hex,
    pub team: Symbol,
    pub kind: crate::world::MinionKind,
}
#[derive(Clone, Debug)]
pub struct ZoneGraph {
    pub empty: Symbol,
    pub zones: Vec<Zone>,
    pub lanes: Vec<(Symbol, Vec<Symbol>)>,
    pub hero_spawns: Vec<(Symbol, Hex)>,
}
impl ZoneGraph {
    pub fn compile(board: &Value, symbols: &mut Symbols) -> Result<Self, String> {
        let mut zones = Vec::new();
        for (key, zone) in board["zones"].as_object().ok_or("zones missing")? {
            let hexes = zone["hexes"]
                .as_array()
                .ok_or("zone hexes missing")?
                .iter()
                .map(|v| serde_json::from_value(v.clone()).map_err(|e| format!("zone hex: {e}")))
                .collect::<Result<HashSet<Hex>, _>>()?;
            let neighbors = zone["neighbors"]
                .as_array()
                .ok_or("zone neighbors missing")?
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(|s| symbols.intern(s))
                        .ok_or("invalid zone neighbor".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            zones.push(Zone {
                id: symbols.intern(key),
                actual_id: symbols.intern(zone["id"].as_str().ok_or("zone identity missing")?),
                hexes,
                neighbors,
                spawns: zone["spawn_points"]
                    .as_array()
                    .ok_or("zone spawn points missing")?
                    .iter()
                    .enumerate()
                    .filter(|(_, sp)| sp["type"] == "MINION")
                    .map(|(source_index, sp)| {
                        Ok(Spawn {
                            source_index,
                            hex: serde_json::from_value(sp["location"].clone())
                                .map_err(|e| format!("spawn hex: {e}"))?,
                            team: symbols.intern(sp["team"].as_str().ok_or("spawn team missing")?),
                            kind: serde_json::from_value(sp["minion_type"].clone())
                                .map_err(|e| format!("spawn kind: {e}"))?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?,
            });
        }
        Ok(Self {
            hero_spawns: board["spawn_points"]
                .as_array()
                .ok_or("spawn points missing")?
                .iter()
                .filter(|sp| sp["type"] == "HERO")
                .map(|sp| {
                    Ok((
                        symbols.intern(sp["team"].as_str().ok_or("spawn team missing")?),
                        serde_json::from_value(sp["location"].clone())
                            .map_err(|e| format!("hero spawn hex: {e}"))?,
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?,
            lanes: board["lanes"]
                .as_object()
                .ok_or("lanes missing")?
                .iter()
                .map(|(id, zones)| {
                    let id = symbols.intern(id);
                    let zones = zones
                        .as_array()
                        .ok_or("invalid lane")?
                        .iter()
                        .map(|z| {
                            z.as_str()
                                .map(|s| symbols.intern(s))
                                .ok_or("invalid lane zone".to_string())
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok((id, zones))
                })
                .collect::<Result<Vec<_>, String>>()?,
            zones,
            empty: symbols.intern(""),
        })
    }
    pub fn zone_at(&self, hex: Hex) -> Option<Symbol> {
        // Board.get_zone_for_hex falls back to the first zone's own ID when
        // no concrete tile exists. Concrete tiles are checked by NativeWorld.
        self.zones
            .iter()
            .find(|z| z.hexes.contains(&hex))
            .map(|z| z.actual_id)
    }
}
impl NativeWorld {
    pub fn safe_travel_zones(&self, team: Symbol, zone: Symbol) -> Vec<Symbol> {
        let graph = &self.definitions.zones;
        let Some(start) = graph.zones.iter().find(|z| z.id == zone) else {
            return Vec::new();
        };
        let blocked = |zone: &Zone| {
            self.locations.iter().any(|(id, hex)| {
                zone.hexes.contains(hex)
                    && self
                        .entity(*id)
                        .is_some_and(|e| e.has_team && e.team != Some(team))
            })
        };
        if blocked(start) {
            return Vec::new();
        }
        std::iter::once(start.id)
            .chain(start.neighbors.iter().copied())
            .filter(|id| {
                graph
                    .zones
                    .iter()
                    .find(|z| z.id == *id)
                    .is_some_and(|z| !blocked(z))
            })
            .collect()
    }
    pub fn has_safe_travel_zone(&self, hero: Symbol) -> bool {
        let Some(team) = self.entity(hero).and_then(|e| e.team) else {
            return false;
        };
        self.positions(hero)
            .filter_map(|h| self.zone_at(h))
            .filter(|z| *z != self.definitions.zones.empty)
            .any(|z| !self.safe_travel_zones(team, z).is_empty())
    }
}

impl NativeWorld {
    pub fn ensure_fast_travel_closure(&self) -> Result<(), String> {
        if self.has_effects || self.has_uncompiled_auras {
            Err("Fast Travel action/topology effect closure not compiled".into())
        } else {
            Ok(())
        }
    }
    /// One unchanged destination scan: derive shared action and safe-zone facts
    /// once, then test ordered candidates. Fast Travel has no path traversal.
    pub fn fast_travel_destinations(
        &self,
        unit: Option<Symbol>,
        change: bool,
        candidates: &[Hex],
    ) -> Result<Vec<bool>, String> {
        self.ensure_fast_travel_closure()?;
        self.fast_travel_destinations_with_card(unit, change, candidates, None)
    }
    pub fn fast_travel_destinations_with_card(
        &self,
        unit: Option<Symbol>,
        change: bool,
        candidates: &[Hex],
        card: Option<crate::cards::CardId>,
    ) -> Result<Vec<bool>, String> {
        self.ensure_move_closure(true)?;
        let reject = || vec![false; candidates.len()];
        let Some(unit) = unit else {
            return Ok(reject());
        };
        if !self
            .action_legality(
                unit,
                crate::world::ActionKind::FastTravel,
                card,
                &crate::data::Registers::default(),
            )?
            .allowed
        {
            return Ok(reject());
        }
        let Some(entity) = self.entity(unit).filter(|e| {
            matches!(
                e.kind,
                crate::world::EntityKind::Hero
                    | crate::world::EntityKind::Piece
                    | crate::world::EntityKind::Minion
            )
        }) else {
            return Ok(reject());
        };
        let Some(team) = entity.team else {
            return Ok(reject());
        };
        let Some(from) = self.position(unit) else {
            return Ok(reject());
        };
        let Some(start_zone) = self.zone_at(from) else {
            return Ok(reject());
        };
        if start_zone == self.definitions.zones.empty {
            return Ok(reject());
        }
        let safe = self.safe_travel_zones(team, start_zone);
        candidates
            .iter()
            .map(|hex| {
                Ok(self
                    .zone_at(*hex)
                    .is_some_and(|z| safe.contains(&z) && (!change || z != start_zone))
                    && self.effect_connected(from, *hex, &[unit])?
                    && !self.obstacle_for_actor(*hex, Some(unit))?
                    && self.on_map(*hex)
                    && self.occupant(*hex).is_none())
            })
            .collect()
    }
}
