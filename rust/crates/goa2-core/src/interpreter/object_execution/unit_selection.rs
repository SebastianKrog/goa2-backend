//! Live UNIT/UNIT_OR_TOKEN selectors share typed targeting and sparse context.
use super::phases::Field;
use super::*;
use crate::{
    spatial::SpatialContext,
    targeting::{Relation, UnitCandidates, UnitFilter},
    world::{ActionKind, EntityKind, MinionKind},
};

pub(super) struct UnitSelectionPlan {
    output: Symbol,
    prompt: Symbol,
    owner: Symbol,
    discard: bool,
    tokens: bool,
    choices: Vec<Symbol>,
    selected: Option<Symbol>,
    skip: bool,
}
impl NativeSession {
    pub(super) fn unit_filter_list_bound(
        &self,
        objects: &BoundObjects,
        object: ObjectId,
        field: Field,
    ) -> Result<Vec<ObjectValue>> {
        let Some(ObjectValue::Object(id)) = objects.arena.field(object, objects.phases.k(field))
        else {
            return Err(unsupported("unit filter list missing"));
        };
        Ok(objects
            .arena
            .sequence(id)
            .ok_or_else(|| unsupported("unit filter list invalid"))?
            .to_vec())
    }
    pub(super) fn unit_filter_texts_bound(
        &self,
        objects: &BoundObjects,
        object: ObjectId,
        field: Field,
    ) -> Result<Vec<Symbol>> {
        self.unit_filter_list_bound(objects, object, field)?
            .into_iter()
            .map(|v| match v {
                ObjectValue::Atom(Datum::Text(id)) => Ok(id),
                _ => Err(unsupported("unit filter string list invalid")),
            })
            .collect()
    }
    pub(super) fn unit_filter_bound(
        &self,
        objects: &BoundObjects,
        object: ObjectId,
        depth: usize,
    ) -> Result<UnitFilter> {
        if depth > 64 {
            return Err(unsupported("unit filter nesting exceeds native limit"));
        }
        let kind = self.passive_text_bound(objects, object, Field::FilterType)?;
        Ok(match self.symbols.name(kind) {
            "and_filter" | "or_filter" => {
                let filters = self
                    .unit_filter_list_bound(objects, object, Field::Filters)?
                    .into_iter()
                    .map(|v| match v {
                        ObjectValue::Object(id) => self.unit_filter_bound(objects, id, depth + 1),
                        _ => Err(unsupported("composite unit filter missing model")),
                    })
                    .collect::<Result<Vec<_>>>()?;
                if self.symbols.name(kind) == "or_filter" {
                    UnitFilter::Or(filters)
                } else {
                    UnitFilter::And(filters)
                }
            }
            "team_filter" => UnitFilter::Team(
                match self.symbols.name(self.passive_text_bound(
                    objects,
                    object,
                    Field::Relation,
                )?) {
                    "FRIENDLY" => Relation::Friendly,
                    "ENEMY" => Relation::Enemy,
                    "SELF" => Relation::SelfOnly,
                    _ => return Err(unsupported("unit team relation invalid")),
                },
            ),
            "unit_type_filter" => UnitFilter::Kind(
                match self.symbols.name(self.passive_text_bound(
                    objects,
                    object,
                    Field::UnitType,
                )?) {
                    "HERO" => EntityKind::Hero,
                    "MINION" => EntityKind::Minion,
                    "TOKEN" => EntityKind::Token,
                    _ => return Err(unsupported("unit kind invalid")),
                },
            ),
            "has_marker_filter" => UnitFilter::HasMarker(self.passive_text_bound(
                objects,
                object,
                Field::MarkerType,
            )?),
            "exclude_identity_filter" => UnitFilter::Exclude {
                self_: self.spatial_bool_bound(objects, object, Field::ExcludeSelf)?,
                keys: self.unit_filter_texts_bound(objects, object, Field::ExcludeKeys)?,
            },
            "immunity_filter" => UnitFilter::Immunity,
            "range_filter" => UnitFilter::Range(self.hex_filter_bound(objects, object)?),
            "adjacency_filter"
            | "line_behind_target_filter"
            | "between_hexes_filter"
            | "clear_line_of_sight_filter"
            | "same_direction_from_origin_filter"
            | "relative_distance_filter"
            | "in_straight_line_filter"
            | "adjacent_to_terrain_filter"
            | "not_in_straight_line_filter" => {
                UnitFilter::Spatial(self.hex_filter_bound(objects, object)?)
            }
            "space_behind_empty_filter" => UnitFilter::BehindEmpty(self.filter_origin_bound(
                objects,
                object,
                Field::OriginId,
                Field::OriginKey,
            )?),
            "adjacent_to_token_in_range_filter" => UnitFilter::AdjacentToken {
                kind: self.passive_text_bound(objects, object, Field::TokenType)?,
                max_range: self.spatial_int_bound(objects, object, Field::MaxRange)?
                    .ok_or_else(|| unsupported("token range missing"))?,
            },
            "adjacency_to_context_filter" => UnitFilter::AdjacentContext(self.passive_text_bound(
                objects,
                object,
                Field::TargetKey,
            )?),
            "has_empty_neighbor_filter" => UnitFilter::EmptyNeighbor,
            "can_be_moved_filter" => {
                UnitFilter::Displacement(crate::validation::DisplacementKind::Move)
            }
            "can_be_placed_filter" => {
                UnitFilter::Displacement(crate::validation::DisplacementKind::Place)
            }
            "remained_in_place_filter" | "has_turn_start_position_filter" => {
                let snapshot = objects.phases.root(&objects.arena, Field::Snapshot)?;
                let positions = objects
                    .arena
                    .fields(snapshot)
                    .ok_or_else(|| unsupported("turn-start positions must be a mapping"))?
                    .iter()
                    .map(|(id, value)| Ok((*id, self.spatial_hex_bound(objects, value)?.0)))
                    .collect::<Result<Vec<_>>>()?;
                UnitFilter::TurnStartPositions {
                    positions,
                    stationary: self.symbols.name(kind) == "remained_in_place_filter",
                }
            }
            "has_straight_line_destination_filter" => {
                let minimum = self
                    .spatial_int_bound(objects, object, Field::PushDistance)?
                    .unwrap_or(2);
                UnitFilter::StraightDestination {
                    minimum,
                    maximum: self
                        .spatial_int_bound(objects, object, Field::MaxDistance)?
                        .unwrap_or(minimum),
                    through: self.spatial_bool_bound(objects, object, Field::Through)?,
                }
            }
            "adjacent_to_obstacles_filter" => UnitFilter::AdjacentObstacles(
                self.spatial_int_bound(objects, object, Field::MinCount)?
                    .unwrap_or(3),
            ),
            "unit_on_spawn_point_filter" => UnitFilter::OnSpawnPoint,
            "context_ids_filter" => {
                UnitFilter::ContextIds(self.passive_text_bound(objects, object, Field::IdsKey)?)
            }
            "hero_piece_filter" => UnitFilter::Piece {
                exclude_acting: self.spatial_bool_bound(objects, object, Field::ExcludeActing)?,
            },
            "token_removal_completable_filter" => {
                UnitFilter::BatchRemoval(Box::new(self.removal_search_bound(objects, object)?))
            }
            "token_type_filter" => UnitFilter::TokenType(self.passive_optional_text_bound(
                objects,
                object,
                Field::TokenType,
            )?),
            "minion_type_filter" => UnitFilter::MinionTypes(
                self.unit_filter_texts_bound(objects, object, Field::MinionTypes)?
                    .into_iter()
                    .map(|id| match self.symbols.name(id) {
                        "MELEE" => Ok(MinionKind::Melee),
                        "RANGED" => Ok(MinionKind::Ranged),
                        "HEAVY" => Ok(MinionKind::Heavy),
                        _ => Err(unsupported("minion kind invalid")),
                    })
                    .collect::<Result<Vec<_>>>()?,
            ),
            "terrain_filter" => {
                UnitFilter::Terrain(self.spatial_bool_bound(objects, object, Field::IsTerrain)?)
            }
            "count_match_filter" => UnitFilter::CountMatch(crate::targeting::CandidateCount {
                filters: self
                    .unit_filter_list_bound(objects, object, Field::SubFilters)?
                    .into_iter()
                    .map(|v| match v {
                        ObjectValue::Object(id) => self.unit_filter_bound(objects, id, depth + 1),
                        _ => Err(unsupported("count subfilter missing")),
                    })
                    .collect::<Result<Vec<_>>>()?,
                min: self
                    .spatial_int_bound(objects, object, Field::MinCount)?
                    .ok_or_else(|| unsupported("count minimum missing"))?,
                max: self.spatial_int_bound(objects, object, Field::MaxCount)?,
                tokens: self.spatial_bool_bound(objects, object, Field::IncludeTokens)?,
                origin: self
                    .symbols
                    .lookup("_cmf_origin_hex")
                    .ok_or_else(|| unsupported("count origin key missing"))?,
            }),
            "cards_in_container_filter" => UnitFilter::Cards {
                container: self.passive_text_bound(objects, object, Field::FilterContainer)?,
                min: self.spatial_int_bound(objects, object, Field::MinCards)?,
                max: self.spatial_int_bound(objects, object, Field::MaxCards)?,
                visible: self.spatial_bool_bound(objects, object, Field::ExcludeFacedown)?,
            },
            "co_mover_valid_hex_filter" => UnitFilter::Spatial(crate::spatial::HexFilter::Reject),
            "has_previous_slot_card_filter" => UnitFilter::PreviousSlotCard,
            "has_unresolved_card_filter" => UnitFilter::UnresolvedCard,
            "has_resolved_card_filter" => UnitFilter::ResolvedCard,
            "played_card_filter" => {
                let action = self
                    .passive_optional_text_bound(objects, object, Field::ActionType)?
                    .map(|id| match self.symbols.name(id) {
                        "MOVEMENT" => Ok(ActionKind::Movement),
                        "ATTACK" => Ok(ActionKind::Attack),
                        "SKILL" => Ok(ActionKind::Skill),
                        "DEFENSE" => Ok(ActionKind::Defense),
                        "DEFENSE_SKILL" => Ok(ActionKind::DefenseSkill),
                        "HOLD" => Ok(ActionKind::Hold),
                        "CLEAR" => Ok(ActionKind::Clear),
                        "FAST_TRAVEL" => Ok(ActionKind::FastTravel),
                        _ => Err(unsupported("played action kind invalid")),
                    })
                    .transpose()?;
                let color = self.passive_optional_text_bound(objects, object, Field::CardColor)?;
                if let Some(color) = color {
                    crate::card_selection::validate_color(self.symbols.name(color))
                        .map_err(unsupported)?;
                }
                UnitFilter::PlayedCard { action, color }
            }
            other => return Err(unsupported(format!("live unit filter not bound: {other}"))),
        })
    }
    pub(super) fn unit_context_keys_bound(
        &self,
        objects: &BoundObjects,
        filter: &UnitFilter,
        keys: &mut Vec<Symbol>,
        lists: &mut Vec<Symbol>,
    ) {
        match filter {
            UnitFilter::BatchRemoval(search) => {
                self.batch_context_keys_bound(objects, &search.batch, keys)
            }
            UnitFilter::CountMatch(count) => {
                for filter in &count.filters {
                    self.unit_context_keys_bound(objects, filter, keys, lists);
                }
            }
            UnitFilter::Or(filters) | UnitFilter::And(filters) => {
                for f in filters {
                    self.unit_context_keys_bound(objects, f, keys, lists);
                }
            }
            UnitFilter::Range(f) | UnitFilter::Spatial(f) => {
                self.spatial_filter_keys_bound(objects, f, keys)
            }
            UnitFilter::BehindEmpty(origin) => keys.extend(origin.key),
            UnitFilter::AdjacentContext(key) => keys.push(*key),
            UnitFilter::Exclude { keys: values, .. } => {
                keys.extend(values);
                lists.extend(values);
            }
            UnitFilter::ContextIds(key) => {
                keys.push(*key);
                lists.push(*key);
            }
            UnitFilter::Immunity => keys.extend(
                [
                    Field::CurrentAction,
                    Field::PerformingCard,
                    Field::PerformingOwner,
                ]
                .map(|f| objects.phases.k(f)),
            ),
            _ => {}
        }
    }
    pub(super) fn effective_unit_filters_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Vec<UnitFilter>> {
        let mut filters = self
            .unit_filter_list_bound(objects, step, Field::Filters)?
            .into_iter()
            .map(|v| match v {
                ObjectValue::Object(id) => self.unit_filter_bound(objects, id, 0),
                _ => Err(unsupported("unit filter model missing")),
            })
            .collect::<Result<Vec<_>>>()?;
        if !self.spatial_bool_bound(objects, step, Field::SkipSelf)?
            && !filters
                .iter()
                .any(|f| matches!(f, UnitFilter::Exclude { self_: true, .. }))
        {
            filters.push(UnitFilter::Exclude {
                self_: true,
                keys: Vec::new(),
            });
        }
        if !self.spatial_bool_bound(objects, step, Field::SkipImmunity)?
            && !filters.iter().any(|f| matches!(f, UnitFilter::Immunity))
        {
            filters.push(UnitFilter::Immunity);
        }
        Ok(filters)
    }
    pub(super) fn unit_inputs_bound(
        &self,
        objects: &BoundObjects,
        filters: &[UnitFilter],
    ) -> Result<crate::data::SparseRegisters> {
        let mut keys = Vec::new();
        let mut lists = Vec::new();
        for filter in filters {
            self.unit_context_keys_bound(objects, filter, &mut keys, &mut lists);
        }
        let mut inputs = self.spatial_inputs_bound(objects, keys);
        for key in lists {
            if let Some(ObjectValue::Object(id)) = objects.arena.context_value(key)
                && let Some(values) = objects.arena.sequence(id)
            {
                let list = values
                    .iter()
                    .map(|v| match v {
                        ObjectValue::Atom(v) => Ok(v.clone()),
                        _ => Err(unsupported(
                            "aggregate unit context list coercion not bound",
                        )),
                    })
                    .collect::<Result<Vec<_>>>()?;
                if let Some((_, value)) = inputs.0.iter_mut().find(|(k, _)| *k == key) {
                    *value = Datum::List(list);
                }
            }
        }
        Ok(inputs)
    }
    pub(super) fn preflight_unit_selection_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
        mandatory: bool,
    ) -> Result<UnitSelectionPlan> {
        self.sync_card_collections_bound(objects)?;
        let tokens =
            match self
                .symbols
                .name(self.passive_text_bound(objects, step, Field::TargetType)?)
            {
                "UNIT" => false,
                "UNIT_OR_TOKEN" => true,
                _ => return Err(unsupported("unit selector target kind invalid")),
            };
        let filters = self.effective_unit_filters_bound(objects, step)?;
        let mut owner = self.spatial_text_bound(objects, step, Field::OverridePlayer)?;
        if owner.is_none()
            && let Some(key) = self.spatial_text_bound(objects, step, Field::OverridePlayerKey)?
        {
            owner = self.truthy_symbol_bound(objects, objects.arena.context_value(key))?;
        }
        let actor = owner.or(self.planning.actor);
        let owner = self.normalized_owner(actor.unwrap_or(self.no_actor));
        let inputs = self.unit_inputs_bound(objects, &filters)?;
        let spatial = SpatialContext {
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
        let choices = self
            .world
            .select_units(
                if tokens {
                    UnitCandidates::UnitsAndTokens
                } else {
                    UnitCandidates::UnitsWithIllusions
                },
                actor,
                &filters,
                &spatial,
            )
            .map_err(unsupported)?;
        let mut selected = None;
        let mut skip = false;
        if !choices.is_empty() {
            if mandatory
                && choices.len() == 1
                && self.spatial_bool_bound(objects, step, Field::Auto)?
            {
                selected = Some(choices[0]);
            } else if let Some(pending) = objects
                .arena
                .field(step, objects.pending)
                .filter(|v| objects.arena.truthy(v))
            {
                if !matches!(pending,ObjectValue::Object(id) if objects.arena.fields(id).is_some())
                {
                    return Err(unsupported("unit pending input must be a mapping"));
                }
                if let Some(ObjectValue::Atom(Datum::Text(id))) =
                    objects.selected(step, self.selection_key)
                {
                    skip = !mandatory && id == self.skip;
                    if !skip && choices.contains(&id) {
                        selected = Some(id);
                    }
                }
            }
        }
        Ok(UnitSelectionPlan {
            output: self.passive_text_bound(objects, step, Field::Output)?,
            prompt: self.passive_text_bound(objects, step, Field::Prompt)?,
            owner,
            discard: self.spatial_text_bound(objects, step, Field::DecisionPurpose)?
                == Some(objects.phases.k(Field::DiscardPurpose)),
            tokens,
            choices,
            selected,
            skip,
        })
    }
    pub(super) fn unit_selection_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: UnitSelectionPlan,
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
        if let Some(id) = plan.selected {
            objects
                .arena
                .set_context(plan.output, ObjectValue::Atom(Datum::Text(id)))
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
            choices: RequestChoices::Units {
                tokens: plan.tokens,
                values: plan.choices.into_iter().map(|id| (id, None)).collect(),
            },
            can_skip: !mandatory,
            discard: plan.discard,
        }))
    }
}
