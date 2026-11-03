//! Reproduce GameStep's after-validator at source StepResult/AnyStep boundaries.
use super::*;
impl NativeSession {
    fn isolate_nested_step_value(
        objects: &mut BoundObjects,
        value: ObjectValue,
        depth: usize,
    ) -> Result<(ObjectValue, bool)> {
        if depth >= 128 {
            return Err(unsupported("nested step isolation depth exceeded"));
        }
        let ObjectValue::Object(id) = value else {
            return Ok((value, false));
        };
        if objects.arena.kind(id) == ObjectKind::Step {
            return Ok((ObjectValue::Object(objects.copy_template(id)?.0), true));
        }
        if let Some(values) = objects.arena.sequence(id) {
            let values = values.to_vec();
            let mut changed = false;
            let mut copies = Vec::new();
            for value in values {
                let (copy, different) = Self::isolate_nested_step_value(objects, value, depth + 1)?;
                changed |= different;
                copies.push(copy);
            }
            if !changed {
                return Ok((ObjectValue::Object(id), false));
            }
            return Ok((
                if !objects.arena.is_mutable_list(id) {
                    objects.arena.tuple(copies)
                } else {
                    objects.arena.list(copies)
                },
                true,
            ));
        }
        if objects.arena.kind(id) == ObjectKind::Dict {
            let fields = objects
                .arena
                .fields(id)
                .ok_or_else(|| unsupported("nested step isolation dictionary missing"))?
                .to_vec();
            let mut changed = false;
            let mut copies = Vec::new();
            for (field, value) in fields {
                let (copy, different) = Self::isolate_nested_step_value(objects, value, depth + 1)?;
                changed |= different;
                copies.push((field, copy));
            }
            if changed {
                return Ok((objects.arena.dictionary(copies).map_err(unsupported)?, true));
            }
        }
        Ok((ObjectValue::Object(id), false))
    }
    pub(super) fn validate_returned_step_bound(
        objects: &mut BoundObjects,
        step: ObjectId,
    ) -> Result<()> {
        let fields = objects
            .arena
            .fields(step)
            .ok_or_else(|| unsupported("returned step root missing"))?
            .to_vec();
        for (field, value) in fields {
            let (copy, changed) = Self::isolate_nested_step_value(objects, value, 0)?;
            if changed {
                objects
                    .arena
                    .set_field(step, field, copy)
                    .map_err(unsupported)?;
            }
        }
        Ok(())
    }
}
