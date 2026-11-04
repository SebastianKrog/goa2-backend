//! Rune choice preserves retry, pending-token and optional metadata semantics.
use super::phases::Field;
use super::*;

pub(super) struct RunePlan {
    prompt: Symbol,
    output: Symbol,
    options: Vec<Symbol>,
    selected: Option<Symbol>,
    clear_input: bool,
    matches: Option<(Symbol, bool)>,
    mapped: Option<(Symbol, Symbol)>,
}
impl NativeSession {
    fn rune_options_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        field: Field,
    ) -> Result<Vec<Symbol>> {
        let Some(ObjectValue::Object(list)) = objects.arena.field(step, objects.phases.k(field))
        else {
            return Err(unsupported("rune options list missing"));
        };
        let allowed = [
            Field::RuneAxe,
            Field::RuneBird,
            Field::RuneAnvil,
            Field::RuneHorn,
        ]
        .map(|f| objects.phases.k(f));
        objects
            .arena
            .sequence(list)
            .ok_or_else(|| unsupported("rune options must be a sequence"))?
            .iter()
            .map(|value| match value {
                ObjectValue::Atom(Datum::Text(rune)) if allowed.contains(rune) => Ok(*rune),
                _ => Err(unsupported("unknown rune option")),
            })
            .collect()
    }
    pub(super) fn preflight_rune_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<RunePlan> {
        let options = self.rune_options_bound(objects, step, Field::Options)?;
        let prompt = self.passive_text_bound(objects, step, Field::Prompt)?;
        let output = self.passive_text_bound(objects, step, Field::Output)?;
        let pending = objects
            .arena
            .field(step, objects.pending)
            .filter(|v| objects.arena.truthy(v));
        if pending.as_ref().is_some_and(
            |v| !matches!(v, ObjectValue::Object(id) if objects.arena.fields(*id).is_some()),
        ) {
            return Err(unsupported("rune pending input must be a mapping"));
        }
        // A non-string Python repr cannot equal any of the four rune values.
        let selected = pending
            .as_ref()
            .and_then(|_| objects.selected(step, self.selection_key))
            .and_then(|v| match v {
                ObjectValue::Atom(Datum::Text(rune)) if options.contains(&rune) => Some(rune),
                _ => None,
            })
            .or_else(|| (options.len() == 1).then(|| options[0]));
        let mut matches = None;
        let mut mapped = None;
        if let Some(selected) = selected {
            if let Some(key) = self
                .passive_optional_text_bound(objects, step, Field::MatchesOutput)?
                .filter(|key| !self.symbols.name(*key).is_empty())
            {
                matches = Some((
                    key,
                    self.rune_options_bound(objects, step, Field::MatchingOptions)?
                        .contains(&selected),
                ));
            }
            if let Some(key) = self
                .passive_optional_text_bound(objects, step, Field::ValueOutput)?
                .filter(|key| !self.symbols.name(*key).is_empty())
            {
                let Some(ObjectValue::Object(values)) =
                    objects.arena.field(step, objects.phases.k(Field::ValueMap))
                else {
                    return Err(unsupported("rune value map missing"));
                };
                if objects.arena.fields(values).is_none() {
                    return Err(unsupported("rune value map must be a dictionary"));
                }
                if let Some(value) = objects.arena.field(values, selected) {
                    let ObjectValue::Atom(Datum::Text(value)) = value else {
                        return Err(unsupported("rune mapped value must be text"));
                    };
                    mapped = Some((key, value));
                }
            }
        }
        Ok(RunePlan {
            prompt,
            output,
            options,
            selected,
            clear_input: pending.is_some(),
            matches,
            mapped,
        })
    }
    pub(super) fn rune_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: RunePlan,
    ) -> Result<Option<NativeRequest>> {
        if plan.clear_input {
            // Source clears pending_input on every consumed attempt, retaining
            // pending_request_id and any aliases to the original input mapping.
            objects
                .arena
                .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
        }
        if let Some(selected) = plan.selected {
            objects
                .arena
                .set_context(plan.output, ObjectValue::Atom(Datum::Text(selected)))
                .map_err(unsupported)?;
            if let Some((key, matched)) = plan.matches {
                objects
                    .arena
                    .set_context(
                        key,
                        ObjectValue::Atom(if matched {
                            Datum::Bool(true)
                        } else {
                            Datum::Null
                        }),
                    )
                    .map_err(unsupported)?;
            }
            if let Some((key, value)) = plan.mapped {
                objects
                    .arena
                    .set_context(key, ObjectValue::Atom(Datum::Text(value)))
                    .map_err(unsupported)?;
            }
            return Ok(None);
        }
        let owner = self
            .planning
            .actor
            .filter(|id| !self.symbols.name(*id).is_empty())
            .map(|id| self.normalized_owner(id))
            .unwrap_or(objects.phases.k(Field::EmptyOwner));
        Ok(Some(NativeRequest {
            controlled: None,
            id: RequestToken::Generated(0),
            owner,
            prompt: plan.prompt,
            movement_range: None,
            pre_action_range: None,
            choices: RequestChoices::Runes(plan.options),
            can_skip: false,
            discard: false,
        }))
    }
}

impl NativeSession {
    pub(super) fn validate_inscription_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        selection: &Datum,
    ) -> Result<()> {
        let Some(ObjectValue::Object(placed)) = objects
            .arena
            .field(step, objects.phases.k(Field::PlacedRunes))
        else {
            return Err(unsupported("placed runes missing"));
        };
        let fields = objects
            .arena
            .fields(placed)
            .ok_or_else(|| unsupported("placed runes must be a mapping"))?;
        let remaining = [
            Field::RuneAxe,
            Field::RuneBird,
            Field::RuneAnvil,
            Field::RuneHorn,
        ]
        .map(|field| objects.phases.k(field))
        .into_iter()
        .filter(|rune| {
            !fields
                .iter()
                .any(|(_, value)| matches!(value, ObjectValue::Atom(Datum::Text(id)) if id == rune))
        })
        .collect::<Vec<_>>();
        if super::super::valid_rune_batch(selection, &remaining).is_none() {
            return Err(EngineError::InvalidInput(
                "Choose one remaining rune or complete the remaining rune slots".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn inscribe_runes_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        events: &mut Vec<NativeEvent>,
    ) -> Result<Option<NativeRequest>> {
        let hero = self.passive_text_bound(objects, step, Field::HeroId)?;
        let key = |field| objects.phases.k(field);
        let slots = [
            Field::RuneSlotOne,
            Field::RuneSlotTwo,
            Field::RuneSlotThree,
            Field::RuneSlotFour,
        ]
        .map(key);
        let all = [
            Field::RuneAxe,
            Field::RuneBird,
            Field::RuneAnvil,
            Field::RuneHorn,
        ]
        .map(key);
        let slots_key = key(Field::RuneSlots);
        let Some(ObjectValue::Object(placed_object)) =
            objects.arena.field(step, key(Field::PlacedRunes))
        else {
            return Err(unsupported("placed runes missing"));
        };
        let mut placed = objects
            .arena
            .fields(placed_object)
            .ok_or_else(|| unsupported("placed runes must be a mapping"))?
            .iter()
            .map(|(slot, value)| match value {
                ObjectValue::Atom(Datum::Text(rune))
                    if slots.contains(slot) && all.contains(rune) =>
                {
                    Ok((*slot, *rune))
                }
                _ => Err(unsupported("invalid placed rune")),
            })
            .collect::<Result<Vec<_>>>()?;
        let mut remaining = all
            .into_iter()
            .filter(|rune| !placed.iter().any(|(_, value)| value == rune))
            .collect::<Vec<_>>();
        if placed.len() + remaining.len() != slots.len() {
            return Err(unsupported("duplicate placed rune"));
        }
        let pending = objects
            .arena
            .field(step, objects.pending)
            .is_some_and(|v| objects.arena.truthy(&v));
        let mut chosen = if pending {
            objects
                .selected(step, self.selection_key)
                .and_then(|value| match value {
                    ObjectValue::Atom(value) => Some(value),
                    ObjectValue::Object(list)
                        if objects.arena.kind(list) == crate::codec::ObjectKind::List =>
                    {
                        objects
                            .arena
                            .sequence(list)?
                            .iter()
                            .map(|item| match item {
                                ObjectValue::Atom(Datum::Text(rune)) => Some(Datum::Text(*rune)),
                                _ => None,
                            })
                            .collect::<Option<Vec<_>>>()
                            .map(Datum::List)
                    }
                    _ => None,
                })
                .and_then(|selection| super::super::valid_rune_batch(&selection, &remaining))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        remaining.retain(|rune| !chosen.contains(rune));
        if remaining.len() == 1 {
            chosen.push(remaining.pop().unwrap());
        }
        if pending {
            objects
                .arena
                .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
        }
        for rune in chosen {
            let slot = slots[placed.len()];
            objects
                .arena
                .set_field(placed_object, slot, ObjectValue::Atom(Datum::Text(rune)))
                .map_err(unsupported)?;
            placed.push((slot, rune));
        }
        if remaining.is_empty() {
            if let Some(model) = self.hero_model_bound(objects, hero) {
                let values = Datum::Object(
                    placed
                        .iter()
                        .map(|(slot, rune)| (*slot, Datum::Text(*rune)))
                        .collect(),
                );
                let value = objects.arena.allocate(&values);
                objects
                    .arena
                    .set_field(model, slots_key, value)
                    .map_err(unsupported)?;
                events.push(NativeEvent::RunesPlaced { hero, placed });
            }
            return Ok(None);
        }
        Ok(Some(NativeRequest {
            id: RequestToken::Generated(0),
            owner: self.normalized_owner(hero),
            controlled: None,
            prompt: hero,
            movement_range: None,
            pre_action_range: None,
            choices: RequestChoices::RuneInscription {
                hero,
                placed,
                remaining,
            },
            can_skip: false,
            discard: false,
        }))
    }
}
