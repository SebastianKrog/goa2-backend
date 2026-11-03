//! Hex-first lane supply selection and legacy chosen-minion respawning.
use super::phases::Field;
use super::*;
use crate::{
    spatial::{HexFilter, SpatialContext},
    world::EntityKind,
};

pub(super) enum RespawnPlan {
    Done {
        clear: bool,
        remove: bool,
    },
    Prompt {
        request: NativeRequest,
        clear: bool,
        store: Option<ObjectValue>,
    },
    Place {
        placement: super::placement::Placement,
        remove: bool,
        unit: Symbol,
    },
}
impl NativeSession {
    fn respawn_supply(&self, team: Symbol, hex: Hex) -> Vec<Symbol> {
        let zone_id = self.world.zone_at(hex);
        let zone = zone_id.and_then(|id| {
            self.world
                .definitions
                .zones
                .zones
                .iter()
                .find(|z| z.id == id)
        });
        let lane = zone_id.and_then(|id| {
            self.world
                .definitions
                .zones
                .lanes
                .iter()
                .find(|(_, ids)| ids.contains(&id))
                .map(|(id, _)| *id)
        });
        let mut seen = Vec::new();
        let mut result = Vec::new();
        for row in &self.world.definitions.entities {
            if row.kind != EntityKind::Minion
                || row.roster != Some(team)
                || self.world.position(row.id).is_some()
                || lane.is_some_and(|lane| row.lane != Some(lane))
            {
                continue;
            }
            let Some(kind) = row.minion_kind else {
                continue;
            };
            if seen.contains(&kind) {
                continue;
            }
            if let Some(zone) = zone {
                let spawns = zone
                    .spawns
                    .iter()
                    .filter(|s| s.team == team && s.kind == kind)
                    .count();
                let present = self
                    .world
                    .definitions
                    .entities
                    .iter()
                    .filter(|m| {
                        m.kind == EntityKind::Minion
                            && m.roster == Some(team)
                            && m.minion_kind == Some(kind)
                            && self
                                .world
                                .position(m.id)
                                .is_some_and(|h| zone.hexes.contains(&h))
                    })
                    .count();
                if spawns <= present {
                    continue;
                }
            }
            seen.push(kind);
            result.push(row.id);
        }
        result
    }
    fn respawn_filters_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Vec<HexFilter>> {
        self.unit_filter_list_bound(objects, step, Field::HexFilters)?
            .into_iter()
            .map(|value| match value {
                ObjectValue::Object(id) => self.hex_filter_bound(objects, id),
                _ => Err(unsupported("respawn hex filter model missing")),
            })
            .collect()
    }
    fn respawn_hexes_bound(
        &self,
        objects: &BoundObjects,
        filters: &[HexFilter],
        candidates: &[Hex],
    ) -> Result<Vec<Hex>> {
        let mut keys = Vec::new();
        for filter in filters {
            self.spatial_filter_keys_bound(objects, filter, &mut keys);
        }
        let inputs = self.spatial_inputs_bound(objects, keys);
        let context = SpatialContext {
            count_observer: Some(&self.count_observer),
            world: &self.world,
            registers: &inputs,
            symbols: &self.symbols,
            actor: self.planning.actor,
            frame: None,
            coordinates: self.coordinates,
            range_override: None,
            through_override: None,
        };
        let mut accepted = candidates
            .iter()
            .map(|hex| self.world.occupant(*hex).is_none())
            .collect::<Vec<_>>();
        for filter in filters {
            filter
                .apply_scan(&context, candidates, &mut accepted)
                .map_err(unsupported)?;
        }
        Ok(candidates
            .iter()
            .zip(accepted)
            .filter_map(|(hex, yes)| yes.then_some(*hex))
            .collect())
    }
    fn respawn_request_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        prompt: Symbol,
        choices: RequestChoices,
    ) -> Result<NativeRequest> {
        if self.next_request == u64::MAX && objects.token(step)?.is_none() {
            return Err(unsupported("request generation counter exhausted"));
        }
        Ok(NativeRequest {
            controlled: None,
            id: RequestToken::Generated(0),
            owner: self
                .planning
                .actor
                .filter(|id| !self.symbols.name(*id).is_empty())
                .unwrap_or(objects.phases.k(Field::SystemName)),
            prompt,
            movement_range: None,
            pre_action_range: None,
            choices,
            can_skip: false,
            discard: false,
        })
    }
    fn respawn_place_bound(
        &self,
        objects: &BoundObjects,
        unit: Symbol,
        hex: Hex,
        remove: bool,
    ) -> Result<RespawnPlan> {
        Ok(RespawnPlan::Place {
            placement: self.prepare_spatial_bound(objects, unit, hex, None)?,
            remove,
            unit,
        })
    }
    pub(super) fn preflight_minion_respawn_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<RespawnPlan> {
        self.sync_card_collections_bound(objects)?;
        let team = self.passive_text_bound(objects, step, Field::ResourceTeam)?;
        let teams = objects.phases.root(&objects.arena, Field::Teams)?;
        if objects.arena.field(teams, team).is_none() {
            return Ok(RespawnPlan::Done {
                clear: false,
                remove: false,
            });
        }
        let lane_bound = self.spatial_bool_bound(objects, step, Field::LaneBound)?;
        let legacy = if lane_bound {
            None
        } else {
            let Some(key) = self
                .passive_optional_text_bound(objects, step, Field::UnitKey)?
                .filter(|id| !self.symbols.name(*id).is_empty())
            else {
                return Ok(RespawnPlan::Done {
                    clear: false,
                    remove: false,
                });
            };
            let Some(ObjectValue::Atom(Datum::Text(id))) = objects.arena.context_value(key) else {
                return Ok(RespawnPlan::Done {
                    clear: false,
                    remove: false,
                });
            };
            if !self
                .world
                .entity(id)
                .is_some_and(|e| e.kind == EntityKind::Minion && e.roster == Some(team))
                || self.world.position(id).is_some()
            {
                return Ok(RespawnPlan::Done {
                    clear: false,
                    remove: false,
                });
            }
            Some(id)
        };
        let filters = self.respawn_filters_bound(objects, step)?;
        let stored = objects
            .arena
            .context_value(objects.phases.k(Field::LaneRespawnHex))
            .filter(|v| !matches!(v, ObjectValue::Atom(Datum::Null)));
        let pending = objects
            .arena
            .field(step, objects.pending)
            .is_some_and(|v| objects.arena.truthy(&v));
        let selection = objects.selected(step, self.selection_key);
        let mut clear = false;
        if pending {
            if lane_bound
                && let Some(stored) = &stored
                && let Some(ObjectValue::Atom(Datum::Text(selected))) = &selection
            {
                let (hex, _) = self.spatial_hex_bound(objects, stored)?;
                let supply = self.respawn_supply(team, hex);
                if let Some(id) = supply.into_iter().find(|id| id == selected) {
                    // The source checks only occupancy again in its second stage.
                    if self.world.occupant(hex).is_none() {
                        return self.respawn_place_bound(objects, id, hex, true);
                    }
                }
                return Ok(RespawnPlan::Done {
                    clear: false,
                    remove: true,
                });
            }
            if (!lane_bound || stored.is_none())
                && let Some(ObjectValue::Object(id)) = &selection
                && objects.arena.kind(*id) == ObjectKind::Dict
            {
                let hex = self
                    .spatial_hex_bound(objects, selection.as_ref().unwrap())
                    .ok()
                    .map(|(h, _)| h);
                let valid = if let Some(hex) = hex {
                    !self
                        .respawn_hexes_bound(objects, &filters, &[hex])?
                        .is_empty()
                } else {
                    false
                };
                if valid {
                    let hex = hex.unwrap();
                    if let Some(unit) = legacy {
                        return self.respawn_place_bound(objects, unit, hex, false);
                    }
                    let candidates = self.respawn_supply(team, hex);
                    if candidates.is_empty() {
                        return Ok(RespawnPlan::Done {
                            clear: false,
                            remove: false,
                        });
                    }
                    if candidates.len() == 1 {
                        return self.respawn_place_bound(objects, candidates[0], hex, false);
                    }
                    let prompt =
                        Arc::make_mut(&mut self.symbols).intern("Choose a minion to respawn.");
                    let values = candidates
                        .into_iter()
                        .map(|id| (id, self.world.entity(id).unwrap().minion_kind.unwrap()))
                        .collect();
                    let request = self.respawn_request_bound(
                        objects,
                        step,
                        prompt,
                        RequestChoices::Minions(values),
                    )?;
                    return Ok(RespawnPlan::Prompt {
                        request,
                        clear: false,
                        store: selection,
                    });
                }
                clear = lane_bound;
            }
        }
        let candidates = self.world.tile_hexes().to_vec();
        let mut choices = self.respawn_hexes_bound(objects, &filters, &candidates)?;
        if lane_bound {
            choices.retain(|hex| !self.respawn_supply(team, *hex).is_empty());
        }
        if choices.is_empty() {
            return Ok(RespawnPlan::Done {
                clear,
                remove: false,
            });
        }
        let text = legacy.map_or_else(
            || "Select space to respawn a minion.".to_owned(),
            |id| format!("Select space to respawn {}.", self.symbols.name(id)),
        );
        let prompt = Arc::make_mut(&mut self.symbols).intern(&text);
        let request =
            self.respawn_request_bound(objects, step, prompt, RequestChoices::Hexes(choices))?;
        Ok(RespawnPlan::Prompt {
            request,
            clear,
            store: None,
        })
    }
    pub(super) fn minion_respawn_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: RespawnPlan,
        events: &mut Vec<NativeEvent>,
    ) -> Result<Option<NativeRequest>> {
        objects.arena.symbols = self.symbols.clone();
        match plan {
            RespawnPlan::Done { clear, remove } => {
                if clear {
                    objects
                        .arena
                        .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                        .map_err(unsupported)?;
                }
                if remove {
                    objects
                        .arena
                        .remove_context(objects.phases.k(Field::LaneRespawnHex))
                        .map_err(unsupported)?;
                }
            }
            RespawnPlan::Prompt {
                request,
                clear,
                store,
            } => {
                if clear {
                    objects
                        .arena
                        .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                        .map_err(unsupported)?;
                }
                if let Some(hex) = store {
                    objects
                        .arena
                        .set_context(objects.phases.k(Field::LaneRespawnHex), hex)
                        .map_err(unsupported)?;
                }
                return Ok(Some(request));
            }
            RespawnPlan::Place {
                placement,
                remove,
                unit,
            } => {
                if remove {
                    objects
                        .arena
                        .remove_context(objects.phases.k(Field::LaneRespawnHex))
                        .map_err(unsupported)?;
                }
                self.apply_spatial_bound(objects, &placement)?;
                events.push(NativeEvent::UnitPlaced {
                    unit,
                    from: None,
                    to: placement.to,
                });
            }
        }
        Ok(None)
    }
}
