//! Sequential selection with live candidates and persistent list identity.
use super::phases::Field;
use super::{
    Arc, BoundObjects, Datum, NativeRequest, NativeSession, ObjectId, ObjectValue, RequestChoices,
    RequestToken, Result, Symbol, unsupported,
};
use crate::{data::SparseRegisters, spatial::SpatialContext, targeting::UnitCandidates};

pub(super) struct MultiSelectionPlan {
    output: Symbol,
    list: ObjectId,
    values: Vec<Symbol>,
    append: Option<Symbol>,
    clear_pending: bool,
    outcome: MultiOutcome,
}
enum MultiOutcome {
    Alias,
    Finish {
        abort: bool,
    },
    Prompt {
        choices: Vec<Symbol>,
        prompt: Symbol,
        owner: Symbol,
        max: i64,
        can_skip: bool,
    },
}
impl NativeSession {
    /// Only context slots observed by a filter need the hypothetical append/output
    /// overlay. Never clone the arena or mutate it during candidate preflight.
    fn multi_inputs_bound(
        &self,
        objects: &BoundObjects,
        mut inputs: SparseRegisters,
        updated: Option<(ObjectId, Symbol, &[Symbol])>,
        keys: &[Symbol],
    ) -> SparseRegisters {
        if let Some((list, output, values)) = updated {
            if keys.contains(&output) && !inputs.0.iter().any(|(key, _)| *key == output) {
                inputs.0.push((output, Datum::Null));
            }
            for (key, value) in &mut inputs.0 {
                if *key == output
                    || matches!(objects.arena.context_value(*key), Some(ObjectValue::Object(id)) if id == list)
                {
                    *value = Datum::List(values.iter().copied().map(Datum::Text).collect());
                }
            }
        }
        inputs
    }
    fn multi_candidates_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        values: &[Symbol],
        updated: Option<(ObjectId, Symbol)>,
    ) -> Result<Vec<Symbol>> {
        self.sync_card_collections_bound(objects)?;
        let target = self.passive_text_bound(objects, step, Field::TargetType)?;
        let updated = updated.map(|(list, output)| (list, output, values));
        let choices = match self.symbols.name(target) {
            "UNIT" | "UNIT_OR_TOKEN" => {
                let tokens = self.symbols.name(target) == "UNIT_OR_TOKEN";
                let filters = self.effective_unit_filters_bound(objects, step)?;
                let mut keys = Vec::new();
                let mut lists = Vec::new();
                for filter in &filters {
                    self.unit_context_keys_bound(objects, filter, &mut keys, &mut lists);
                }
                let inputs = self.multi_inputs_bound(
                    objects,
                    self.unit_inputs_bound(objects, &filters)?,
                    updated,
                    &keys,
                );
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
                self.world
                    .select_units(
                        if tokens {
                            UnitCandidates::UnitsAndTokens
                        } else {
                            UnitCandidates::Units
                        },
                        self.planning.actor,
                        &filters,
                        &context,
                    )
                    .map_err(unsupported)?
            }
            "HEX" => {
                if objects
                    .phases
                    .tiles
                    .iter()
                    .any(|(_, consistent)| !consistent)
                {
                    return Err(unsupported("multi selector board tile/key geometry drift"));
                }
                let filters = self
                    .unit_filter_list_bound(objects, step, Field::Filters)?
                    .into_iter()
                    .map(|value| match value {
                        ObjectValue::Object(id) => self.hex_filter_bound(objects, id),
                        _ => Err(unsupported("multi hex filter model missing")),
                    })
                    .collect::<Result<Vec<_>>>()?;
                let mut keys = Vec::new();
                for filter in &filters {
                    self.spatial_filter_keys_bound(objects, filter, &mut keys);
                }
                let inputs = self.spatial_inputs_bound(objects, keys.iter().copied());
                let inputs = self.multi_inputs_bound(objects, inputs, updated, &keys);
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
                let names = candidates
                    .iter()
                    .zip(accepted)
                    .filter_map(|(hex, yes)| {
                        let (q, r, s) = hex.coordinates();
                        yes.then(|| format!("({q}, {r}, {s})"))
                    })
                    .collect::<Vec<_>>();
                let choices = names
                    .iter()
                    .map(|name| {
                        self.symbols
                            .lookup(name)
                            .unwrap_or_else(|| Arc::make_mut(&mut self.symbols).intern(name))
                    })
                    .collect();
                objects.arena.symbols = self.symbols.clone();
                choices
            }
            // Source initializes no candidates for other target kinds.
            _ => Vec::new(),
        };
        Ok(choices
            .into_iter()
            .filter(|id| !values.contains(id))
            .collect())
    }
    pub(super) fn preflight_multi_selection_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        mandatory: bool,
    ) -> Result<MultiSelectionPlan> {
        let output = self.passive_text_bound(objects, step, Field::Output)?;
        let Some(ObjectValue::Object(list)) = objects
            .arena
            .field(step, objects.phases.k(Field::Selections))
        else {
            return Err(unsupported("multi selections list missing"));
        };
        if !objects.arena.is_mutable_list(list) {
            return Err(unsupported("multi selections require a mutable list"));
        }
        let mut values = self.unit_filter_texts_bound(objects, step, Field::Selections)?;
        let mut plan = MultiSelectionPlan {
            output,
            list,
            values: Vec::new(),
            append: None,
            clear_pending: false,
            outcome: MultiOutcome::Alias,
        };
        if self
            .planning
            .actor
            .is_none_or(|id| self.symbols.name(id).is_empty())
        {
            return Ok(plan);
        }
        let min = self
            .spatial_int_bound(objects, step, Field::MinSelections)?
            .ok_or_else(|| unsupported("multi minimum missing"))?;
        let max = self
            .spatial_int_bound(objects, step, Field::MaxSelections)?
            .ok_or_else(|| unsupported("multi maximum missing"))?;
        let pending = objects
            .arena
            .field(step, objects.pending)
            .filter(|v| objects.arena.truthy(v));
        if let Some(pending) = pending {
            if !matches!(pending, ObjectValue::Object(id) if objects.arena.fields(id).is_some()) {
                return Err(unsupported("multi pending input must be a mapping"));
            }
            let selection = objects
                .selected(step, self.selection_key)
                .unwrap_or(ObjectValue::Atom(Datum::Null));
            let done = matches!(&selection, ObjectValue::Atom(Datum::Text(id)) if *id == self.skip || self.symbols.name(*id) == "DONE");
            if done && values.len() as i64 >= min {
                plan.values = values;
                plan.outcome = MultiOutcome::Finish { abort: false };
                return Ok(plan);
            }
            plan.clear_pending = true;
            if !done {
                let text = match selection {
                    ObjectValue::Atom(value) => value.python_scalar_text(&self.symbols),
                    _ => None,
                }
                .ok_or_else(|| unsupported("multi aggregate input coercion not bound"))?;
                let choices = self.multi_candidates_bound(objects, step, &values, None)?;
                if let Some(id) = choices
                    .into_iter()
                    .find(|id| self.symbols.name(*id) == text)
                {
                    values.push(id);
                    plan.append = Some(id);
                    if values.len() as i64 >= max {
                        plan.values = values;
                        plan.outcome = MultiOutcome::Finish { abort: false };
                        return Ok(plan);
                    }
                }
            }
        }
        let choices = self.multi_candidates_bound(
            objects,
            step,
            &values,
            plan.append.map(|_| (list, output)),
        )?;
        let count = values.len() as i64;
        plan.values = values;
        plan.outcome = if choices.is_empty() {
            MultiOutcome::Finish {
                abort: mandatory && count < min,
            }
        } else {
            MultiOutcome::Prompt {
                choices,
                prompt: self.passive_text_bound(objects, step, Field::Prompt)?,
                owner: self.normalized_owner(self.planning.actor.unwrap()),
                max,
                can_skip: count >= min,
            }
        };
        Ok(plan)
    }
    pub(super) fn multi_selection_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: MultiSelectionPlan,
    ) -> Result<Option<NativeRequest>> {
        if let Some(id) = plan.append {
            objects
                .arena
                .append(plan.list, ObjectValue::Atom(Datum::Text(id)))
                .map_err(unsupported)?;
        }
        if plan.clear_pending {
            // MultiSelect clears pending_input only. submit_input normally cleared
            // the request token already, but injected/repeated steps may retain it.
            objects
                .arena
                .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
        }
        if matches!(plan.outcome, MultiOutcome::Alias) {
            objects
                .arena
                .set_context(plan.output, ObjectValue::Object(plan.list))
                .map_err(unsupported)?;
            return Ok(None);
        }
        if plan.append.is_some() || matches!(plan.outcome, MultiOutcome::Finish { .. }) {
            let copy = objects.arena.list(
                plan.values
                    .iter()
                    .copied()
                    .map(|id| ObjectValue::Atom(Datum::Text(id)))
                    .collect(),
            );
            objects
                .arena
                .set_context(plan.output, copy)
                .map_err(unsupported)?;
        }
        match plan.outcome {
            MultiOutcome::Alias => unreachable!(),
            MultiOutcome::Finish { abort } => {
                if abort {
                    self.abort_objects(objects)?;
                }
                Ok(None)
            }
            MultiOutcome::Prompt {
                choices,
                prompt,
                owner,
                max,
                can_skip,
            } => Ok(Some(NativeRequest {
                controlled: None,
                id: RequestToken::Generated(0),
                owner,
                prompt,
                movement_range: None,
                pre_action_range: None,
                choices: RequestChoices::Multiple {
                    values: choices,
                    count: plan.values.len(),
                    max,
                },
                can_skip,
                discard: false,
            })),
        }
    }
}
