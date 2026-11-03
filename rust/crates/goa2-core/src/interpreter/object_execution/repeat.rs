//! Repeat continuations preserve the live step and deepcopy each template root.
use super::phases::Field;
use super::*;

pub(super) enum RepeatPlan {
    Done,
    Lane,
    Decline,
    Accept {
        done: i64,
        again: bool,
        templates: Vec<ObjectId>,
    },
    Request(NativeRequest),
}
impl NativeSession {
    fn repeat_integer_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        field: Field,
    ) -> Result<i64> {
        match objects.arena.field(step, objects.phases.k(field)) {
            Some(ObjectValue::Atom(Datum::Int(value))) => Ok(value),
            Some(ObjectValue::Atom(Datum::Bool(value))) => Ok(i64::from(value)),
            _ => Err(unsupported("repeat counter requires integer")),
        }
    }
    pub(super) fn preflight_repeat_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<RepeatPlan> {
        let done = self.repeat_integer_bound(objects, step, Field::RepeatsDone)?;
        let max = self.repeat_integer_bound(objects, step, Field::MaxRepeats)?;
        if done >= max {
            return Ok(RepeatPlan::Done);
        }
        let Some(actor) = self
            .planning
            .actor
            .filter(|id| !self.symbols.name(*id).is_empty())
        else {
            return Ok(RepeatPlan::Done);
        };
        if objects
            .arena
            .root_field(objects.phases.k(Field::RepeatZones))
            .is_some_and(|value| objects.arena.truthy(&value))
            && !self.spatial_bool_bound(objects, step, Field::LanePushChecked)?
        {
            self.phase_generation_check(0, 1)?;
            return Ok(RepeatPlan::Lane);
        }
        self.sync_card_collections_bound(objects)?;
        if !self
            .world
            .can_repeat_action(Some(actor))
            .map_err(unsupported)?
        {
            return Ok(RepeatPlan::Done);
        }
        if let Some(pending) = objects
            .arena
            .field(step, objects.pending)
            .filter(|value| objects.arena.truthy(value))
        {
            let ObjectValue::Object(pending) = pending else {
                return Err(unsupported("repeat pending input requires dictionary"));
            };
            if !matches!(objects.arena.field(pending,self.selection_key),Some(ObjectValue::Atom(Datum::Text(value))) if value==self.yes)
            {
                return Ok(RepeatPlan::Decline);
            }
            let done = done
                .checked_add(1)
                .ok_or_else(|| unsupported("repeat counter outside i64"))?;
            let templates = objects.template_objects(step)?;
            for template in &templates {
                if objects.program(*template).is_none() {
                    return Err(unsupported("repeat template program missing"));
                }
            }
            return Ok(RepeatPlan::Accept {
                done,
                again: done < max,
                templates,
            });
        }
        if self.next_request == u64::MAX && objects.token(step)?.is_none() {
            return Err(unsupported("request generation counter exhausted"));
        }
        Ok(RepeatPlan::Request(NativeRequest {
            controlled: None,
            id: RequestToken::Generated(0),
            owner: actor,
            prompt: self.passive_text_bound(objects, step, Field::Prompt)?,
            movement_range: None,
            pre_action_range: None,
            choices: RequestChoices::Repeat { done, max },
            can_skip: false,
            discard: false,
        }))
    }
    pub(super) fn repeat_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: RepeatPlan,
    ) -> Result<Option<NativeRequest>> {
        match plan {
            RepeatPlan::Done => {}
            RepeatPlan::Request(request) => return Ok(Some(request)),
            RepeatPlan::Lane => {
                objects
                    .arena
                    .set_field(
                        step,
                        objects.phases.k(Field::LanePushChecked),
                        ObjectValue::Atom(Datum::Bool(true)),
                    )
                    .map_err(unsupported)?;
                let check = self.phase_step(objects, "check_lane_push")?;
                self.push_live_steps_bound(objects, &[check, step])?;
            }
            RepeatPlan::Decline => {
                objects
                    .arena
                    .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                    .map_err(unsupported)?;
            }
            RepeatPlan::Accept {
                done,
                again,
                templates,
            } => {
                objects
                    .arena
                    .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                    .map_err(unsupported)?;
                objects
                    .arena
                    .set_field(
                        step,
                        objects.phases.k(Field::RepeatsDone),
                        ObjectValue::Atom(Datum::Int(done)),
                    )
                    .map_err(unsupported)?;
                let mut children = Vec::with_capacity(templates.len() + usize::from(again));
                for template in templates {
                    let copy = objects.copy_template(template)?.0;
                    Self::validate_returned_step_bound(objects, copy)?;
                    children.push(copy);
                }
                if again {
                    objects
                        .arena
                        .set_field(
                            step,
                            objects.phases.k(Field::LanePushChecked),
                            ObjectValue::Atom(Datum::Bool(false)),
                        )
                        .map_err(unsupported)?;
                    children.push(step);
                }
                self.push_live_steps_bound(objects, &children)?;
            }
        }
        Ok(None)
    }
}
