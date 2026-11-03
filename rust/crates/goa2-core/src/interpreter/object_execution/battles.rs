//! Typed battle planning with ordered authoritative-arena mutations.
use super::phases::Field;
use super::*;
use crate::battle::{BattleMutation, BattleOp, BattleStep, PushSpec, RemovalSpec, Resolution};

pub(super) struct BattlePlan {
    result: Resolution,
    writes: Vec<BattleMutation>,
    zones: Vec<(Symbol, Symbol)>,
    waves: Vec<(Symbol, i64)>,
    origins: Vec<(Symbol, Hex, ObjectValue)>,
    clear_pending: bool,
    selected_hex: Option<(Hex, ObjectValue)>,
}
impl NativeSession {
    pub(super) fn battle_key(&self, name: &str) -> Result<Symbol> {
        self.symbols
            .lookup(name)
            .ok_or_else(|| unsupported(format!("battle operand missing: {name}")))
    }
    fn battle_text(&self, objects: &BoundObjects, step: ObjectId, name: &str) -> Result<Symbol> {
        match objects.arena.field(step, self.battle_key(name)?) {
            Some(ObjectValue::Atom(Datum::Text(value))) => Ok(value),
            _ => Err(unsupported("battle text operand invalid")),
        }
    }
    fn battle_optional(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        name: &str,
    ) -> Result<Option<Symbol>> {
        match objects.arena.field(step, self.battle_key(name)?) {
            Some(ObjectValue::Atom(Datum::Null)) => Ok(None),
            Some(ObjectValue::Atom(Datum::Text(value))) => Ok(Some(value)),
            _ => Err(unsupported("battle optional operand invalid")),
        }
    }
    pub(super) fn battle_root_map(&self, objects: &BoundObjects, name: &str) -> Result<ObjectId> {
        match objects.arena.root_field(self.battle_key(name)?) {
            Some(ObjectValue::Object(id)) if objects.arena.kind(id) == ObjectKind::Dict => Ok(id),
            _ => Err(unsupported("battle root mapping invalid")),
        }
    }
    fn battle_spawn_hex(
        &self,
        objects: &BoundObjects,
        zone: Symbol,
        index: usize,
        hex: Hex,
    ) -> Result<ObjectValue> {
        let object = |value| match value {
            Some(ObjectValue::Object(id)) => Ok(id),
            _ => Err(unsupported("battle spawn model binding missing")),
        };
        let board = object(objects.arena.root_field(self.battle_key("board")?))?;
        let zones = object(objects.arena.field(board, self.battle_key("zones")?))?;
        let zone = object(objects.arena.field(zones, zone))?;
        let spawns = object(objects.arena.field(zone, self.battle_key("spawn_points")?))?;
        let spawn = object(
            objects
                .arena
                .sequence(spawns)
                .and_then(|values| values.get(index))
                .cloned(),
        )?;
        let target = objects
            .arena
            .field(spawn, self.battle_key("location")?)
            .ok_or_else(|| unsupported("spawn location missing"))?;
        let (actual, original) = self.spatial_hex_bound(objects, &target)?;
        if actual != hex || original.is_none() {
            return Err(unsupported("battle spawn Hex/cache drift"));
        }
        Ok(target)
    }
    pub(super) fn preflight_battle_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
        family: BattleStep,
    ) -> Result<BattlePlan> {
        self.sync_card_collections_bound(objects)?;
        let zone_map = self.battle_root_map(objects, "battle_zones")?;
        let wave_map = self.battle_root_map(objects, "wave_counters")?;
        let zones = objects
            .arena
            .fields(zone_map)
            .unwrap()
            .iter()
            .map(|(lane, value)| {
                let zone = match value {
                    ObjectValue::Atom(Datum::Text(zone)) => *zone,
                    ObjectValue::Atom(Datum::Null) => self.world.definitions.zones.empty,
                    _ => return Err(unsupported("battle-zone mapping value invalid")),
                };
                Ok((*lane, zone))
            })
            .collect::<Result<Vec<_>>>()?;
        let waves = objects
            .arena
            .fields(wave_map)
            .unwrap()
            .iter()
            .map(|(lane, value)| {
                let count = match value {
                    ObjectValue::Atom(Datum::Int(count)) => *count,
                    ObjectValue::Atom(Datum::Bool(count)) => i64::from(*count),
                    _ => return Err(unsupported("wave counter must be an integer")),
                };
                Ok((*lane, count))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut origins = Vec::new();
        let live = match family {
            BattleStep::Check(_) => {
                BattleStep::Check(self.battle_optional(objects, step, "lane_id")?)
            }
            BattleStep::Minions(_) => {
                BattleStep::Minions(self.battle_optional(objects, step, "lane_id")?)
            }
            BattleStep::Push(_) => BattleStep::Push(PushSpec {
                lane: self.battle_text(objects, step, "lane_id")?,
                loser: self.battle_text(objects, step, "losing_team")?,
                target: self.battle_optional(objects, step, "target_zone_id")?,
                skip_wave: match objects
                    .arena
                    .field(step, self.battle_key("skip_wave_counter")?)
                {
                    Some(ObjectValue::Atom(Datum::Bool(value))) => value,
                    _ => return Err(unsupported("battle wave flag must be boolean")),
                },
            }),
            BattleStep::Casualties(_) => BattleStep::Casualties(RemovalSpec {
                loser: self.battle_text(objects, step, "losing_team")?,
                zone: self.battle_text(objects, step, "zone_id")?,
                remaining: match objects
                    .arena
                    .field(step, self.battle_key("remaining_to_remove")?)
                {
                    Some(ObjectValue::Atom(Datum::Int(value))) => value,
                    _ => return Err(unsupported("battle removal count must be an integer")),
                },
            }),
            BattleStep::Displace(_) => {
                let Some(ObjectValue::Object(list)) =
                    objects.arena.field(step, self.battle_key("displacements")?)
                else {
                    return Err(unsupported("battle displacement list missing"));
                };
                let mut items = Vec::new();
                for value in objects
                    .arena
                    .sequence(list)
                    .ok_or_else(|| unsupported("battle displacement list invalid"))?
                {
                    let ObjectValue::Object(pair) = value else {
                        return Err(unsupported("displacement pair invalid"));
                    };
                    let Some([ObjectValue::Atom(Datum::Text(unit)), origin]) =
                        objects.arena.sequence(*pair)
                    else {
                        return Err(unsupported("displacement pair shape invalid"));
                    };
                    let (hex, original) = self.spatial_hex_bound(objects, origin)?;
                    if original.is_none() {
                        return Err(unsupported("displacement origin must be an immutable Hex"));
                    }
                    items.push((*unit, hex));
                    origins.push((*unit, hex, origin.clone()));
                }
                BattleStep::Displace(items)
            }
        };
        let pending = objects
            .arena
            .field(step, objects.pending)
            .filter(|v| objects.arena.truthy(v));
        if pending.as_ref().is_some_and(
            |v| !matches!(v, ObjectValue::Object(id) if objects.arena.fields(*id).is_some()),
        ) {
            return Err(unsupported("battle pending input must be a mapping"));
        }
        let selected_object = objects.selected(step, self.selection_key);
        let selected_hex = if let Some(ObjectValue::Object(id)) = &selected_object {
            if objects.arena.is_shared_frozen_model(*id) {
                let value = selected_object.as_ref().unwrap();
                Some((self.spatial_hex_bound(objects, value)?.0, value.clone()))
            } else {
                None
            }
        } else {
            None
        };
        let selected = selected_object
            .map(|v| {
                objects
                    .arena
                    .typed_value(&v, Arc::make_mut(&mut self.symbols))
                    .map_err(unsupported)
            })
            .transpose()?;
        let mut preview = self.world.clone();
        preview.battle_zones = zones.clone();
        preview.waves = waves.clone();
        let favored = if matches!(objects.arena.root_field(self.battle_key("tie_breaker_team")?),
            Some(ObjectValue::Atom(Datum::Text(value))) if value == objects.phases.k(Field::Red))
        {
            objects.phases.k(Field::Red)
        } else {
            objects.phases.k(Field::Blue)
        };
        let rules = self
            .battle_rules
            .as_ref()
            .ok_or_else(|| unsupported("battle rules missing"))?;
        let (result, writes) = rules
            .resolve_traced(
                &mut preview,
                &self.symbols,
                live.clone(),
                selected.as_ref(),
                favored,
            )
            .map_err(|error| match error {
                crate::battle::BattleError::Rule(text) => EngineError::Rule(text),
                crate::battle::BattleError::Unsupported(text) => unsupported(text),
            })?;
        let clear_pending = pending.is_some()
            && writes
                .iter()
                .any(|write| matches!(write, BattleMutation::ClearPending));
        for write in &writes {
            match write {
                BattleMutation::Remove(unit) | BattleMutation::RemoveToken(unit) => {
                    self.preflight_remove_spatial_bound(objects, *unit)?
                }
                BattleMutation::Spawn {
                    unit,
                    zone,
                    source_index,
                    hex,
                } => {
                    self.preflight_remove_spatial_bound(objects, *unit)?;
                    self.tile_bound(objects, *hex)?;
                    self.battle_spawn_hex(objects, *zone, *source_index, *hex)?;
                }
                BattleMutation::BlockedSpawn {
                    unit,
                    zone,
                    source_index,
                    hex,
                } => origins.push((
                    *unit,
                    *hex,
                    self.battle_spawn_hex(objects, *zone, *source_index, *hex)?,
                )),
                _ => {}
            }
        }
        if let Resolution::Done(ops) = &result {
            if !ops.is_empty() {
                self.phase_generation_check(0, ops.len() as u64)?;
            }
        } else if self.next_request == u64::MAX && objects.token(step)?.is_none() {
            return Err(unsupported("request generation counter exhausted"));
        }
        Ok(BattlePlan {
            result,
            writes,
            zones,
            waves,
            origins,
            clear_pending,
            selected_hex,
        })
    }
    pub(super) fn battle_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: BattlePlan,
    ) -> Result<Option<NativeRequest>> {
        self.world.battle_zones = plan.zones;
        self.world.waves = plan.waves;
        if plan.clear_pending {
            objects
                .arena
                .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
        }
        for write in plan.writes {
            match write {
                BattleMutation::Wave(lane, value) => {
                    let map = self.battle_root_map(objects, "wave_counters")?;
                    objects
                        .arena
                        .set_field(map, lane, ObjectValue::Atom(Datum::Int(value)))
                        .map_err(unsupported)?;
                    if let Some((_, old)) = self.world.waves.iter_mut().find(|(id, _)| *id == lane)
                    {
                        *old = value;
                    } else {
                        self.world.waves.push((lane, value));
                    }
                }
                BattleMutation::Zone(lane, zone) => {
                    let map = self.battle_root_map(objects, "battle_zones")?;
                    objects
                        .arena
                        .set_field(map, lane, ObjectValue::Atom(Datum::Text(zone)))
                        .map_err(unsupported)?;
                    if let Some((_, old)) = self
                        .world
                        .battle_zones
                        .iter_mut()
                        .find(|(id, _)| *id == lane)
                    {
                        *old = zone;
                    } else {
                        self.world.battle_zones.push((lane, zone));
                    }
                }
                BattleMutation::Remove(unit) => {
                    self.remove_entity_spatial_bound(objects, unit)?;
                }
                BattleMutation::RemoveToken(unit) => {
                    self.remove_mine_spatial_bound(objects, unit)?;
                }
                BattleMutation::Spawn {
                    unit,
                    zone,
                    source_index,
                    hex,
                } => {
                    let value = self.battle_spawn_hex(objects, zone, source_index, hex)?;
                    let ObjectValue::Object(original) = value else {
                        unreachable!()
                    };
                    let placement =
                        self.prepare_spatial_bound(objects, unit, hex, Some(original))?;
                    self.apply_spatial_bound(objects, &placement)?;
                }
                BattleMutation::BlockedSpawn { .. } | BattleMutation::ClearPending => {}
            }
        }
        let Resolution::Done(ops) = plan.result else {
            let Resolution::Prompt(prompt) = plan.result else {
                unreachable!()
            };
            return Ok(Some(NativeRequest {
                controlled: None,
                id: RequestToken::Generated(0),
                owner: prompt.owner,
                prompt: prompt.prompt,
                movement_range: None,
                pre_action_range: None,
                choices: match prompt.choices {
                    crate::battle::Choices::Units(values) => RequestChoices::Units {
                        tokens: false,
                        values: values.into_iter().map(|id| (id, None)).collect(),
                    },
                    crate::battle::Choices::Hexes(values) => RequestChoices::ZoneHexes {
                        values,
                        unit: prompt.unit,
                    },
                },
                can_skip: false,
                discard: false,
            }));
        };
        let mut queued = Vec::with_capacity(ops.len());
        let mut origins = plan.origins;
        for op in ops {
            let (name, fields) = match op {
                BattleOp::Step(BattleStep::Push(spec)) => (
                    "lane_push",
                    vec![
                        ("lane_id", Datum::Text(spec.lane)),
                        ("losing_team", Datum::Text(spec.loser)),
                        (
                            "target_zone_id",
                            spec.target.map_or(Datum::Null, Datum::Text),
                        ),
                        ("skip_wave_counter", Datum::Bool(spec.skip_wave)),
                    ],
                ),
                BattleOp::Step(BattleStep::Casualties(spec)) => (
                    "minion_removal",
                    vec![
                        ("losing_team", Datum::Text(spec.loser)),
                        ("remaining_to_remove", Datum::Int(spec.remaining)),
                        ("zone_id", Datum::Text(spec.zone)),
                    ],
                ),
                BattleOp::Step(BattleStep::Displace(items)) => {
                    let child = self.phase_step(objects, "displacement")?;
                    let mut values = Vec::new();
                    for (unit, hex) in items {
                        let index = origins
                            .iter()
                            .position(|(id, h, _)| *id == unit && *h == hex)
                            .ok_or_else(|| {
                                unsupported("battle displacement origin binding missing")
                            })?;
                        let (_, _, origin) = origins.remove(index);
                        values.push(
                            objects
                                .arena
                                .tuple(vec![ObjectValue::Atom(Datum::Text(unit)), origin]),
                        );
                    }
                    let list = objects.arena.list(values);
                    objects
                        .arena
                        .set_field(child, self.battle_key("displacements")?, list)
                        .map_err(unsupported)?;
                    queued.push(child);
                    continue;
                }
                BattleOp::Remove(unit) => ("remove_unit", vec![("unit_id", Datum::Text(unit))]),
                BattleOp::Place(unit, hex) => (
                    "place_unit",
                    vec![
                        ("unit_id", Datum::Text(unit)),
                        ("target_hex_arg", native_hex(hex, self.coordinates)),
                    ],
                ),
                BattleOp::GameOver(outcome) => (
                    "game_over",
                    vec![
                        ("winner", outcome.team.map_or(Datum::Null, Datum::Text)),
                        (
                            "individual_winner_id",
                            outcome.hero.map_or(Datum::Null, Datum::Text),
                        ),
                        ("condition", Datum::Text(outcome.condition)),
                    ],
                ),
                _ => return Err(unsupported("unexpected arena battle continuation")),
            };
            let child = self.phase_step(objects, name)?;
            for (name, value) in fields {
                let value = if name == "target_hex_arg" {
                    let Datum::Object(fields) = value else {
                        unreachable!()
                    };
                    let hex = Datum::Object(fields.clone())
                        .hex(self.coordinates, &self.symbols)
                        .map_err(unsupported)?;
                    if let Some((_, original)) = plan
                        .selected_hex
                        .as_ref()
                        .filter(|(selected, _)| *selected == hex)
                    {
                        original.clone()
                    } else {
                        objects
                            .arena
                            .shared_frozen_model(
                                fields
                                    .into_iter()
                                    .map(|(k, v)| (k, ObjectValue::Atom(v)))
                                    .collect(),
                            )
                            .map_err(unsupported)?
                    }
                } else {
                    ObjectValue::Atom(value)
                };
                objects
                    .arena
                    .set_field(child, self.battle_key(name)?, value)
                    .map_err(unsupported)?;
            }
            queued.push(child);
        }
        self.push_live_steps_bound(objects, &queued)?;
        Ok(None)
    }
}
