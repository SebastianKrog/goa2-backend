//! Live hex selectors share native scans and read only referenced context slots.
use super::phases::Field;
use super::*;
use crate::{
    data::{RegisterRead, SparseRegisters},
    spatial::{AdjacencyTag, HexFilter, Origin, SpatialContext},
};

pub(super) struct HexSelectionPlan {
    output: Symbol,
    prompt: Symbol,
    owner: Symbol,
    discard: bool,
    choices: Vec<Hex>,
    selected: Option<(Hex, Option<ObjectId>)>,
    skip: bool,
}
impl NativeSession {
    pub(super) fn spatial_bool_bound(
        &self,
        objects: &BoundObjects,
        object: ObjectId,
        field: Field,
    ) -> Result<bool> {
        match objects.arena.field(object, objects.phases.k(field)) {
            Some(ObjectValue::Atom(Datum::Bool(value))) => Ok(value),
            _ => Err(unsupported("spatial boolean field missing")),
        }
    }
    pub(super) fn spatial_int_bound(
        &self,
        objects: &BoundObjects,
        object: ObjectId,
        field: Field,
    ) -> Result<Option<i64>> {
        match objects.arena.field(object, objects.phases.k(field)) {
            None | Some(ObjectValue::Atom(Datum::Null)) => Ok(None),
            Some(ObjectValue::Atom(Datum::Int(value))) => Ok(Some(value)),
            _ => Err(unsupported("spatial integer field invalid")),
        }
    }
    pub(super) fn spatial_text_bound(
        &self,
        objects: &BoundObjects,
        object: ObjectId,
        field: Field,
    ) -> Result<Option<Symbol>> {
        Ok(self
            .passive_optional_text_bound(objects, object, field)?
            .filter(|value| !self.symbols.name(*value).is_empty()))
    }
    pub(super) fn filter_origin_bound(
        &self,
        objects: &BoundObjects,
        object: ObjectId,
        literal: Field,
        key: Field,
    ) -> Result<Origin> {
        Ok(Origin {
            literal: self.spatial_text_bound(objects, object, literal)?,
            key: self.spatial_text_bound(objects, object, key)?,
            frame: false,
        })
    }
    pub(super) fn hex_filter_bound(
        &self,
        objects: &BoundObjects,
        object: ObjectId,
    ) -> Result<HexFilter> {
        self.hex_filter_bound_depth(objects, object, 0)
    }
    fn hex_filter_bound_depth(
        &self,
        objects: &BoundObjects,
        object: ObjectId,
        depth: usize,
    ) -> Result<HexFilter> {
        if depth >= 128 {
            return Err(unsupported("composite hex filter depth exceeded"));
        }
        let kind = self.passive_text_bound(objects, object, Field::FilterType)?;
        Ok(match self.symbols.name(kind) {
            "adjacency_to_context_filter" => HexFilter::AdjacentContext(self.passive_text_bound(
                objects,
                object,
                Field::TargetKey,
            )?),
            "between_hexes_filter" => HexFilter::Between {
                from: self.passive_text_bound(objects, object, Field::FromHexKey)?,
                to: self.passive_text_bound(objects, object, Field::ToHexKey)?,
            },
            "relative_distance_filter" => {
                let op = self.passive_text_bound(objects, object, Field::Operator)?;
                let op = self.symbols.name(op);
                if !matches!(op, ">" | ">=" | "==" | "<=" | "<") {
                    return Err(unsupported("relative distance operator invalid"));
                }
                HexFilter::Relative {
                    origin: self.filter_origin_bound(
                        objects,
                        object,
                        Field::OriginId,
                        Field::OriginKey,
                    )?,
                    reference: self.passive_text_bound(objects, object, Field::ReferenceKey)?,
                    comparison: crate::queries::Comparison::parse(op),
                }
            }
            "terrain_filter" => {
                HexFilter::Terrain(self.spatial_bool_bound(objects, object, Field::IsTerrain)?)
            }
            "has_previous_slot_card_filter" => HexFilter::Reject,
            "exclude_identity_filter" => HexFilter::Exclude(self.unit_filter_texts_bound(
                objects,
                object,
                Field::ExcludeKeys,
            )?),
            "and_filter" | "or_filter" => {
                let children = self
                    .unit_filter_list_bound(objects, object, Field::Filters)?
                    .into_iter()
                    .map(|value| match value {
                        ObjectValue::Object(child) => {
                            self.hex_filter_bound_depth(objects, child, depth + 1)
                        }
                        _ => Err(unsupported("composite hex child must be a model")),
                    })
                    .collect::<Result<Vec<_>>>()?;
                if self.symbols.name(kind) == "or_filter" {
                    HexFilter::Or(children)
                } else {
                    HexFilter::And(children)
                }
            }
            "hex_batch_completable_filter" => {
                HexFilter::Batch(Box::new(self.batch_search_bound(objects, object)?))
            }
            "farthest_empty_adjacent_filter" => {
                HexFilter::Farthest(crate::batch::FarthestAdjacent {
                    origin: self.filter_origin_bound(
                        objects,
                        object,
                        Field::OriginId,
                        Field::OriginKey,
                    )?,
                    anchor: self.passive_text_bound(objects, object, Field::AnchorKey)?,
                    occupied: self.unit_filter_texts_bound(
                        objects,
                        object,
                        Field::OccupiedHexKeys,
                    )?,
                    freed: objects.phases.k(Field::BatchFreed),
                })
            }
            "occupied_filter" => HexFilter::Obstacle {
                wanted: self.spatial_bool_bound(objects, object, Field::IsObstacle)?,
                exclude: self.filter_origin_bound(
                    objects,
                    object,
                    Field::ExcludeId,
                    Field::ExcludeIdKey,
                )?,
            },
            "range_filter" => HexFilter::Range {
                origin: self.filter_origin_bound(
                    objects,
                    object,
                    Field::OriginId,
                    Field::OriginKey,
                )?,
                hex_key: self.spatial_text_bound(objects, object, Field::OriginHexKey)?,
                min: self.spatial_int_bound(objects, object, Field::MinRange)?,
                max: self.spatial_int_bound(objects, object, Field::MaxRange)?,
                min_key: self.spatial_text_bound(objects, object, Field::MinRangeKey)?,
                max_key: self.spatial_text_bound(objects, object, Field::MaxRangeKey)?,
                frame_range: false,
                geometric: match self.symbols.name(self.passive_text_bound(
                    objects,
                    object,
                    Field::DistanceMode,
                )?) {
                    "topology" => false,
                    "geometric" => true,
                    _ => return Err(unsupported("spatial distance mode invalid")),
                },
            },
            "movement_path_filter" => HexFilter::Movement {
                origin: self.filter_origin_bound(objects, object, Field::UnitId, Field::UnitKey)?,
                range: i32::try_from(
                    self.spatial_int_bound(objects, object, Field::Range)?
                        .ok_or_else(|| unsupported("movement filter range missing"))?,
                )
                .map_err(|_| unsupported("movement filter range overflow"))?,
                through: self.spatial_bool_bound(objects, object, Field::Through)?,
            },
            "in_straight_line_filter" | "not_in_straight_line_filter" => HexFilter::Straight {
                origin: self.filter_origin_bound(
                    objects,
                    object,
                    Field::OriginId,
                    Field::OriginKey,
                )?,
                invert: self.symbols.name(kind) == "not_in_straight_line_filter",
            },
            "straight_line_path_filter" => HexFilter::ClearLine {
                origin: self.filter_origin_bound(
                    objects,
                    object,
                    Field::OriginId,
                    Field::OriginKey,
                )?,
                through: self.spatial_bool_bound(objects, object, Field::Through)?,
            },
            "count_match_filter" => {
                let crate::targeting::UnitFilter::CountMatch(count) =
                    self.unit_filter_bound(objects, object, 0)?
                else {
                    unreachable!()
                };
                HexFilter::CountMatch(count)
            }
            "battle_zone_filter" => HexFilter::BattleZone,
            "spawn_point_team_filter" => HexFilter::SpawnTeam(
                match self.symbols.name(self.passive_text_bound(
                    objects,
                    object,
                    Field::Relation,
                )?) {
                    "FRIENDLY" => true,
                    "ENEMY" => false,
                    _ => return Err(unsupported("spawn team relation invalid")),
                },
            ),
            "spawn_point_filter" => HexFilter::Spawn {
                wanted: self.spatial_bool_bound(objects, object, Field::HasSpawn)?,
                minion_only: self.spatial_bool_bound(objects, object, Field::MinionOnly)?,
            },
            "adjacent_spawn_point_filter" => HexFilter::AdjacentSpawn {
                empty: self.spatial_bool_bound(objects, object, Field::IsEmpty)?,
                forbidden: self.spatial_bool_bound(objects, object, Field::MustNotHave)?,
                battle_only: self.spatial_bool_bound(objects, object, Field::BattleOnly)?,
                minion_only: self.spatial_bool_bound(objects, object, Field::MinionOnly)?,
            },
            "line_behind_target_filter" => HexFilter::BehindTarget {
                target: self.passive_text_bound(objects, object, Field::TargetKey)?,
                length: self
                    .spatial_int_bound(objects, object, Field::BehindLength)?
                    .ok_or_else(|| unsupported("behind-target length missing"))?,
                origin: Origin {
                    literal: self
                        .spatial_text_bound(objects, object, Field::OriginId)?
                        .filter(|id| !self.symbols.name(*id).is_empty()),
                    ..Default::default()
                },
            },
            "fast_travel_destination_filter" => HexFilter::FastTravel {
                source_card: self.spatial_text_bound(objects, object, Field::SourceCard)?,
                origin: Origin {
                    literal: self.spatial_text_bound(objects, object, Field::UnitId)?,
                    ..Default::default()
                },
                change: self.spatial_bool_bound(objects, object, Field::ZoneChange)?,
            },
            "adjacency_filter" => {
                let Some(ObjectValue::Object(tags)) = objects
                    .arena
                    .field(object, objects.phases.k(Field::TargetTags))
                else {
                    return Err(unsupported("adjacency tags missing"));
                };
                let tags = objects
                    .arena
                    .sequence(tags)
                    .ok_or_else(|| unsupported("adjacency tags must be a list"))?
                    .iter()
                    .map(|value| match value {
                        ObjectValue::Atom(Datum::Text(tag)) => match self.symbols.name(*tag) {
                            "FRIENDLY" => Ok(AdjacencyTag::Friendly),
                            "ENEMY" => Ok(AdjacencyTag::Enemy),
                            "HERO" => Ok(AdjacencyTag::Hero),
                            "MINION" => Ok(AdjacencyTag::Minion),
                            _ => Err(unsupported("unknown adjacency tag")),
                        },
                        _ => Err(unsupported("adjacency tag must be text")),
                    })
                    .collect::<Result<Vec<_>>>()?;
                HexFilter::AdjacentTags(tags)
            }
            other => return Err(unsupported(format!("live hex filter not bound: {other}"))),
        })
    }
    /// A bounded snapshot of referenced scalars/coordinates, never whole context.
    pub(super) fn spatial_inputs_bound(
        &self,
        objects: &BoundObjects,
        keys: impl IntoIterator<Item = Symbol>,
    ) -> SparseRegisters {
        let mut inputs = SparseRegisters::default();
        for key in keys {
            if inputs.get(key).is_some() {
                continue;
            }
            let Some(value) = objects.arena.context_value(key) else {
                continue;
            };
            let members = match &value {
                ObjectValue::Object(id) if objects.arena.is_mutable_list(*id) => {
                    objects.arena.sequence(*id).unwrap_or(&[]).to_vec()
                }
                _ => vec![value.clone()],
            };
            let hexes = members
                .iter()
                .filter_map(|value| match value {
                    ObjectValue::Object(id) if objects.arena.is_shared_frozen_model(*id) => self
                        .spatial_hex_bound(objects, value)
                        .ok()
                        .map(|(point, _)| point),
                    _ => None,
                })
                .collect();
            inputs.1.push((key, hexes));
            let datum = match value {
                ObjectValue::Atom(value) => value,
                ObjectValue::Object(id) => {
                    if let Some(fields) = objects.arena.fields(id) {
                        if fields.is_empty() {
                            Datum::Object(Vec::new())
                        } else {
                            let coordinates = self
                                .coordinates
                                .iter()
                                .filter_map(|key| {
                                    objects.arena.field(id, *key).and_then(|value| match value {
                                        ObjectValue::Atom(value) => Some((*key, value)),
                                        _ => None,
                                    })
                                })
                                .collect::<Vec<_>>();
                            if coordinates.is_empty() {
                                Datum::Object(vec![(key, Datum::Null)])
                            } else {
                                Datum::Object(coordinates)
                            }
                        }
                    } else if let Some(values) = objects.arena.sequence(id) {
                        Datum::List(
                            values
                                .iter()
                                .map(|value| match value {
                                    ObjectValue::Atom(value) => value.clone(),
                                    ObjectValue::Object(id) => Datum::Object(
                                        self.coordinates
                                            .iter()
                                            .filter_map(|key| {
                                                match objects.arena.field(*id, *key) {
                                                    Some(ObjectValue::Atom(value)) => {
                                                        Some((*key, value))
                                                    }
                                                    _ => None,
                                                }
                                            })
                                            .collect(),
                                    ),
                                    _ => Datum::Null,
                                })
                                .collect(),
                        )
                    } else if objects.arena.truthy(&ObjectValue::Object(id)) {
                        Datum::List(vec![Datum::Null])
                    } else {
                        Datum::List(Vec::new())
                    }
                }
                ObjectValue::Request(_) | ObjectValue::GeneratedStep(_) => Datum::Bool(true),
            };
            inputs.0.push((key, datum));
        }
        inputs
    }
    pub(super) fn spatial_filter_keys_bound(
        &self,
        objects: &BoundObjects,
        filter: &HexFilter,
        keys: &mut Vec<Symbol>,
    ) {
        match filter {
            HexFilter::AdjacentContext(key) => keys.push(*key),
            HexFilter::Between { from, to } => {
                keys.push(*from);
                keys.push(*to);
            }
            HexFilter::Relative {
                origin, reference, ..
            } => {
                keys.extend(origin.key);
                keys.push(*reference);
            }
            HexFilter::CountMatch(count) => {
                let mut lists = Vec::new();
                for filter in &count.filters {
                    self.unit_context_keys_bound(objects, filter, keys, &mut lists);
                }
                keys.extend(lists);
            }
            HexFilter::BehindTarget { target, origin, .. } => {
                keys.push(*target);
                keys.extend(origin.key);
            }
            HexFilter::Exclude(values) => keys.extend(values),
            HexFilter::And(filters) | HexFilter::Or(filters) => {
                for filter in filters {
                    self.spatial_filter_keys_bound(objects, filter, keys);
                }
            }
            HexFilter::Batch(search) => self.batch_context_keys_bound(objects, search, keys),
            HexFilter::Farthest(search) => {
                keys.extend(search.origin.key);
                keys.push(search.anchor);
                keys.push(search.freed);
                keys.extend(&search.occupied);
            }
            HexFilter::Obstacle {
                exclude: origin, ..
            }
            | HexFilter::Movement { origin, .. }
            | HexFilter::Straight { origin, .. }
            | HexFilter::ClearLine { origin, .. } => keys.extend(origin.key),
            HexFilter::FastTravel { origin, .. } => {
                keys.extend(origin.key);
                keys.extend(self.symbols.lookup("card"));
                keys.push(objects.phases.k(Field::CurrentCard));
            }
            HexFilter::Range {
                origin,
                hex_key,
                min_key,
                max_key,
                ..
            } => {
                keys.extend(origin.key);
                keys.extend(*hex_key);
                keys.extend(*min_key);
                keys.extend(*max_key);
            }
            _ => {}
        }
    }
    pub(super) fn preflight_hex_selection_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
        mandatory: bool,
    ) -> Result<HexSelectionPlan> {
        self.sync_card_collections_bound(objects)?;
        if objects
            .phases
            .tiles
            .iter()
            .any(|(_, consistent)| !consistent)
        {
            return Err(unsupported("hex selector board tile/key geometry drift"));
        }
        let Some(ObjectValue::Object(list)) =
            objects.arena.field(step, objects.phases.k(Field::Filters))
        else {
            return Err(unsupported("hex filter list missing"));
        };
        let filters = objects
            .arena
            .sequence(list)
            .ok_or_else(|| unsupported("hex filters must be a list"))?
            .iter()
            .map(|value| match value {
                ObjectValue::Object(filter) => self.hex_filter_bound(objects, *filter),
                _ => Err(unsupported("hex filter model missing")),
            })
            .collect::<Result<Vec<_>>>()?;
        let mut keys = Vec::new();
        for filter in &filters {
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
        let candidates = self.world.tile_hexes();
        let mut accepted = vec![true; candidates.len()];
        for filter in filters {
            filter
                .apply_scan(&context, candidates, &mut accepted)
                .map_err(unsupported)?;
        }
        let choices = candidates
            .iter()
            .zip(accepted)
            .filter_map(|(hex, yes)| yes.then_some(*hex))
            .collect::<Vec<_>>();
        let output = self.passive_text_bound(objects, step, Field::Output)?;
        let prompt = self.passive_text_bound(objects, step, Field::Prompt)?;
        let mut owner = self.spatial_text_bound(objects, step, Field::OverridePlayer)?;
        if owner.is_none()
            && let Some(key) = self.spatial_text_bound(objects, step, Field::OverridePlayerKey)?
        {
            owner = self.truthy_symbol_bound(objects, objects.arena.context_value(key))?;
        }
        let owner = self.normalized_owner(owner.or(self.planning.actor).unwrap_or(self.no_actor));
        let discard = self.spatial_text_bound(objects, step, Field::DecisionPurpose)?
            == Some(objects.phases.k(Field::DiscardPurpose));
        let mut selected = None;
        let mut skip = false;
        if !choices.is_empty() {
            if mandatory
                && choices.len() == 1
                && self.spatial_bool_bound(objects, step, Field::Auto)?
            {
                let index = self.world.tile_index(choices[0]).unwrap();
                let key = objects.arena.board_key(index).ok_or_else(|| {
                    unsupported("automatic hex selection requires board-key identity metadata")
                })?;
                selected = Some((choices[0], Some(key)));
            } else if let Some(pending) = objects
                .arena
                .field(step, objects.pending)
                .filter(|v| objects.arena.truthy(v))
            {
                if !matches!(pending,ObjectValue::Object(id) if objects.arena.fields(id).is_some())
                {
                    return Err(unsupported("hex pending input must be a mapping"));
                }
                if let Some(value) = objects.selected(step, self.selection_key) {
                    skip = !mandatory
                        && matches!(value,ObjectValue::Atom(Datum::Text(id)) if id==self.skip);
                    if !skip
                        && let Ok((hex, original)) = self.spatial_hex_bound(objects, &value)
                        && choices.contains(&hex)
                    {
                        selected = Some((hex, original));
                    }
                }
            }
        }
        if !choices.is_empty()
            && selected.is_none()
            && !skip
            && self.next_request == u64::MAX
            && objects.token(step)?.is_none()
        {
            return Err(unsupported("request generation counter exhausted"));
        }
        Ok(HexSelectionPlan {
            output,
            prompt,
            owner,
            discard,
            choices,
            selected,
            skip,
        })
    }
    pub(super) fn hex_selection_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: HexSelectionPlan,
        mandatory: bool,
    ) -> Result<Option<NativeRequest>> {
        if plan.choices.is_empty() {
            if mandatory {
                self.abort_objects(objects)?;
            } else {
                objects
                    .arena
                    .remove_context(plan.output)
                    .map_err(unsupported)?;
            }
            return Ok(None);
        }
        if plan.skip {
            objects
                .arena
                .remove_context(plan.output)
                .map_err(unsupported)?;
            objects.clear_pending(step)?;
            return Ok(None);
        }
        if let Some((hex, original)) = plan.selected {
            let value = if let Some(id) = original {
                ObjectValue::Object(id)
            } else {
                let Datum::Object(fields) = native_hex(hex, self.coordinates) else {
                    unreachable!()
                };
                objects
                    .arena
                    .shared_frozen_model(
                        fields
                            .into_iter()
                            .map(|(key, value)| (key, ObjectValue::Atom(value)))
                            .collect(),
                    )
                    .map_err(unsupported)?
            };
            objects
                .arena
                .set_context(plan.output, value)
                .map_err(unsupported)?;
            objects.clear_pending(step)?;
            return Ok(None);
        }
        Ok(Some(NativeRequest {
            controlled: None,
            id: RequestToken::Generated(0),
            owner: plan.owner,
            prompt: plan.prompt,
            movement_range: None,
            pre_action_range: None,
            choices: RequestChoices::Hexes(plan.choices),
            can_skip: !mandatory,
            discard: plan.discard,
        }))
    }
}
