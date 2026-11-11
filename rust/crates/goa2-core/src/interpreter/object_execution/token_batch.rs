//! Live token-batch rule operands and hypothetical assignment search.
use super::phases::Field;
use super::{
    Arc, BoundObjects, Datum, Hex, NativeSession, ObjectId, ObjectKind, ObjectValue, Result,
    Symbol, unsupported,
};
use crate::{
    batch::{BatchSearch, RemovalSearch},
    spatial::HexFilter,
};
impl NativeSession {
    fn batch_geometry_bound(&self, objects: &BoundObjects) -> Result<()> {
        if objects
            .phases
            .tiles
            .iter()
            .any(|(_, consistent)| !consistent)
        {
            return Err(unsupported("token batch board tile/key geometry drift"));
        }
        Ok(())
    }
    fn batch_hexes_bound(
        &self,
        objects: &BoundObjects,
        id: ObjectId,
        field: Field,
    ) -> Result<Vec<Hex>> {
        self.unit_filter_list_bound(objects, id, field)?
            .iter()
            .map(|value| self.spatial_hex_bound(objects, value).map(|v| v.0))
            .collect()
    }
    pub(super) fn batch_slots_bound(
        &self,
        objects: &BoundObjects,
        id: ObjectId,
    ) -> Result<Vec<Vec<HexFilter>>> {
        self.unit_filter_list_bound(objects, id, Field::SlotFilters)?
            .into_iter()
            .map(|slot| {
                let ObjectValue::Object(list) = slot else {
                    return Err(unsupported("batch slot must be a filter list"));
                };
                objects
                    .arena
                    .sequence(list)
                    .ok_or_else(|| unsupported("batch slot filter list missing"))?
                    .iter()
                    .map(|value| {
                        let ObjectValue::Object(filter) = value else {
                            return Err(unsupported("batch slot filter model missing"));
                        };
                        self.hex_filter_bound(objects, *filter)
                    })
                    .collect()
            })
            .collect()
    }
    pub(super) fn batch_search_bound(
        &self,
        objects: &BoundObjects,
        id: ObjectId,
    ) -> Result<BatchSearch> {
        self.batch_geometry_bound(objects)?;
        Ok(BatchSearch {
            slot_index: self
                .spatial_int_bound(objects, id, Field::SlotIndex)?
                .ok_or_else(|| unsupported("batch slot index missing"))?,
            keys: self.unit_filter_texts_bound(objects, id, Field::SlotKeys)?,
            filters: self.batch_slots_bound(objects, id)?,
            free: self.batch_hexes_bound(objects, id, Field::FreeHexes)?,
            removable: self.batch_hexes_bound(objects, id, Field::RemovableHexes)?,
            budget: self
                .spatial_int_bound(objects, id, Field::RemovalBudget)?
                .ok_or_else(|| unsupported("batch removal budget missing"))?,
            freed_key: objects.phases.k(Field::BatchFreed),
        })
    }
    pub(super) fn removal_search_bound(
        &self,
        objects: &BoundObjects,
        id: ObjectId,
    ) -> Result<RemovalSearch> {
        self.batch_geometry_bound(objects)?;
        let kind = self.passive_text_bound(objects, id, Field::TokenType)?;
        Ok(RemovalSearch {
            batch: BatchSearch {
                slot_index: 0,
                keys: self.unit_filter_texts_bound(objects, id, Field::SlotKeys)?,
                filters: self.batch_slots_bound(objects, id)?,
                free: Vec::new(),
                removable: Vec::new(),
                budget: 0,
                freed_key: objects.phases.k(Field::BatchFreed),
            },
            on_board: self.token_on_board_bound(objects, kind)?,
            remaining: self
                .spatial_int_bound(objects, id, Field::RemainingRemovals)?
                .ok_or_else(|| unsupported("remaining removals missing"))?,
        })
    }
    pub(super) fn batch_context_keys_bound(
        &self,
        objects: &BoundObjects,
        search: &BatchSearch,
        keys: &mut Vec<Symbol>,
    ) {
        keys.extend(&search.keys);
        keys.push(search.freed_key);
        for slot in &search.filters {
            for filter in slot {
                self.spatial_filter_keys_bound(objects, filter, keys);
            }
        }
    }
}

pub(super) struct BatchPlan {
    kind: Symbol,
    slots: Vec<Vec<ObjectValue>>,
    keys: Vec<Symbol>,
    owner: Symbol,
    owner_key: Symbol,
    prefix: Symbol,
    prompt: Option<Symbol>,
    placed: Option<Symbol>,
    removals: usize,
}
impl NativeSession {
    fn batch_intern_bound(&mut self, objects: &mut BoundObjects, text: &str) -> Symbol {
        let id = Arc::make_mut(&mut self.symbols).intern(text);
        objects.arena.symbols = self.symbols.clone();
        id
    }
    pub(super) fn preflight_token_batch_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
    ) -> Result<Option<BatchPlan>> {
        let (_, kind) = self.token_effective_bound(objects, step)?;
        let count = self
            .spatial_int_bound(objects, step, Field::TokenCount)?
            .ok_or_else(|| unsupported("token batch count missing"))?;
        let (total, placed) = self.token_supply_bound(objects, kind)?;
        if count < 0 || count > total as i64 {
            return Ok(None);
        }
        let count = count as usize;
        let raw = self.unit_filter_list_bound(objects, step, Field::SlotFilters)?;
        if !raw.is_empty() && raw.len() != count {
            return Ok(None);
        }
        let mut slots = Vec::new();
        let mut typed = Vec::new();
        for i in 0..count {
            let values = if raw.is_empty() {
                Vec::new()
            } else {
                let ObjectValue::Object(list) = raw[i] else {
                    return Err(unsupported("token batch slot list missing"));
                };
                objects
                    .arena
                    .sequence(list)
                    .ok_or_else(|| unsupported("token batch slot list missing"))?
                    .to_vec()
            };
            let mut filters = values
                .iter()
                .map(|value| {
                    let ObjectValue::Object(id) = value else {
                        return Err(unsupported("token batch slot filter model missing"));
                    };
                    self.hex_filter_bound(objects, *id)
                })
                .collect::<Result<Vec<_>>>()?;
            filters.push(HexFilter::Obstacle {
                wanted: false,
                exclude: Default::default(),
            });
            slots.push(values);
            typed.push(filters);
        }
        if count > 0 {
            self.batch_geometry_bound(objects)?;
        }
        self.sync_card_collections_bound(objects)?;
        let prefix = self.passive_text_bound(objects, step, Field::KeyPrefix)?;
        let prefix_text = self.symbols.name(prefix).to_owned();
        let keys = (0..count)
            .map(|i| self.batch_intern_bound(objects, &format!("{prefix_text}_hex_{i}")))
            .collect::<Vec<_>>();
        let on_board = self.token_on_board_bound(objects, kind)?;
        let removals = count.saturating_sub(total - placed);
        let search = BatchSearch {
            slot_index: 0,
            keys: keys.clone(),
            filters: typed,
            free: Vec::new(),
            removable: on_board.iter().map(|(_, h)| *h).collect(),
            budget: removals as i64,
            freed_key: objects.phases.k(Field::BatchFreed),
        };
        let mut dependencies = Vec::new();
        self.batch_context_keys_bound(objects, &search, &mut dependencies);
        let inputs = self.spatial_inputs_bound(objects, dependencies);
        let context = crate::spatial::SpatialContext {
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
        if !search.feasible(&context).map_err(unsupported)? {
            return Ok(None);
        }
        let owner = self
            .spatial_text_bound(objects, step, Field::TokenOwner)?
            .or(self
                .planning
                .actor
                .filter(|id| !self.symbols.name(*id).is_empty()))
            .unwrap_or(self.world.definitions.effect_empty);
        let owner_key = self.batch_intern_bound(objects, &format!("{prefix_text}_owner"));
        let prompt = self.spatial_text_bound(objects, step, Field::OptInPrompt)?;
        let placed = self.spatial_text_bound(objects, step, Field::PlacedFlagKey)?;
        self.phase_generation_check(
            0,
            ((count + removals) * 2
                + usize::from(prompt.is_some()) * 2
                + usize::from(placed.is_some())) as u64,
        )?;
        Ok(Some(BatchPlan {
            kind,
            slots,
            keys,
            owner,
            owner_key,
            prefix,
            prompt,
            placed,
            removals,
        }))
    }
    fn batch_field_bound(
        objects: &mut BoundObjects,
        id: ObjectId,
        field: Field,
        value: ObjectValue,
    ) -> Result<()> {
        objects
            .arena
            .set_field(id, objects.phases.k(field), value)
            .map_err(unsupported)
    }
    fn batch_scalar_bound(
        objects: &mut BoundObjects,
        id: ObjectId,
        field: Field,
        value: Datum,
    ) -> Result<()> {
        Self::batch_field_bound(objects, id, field, ObjectValue::Atom(value))
    }
    fn batch_model_bound(objects: &mut BoundObjects, name: &str) -> Result<ObjectId> {
        let datum = objects
            .tokens
            .batch_defaults
            .get(name)
            .ok_or_else(|| unsupported("batch model defaults missing"))?
            .clone();
        let ObjectValue::Object(id) = objects.arena.allocate(&datum) else {
            return Err(unsupported("batch model must be an object"));
        };
        objects
            .arena
            .classify_recipe_object(id, &[], ObjectKind::Model)
            .map_err(unsupported)
    }
    fn batch_grid_value(objects: &mut BoundObjects, slots: &[Vec<ObjectValue>]) -> ObjectValue {
        let rows = slots
            .iter()
            .map(|slot| objects.arena.list(slot.clone()))
            .collect();
        objects.arena.list(rows)
    }
    fn batch_search_model_bound(
        objects: &mut BoundObjects,
        name: &str,
        keys: &[Symbol],
        slots: &[Vec<ObjectValue>],
    ) -> Result<ObjectId> {
        let id = Self::batch_model_bound(objects, name)?;
        let keys = objects.arena.list(
            keys.iter()
                .map(|key| ObjectValue::Atom(Datum::Text(*key)))
                .collect(),
        );
        Self::batch_field_bound(objects, id, Field::SlotKeys, keys)?;
        let grid = Self::batch_grid_value(objects, slots);
        Self::batch_field_bound(objects, id, Field::SlotFilters, grid)?;
        Ok(id)
    }
    pub(super) fn token_batch_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<BatchPlan>,
        mandatory: bool,
    ) -> Result<()> {
        use super::passive_bodies::Kind;
        let Some(plan) = plan else {
            if mandatory {
                self.abort_objects(objects)?;
            }
            return Ok(());
        };
        let program = objects.scheduler.as_ref().unwrap().clone();
        let recipes = program.passive_bodies.recipe(Kind::BatchPrimitives)?;
        let token_recipes = program.passive_bodies.recipe(Kind::TokenOverflow)?;
        let mut slots = plan.slots;
        for slot in &mut slots {
            slot.push(ObjectValue::Object(Self::batch_model_bound(
                objects, "obstacle",
            )?));
        }
        objects
            .arena
            .set_context(plan.owner_key, ObjectValue::Atom(Datum::Text(plan.owner)))
            .map_err(unsupported)?;
        let prefix = self.symbols.name(plan.prefix).to_owned();
        let gate = if plan.prompt.is_some() {
            Some(self.batch_intern_bound(objects, &format!("{prefix}_go")))
        } else {
            None
        };
        let mut steps = Vec::new();
        if let Some(prompt) = plan.prompt {
            let choice = self.batch_intern_bound(objects, &format!("{prefix}_optin"));
            let select = self.allocate_source_recipe_bound(objects, &recipes[1], &Datum::Null)?;
            for (field, value) in [
                (Field::Prompt, Datum::Text(prompt)),
                (Field::Output, Datum::Text(choice)),
                (Field::OverridePlayerKey, Datum::Text(plan.owner_key)),
            ] {
                Self::batch_scalar_bound(objects, select, field, value)?;
            }
            let condition =
                self.allocate_source_recipe_bound(objects, &recipes[2], &Datum::Null)?;
            Self::batch_scalar_bound(objects, condition, Field::InputKey, Datum::Text(choice))?;
            Self::batch_scalar_bound(
                objects,
                condition,
                Field::Output,
                Datum::Text(gate.unwrap()),
            )?;
            steps.extend([select, condition]);
        }
        for i in 0..plan.removals {
            let key = self.batch_intern_bound(objects, &format!("{prefix}_rm_{i}"));
            let select =
                self.allocate_source_recipe_bound(objects, &token_recipes[0], &Datum::Null)?;
            let prompt = self.batch_intern_bound(
                objects,
                &format!(
                    "Select a {} token to remove from the board.",
                    self.symbols.name(plan.kind)
                ),
            );
            for (field, value) in [
                (Field::Prompt, Datum::Text(prompt)),
                (Field::Output, Datum::Text(key)),
                (Field::OverridePlayerKey, Datum::Text(plan.owner_key)),
                (Field::ActiveIf, gate.map_or(Datum::Null, Datum::Text)),
            ] {
                Self::batch_scalar_bound(objects, select, field, value)?;
            }
            let mut filters = self.unit_filter_list_bound(objects, select, Field::Filters)?;
            let ObjectValue::Object(kind_filter) = filters[1] else {
                return Err(unsupported("batch token kind filter missing"));
            };
            Self::batch_scalar_bound(
                objects,
                kind_filter,
                Field::TokenType,
                Datum::Text(plan.kind),
            )?;
            let filter = Self::batch_search_model_bound(objects, "removal", &plan.keys, &slots)?;
            Self::batch_scalar_bound(objects, filter, Field::TokenType, Datum::Text(plan.kind))?;
            Self::batch_scalar_bound(
                objects,
                filter,
                Field::RemainingRemovals,
                Datum::Int((plan.removals - i) as i64),
            )?;
            filters.push(ObjectValue::Object(filter));
            let filters = objects.arena.list(filters);
            Self::batch_field_bound(objects, select, Field::Filters, filters)?;
            let remove =
                self.allocate_source_recipe_bound(objects, &token_recipes[1], &Datum::Null)?;
            Self::batch_scalar_bound(objects, remove, Field::TokenKey, Datum::Text(key))?;
            Self::batch_scalar_bound(
                objects,
                remove,
                Field::ActiveIf,
                gate.map_or(Datum::Null, Datum::Text),
            )?;
            steps.extend([select, remove]);
        }
        let default_overflow = self.batch_intern_bound(objects, "overflow_token_to_remove");
        for (i, key) in plan.keys.iter().enumerate() {
            let select = self.allocate_source_recipe_bound(objects, &recipes[0], &Datum::Null)?;
            let prompt = self.batch_intern_bound(
                objects,
                &format!(
                    "Place {} token {}/{}",
                    self.symbols.name(plan.kind),
                    i + 1,
                    plan.keys.len()
                ),
            );
            for (field, value) in [
                (Field::Prompt, Datum::Text(prompt)),
                (Field::Output, Datum::Text(*key)),
                (Field::OverridePlayerKey, Datum::Text(plan.owner_key)),
                (Field::ActiveIf, gate.map_or(Datum::Null, Datum::Text)),
            ] {
                Self::batch_scalar_bound(objects, select, field, value)?;
            }
            let filter = Self::batch_search_model_bound(objects, "hex", &plan.keys, &slots)?;
            Self::batch_scalar_bound(objects, filter, Field::SlotIndex, Datum::Int(i as i64))?;
            let mut filters = slots[i].clone();
            filters.push(ObjectValue::Object(filter));
            let filters = objects.arena.list(filters);
            Self::batch_field_bound(objects, select, Field::Filters, filters)?;
            let place =
                self.allocate_source_recipe_bound(objects, &token_recipes[2], &Datum::Null)?;
            let output = self.batch_intern_bound(objects, &format!("{prefix}_token_{i}"));
            for (field, value) in [
                (Field::TokenType, Datum::Text(plan.kind)),
                (Field::HexKey, Datum::Text(*key)),
                (Field::OwnerIdKey, Datum::Text(plan.owner_key)),
                (Field::Output, Datum::Text(output)),
                (Field::ExistingTokenKey, Datum::Null),
                (Field::OverflowKey, Datum::Text(default_overflow)),
                (Field::ImmuneActions, Datum::Bool(false)),
                (Field::ActiveIf, gate.map_or(Datum::Null, Datum::Text)),
            ] {
                Self::batch_scalar_bound(objects, place, field, value)?;
            }
            steps.extend([select, place]);
        }
        if let Some(flag) = plan.placed {
            let step = self.allocate_source_recipe_bound(objects, &recipes[3], &Datum::Null)?;
            Self::batch_scalar_bound(objects, step, Field::FlagKey, Datum::Text(flag))?;
            Self::batch_scalar_bound(
                objects,
                step,
                Field::ActiveIf,
                gate.map_or(Datum::Null, Datum::Text),
            )?;
            steps.push(step);
        }
        self.push_live_steps_bound(objects, &steps)
    }
}
