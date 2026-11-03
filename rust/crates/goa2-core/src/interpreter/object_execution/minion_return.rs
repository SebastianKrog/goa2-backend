//! Automatic return controller; relocation consumers keep their own live guards.
use super::phases::Field;
use super::*;
use crate::minion_return::ReturnPlan;

impl NativeSession {
    fn processed_minions_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Vec<Symbol>> {
        let Some(ObjectValue::Object(list)) = objects
            .arena
            .field(step, objects.phases.k(Field::ProcessedMinions))
        else {
            return Err(unsupported("processed minions list missing"));
        };
        objects
            .arena
            .sequence(list)
            .ok_or_else(|| unsupported("processed minions must be a list"))?
            .iter()
            .map(|value| match value {
                ObjectValue::Atom(Datum::Text(id)) => Ok(*id),
                _ => Err(unsupported("processed minion ID must be text")),
            })
            .collect()
    }
    fn return_destination_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        plan: &ReturnPlan,
    ) -> Result<Option<Hex>> {
        let pending = objects.arena.field(step, objects.pending);
        let mut destination = None;
        if let Some(pending) = pending.filter(|value| objects.arena.truthy(value)) {
            if !matches!(pending, ObjectValue::Object(id) if objects.arena.kind(id) == ObjectKind::Dict)
            {
                return Err(unsupported("minion-return input must be a mapping"));
            }
            if let Some(ObjectValue::Object(selection)) = objects.selected(step, self.selection_key)
                && matches!(
                    objects.arena.kind(selection),
                    ObjectKind::Dict | ObjectKind::FrozenModel
                )
            {
                let fields = self
                    .coordinates
                    .iter()
                    .filter_map(|key| match objects.arena.field(selection, *key) {
                        Some(ObjectValue::Atom(value)) => Some((*key, value)),
                        _ => None,
                    })
                    .collect();
                destination = Datum::Object(fields)
                    .hex(self.coordinates, &self.symbols)
                    .ok()
                    .filter(|hex| plan.candidates.contains(hex));
            }
        }
        Ok(destination)
    }
    pub(super) fn preflight_return_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<ReturnPlan>> {
        let processed = self.processed_minions_bound(objects, step)?;
        self.sync_card_collections_bound(objects)?;
        let favored = objects.phases.k(if self.team == Team::Red {
            Field::Red
        } else {
            Field::Blue
        });
        // Automatic returns are not movement actions. movement_legality reads
        // context only for action checks, which this planner never requests;
        // do not substitute the stale ordinary-session context for live data.
        let plan = self
            .return_rules
            .next(
                &self.world,
                &self.symbols,
                &processed,
                favored,
                self.planning.actor,
                &Registers::new(&self.symbols),
            )
            .map_err(unsupported)?;
        if let Some(plan) = &plan {
            let destination = if plan.candidates.is_empty() {
                None
            } else {
                self.return_destination_bound(objects, step, plan)?
                    .or_else(|| (plan.candidates.len() == 1).then(|| plan.candidates[0]))
            };
            if plan.candidates.is_empty() || destination.is_some() {
                self.phase_generation_check(
                    0,
                    u64::from(plan.more) + u64::from(destination.is_some()),
                )?;
            }
        }
        Ok(plan)
    }
    pub(super) fn return_minions_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: Option<ReturnPlan>,
    ) -> Result<Option<NativeRequest>> {
        let Some(plan) = plan else { return Ok(None) };
        let mut destination = None;
        if !plan.candidates.is_empty() {
            destination = self.return_destination_bound(objects, step, &plan)?;
            if destination.is_none()
                && objects
                    .arena
                    .field(step, objects.pending)
                    .is_some_and(|value| objects.arena.truthy(&value))
            {
                objects
                    .arena
                    .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                    .map_err(unsupported)?;
            }
            destination =
                destination.or_else(|| (plan.candidates.len() == 1).then(|| plan.candidates[0]));
        }
        if !plan.candidates.is_empty() && destination.is_none() {
            return Ok(Some(NativeRequest {
                controlled: None,
                id: RequestToken::Generated(0),
                owner: plan.owner,
                prompt: plan.prompt,
                movement_range: None,
                pre_action_range: None,
                choices: RequestChoices::Hexes(plan.candidates),
                can_skip: false,
                discard: false,
            }));
        }
        let mut queued = Vec::with_capacity(2);
        if let Some(hex) = destination {
            let child = self.phase_step(
                objects,
                if plan.placement {
                    "return_placement"
                } else {
                    "return_movement"
                },
            )?;
            objects
                .arena
                .set_field(
                    child,
                    objects.phases.k(Field::UnitId),
                    ObjectValue::Atom(Datum::Text(plan.unit)),
                )
                .map_err(unsupported)?;
            let Datum::Object(fields) = native_hex(hex, self.coordinates) else {
                unreachable!()
            };
            let target = objects
                .arena
                .shared_frozen_model(
                    fields
                        .into_iter()
                        .map(|(key, value)| (key, ObjectValue::Atom(value)))
                        .collect(),
                )
                .map_err(unsupported)?;
            objects
                .arena
                .set_field(child, objects.phases.k(Field::TargetHex), target)
                .map_err(unsupported)?;
            if !plan.placement {
                objects
                    .arena
                    .set_field(
                        child,
                        objects.phases.k(Field::Range),
                        ObjectValue::Atom(Datum::Int(i64::from(plan.distance))),
                    )
                    .map_err(unsupported)?;
            }
            queued.push(child);
        }
        if plan.more {
            let mut processed = self.processed_minions_bound(objects, step)?;
            processed.push(plan.unit);
            let continuation = self.phase_step(objects, "return_minions")?;
            let list = objects.arena.list(
                processed
                    .into_iter()
                    .map(|id| ObjectValue::Atom(Datum::Text(id)))
                    .collect(),
            );
            objects
                .arena
                .set_field(
                    continuation,
                    objects.phases.k(Field::ProcessedMinions),
                    list,
                )
                .map_err(unsupported)?;
            queued.push(continuation);
        }
        self.push_live_steps_bound(objects, &queued)?;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn live_return_cold_compilation_does_not_link_ordinary_move_or_mine_bodies() {
        let case = fixture_case("object-execution-minion-return-walk");
        let mut steps = Vec::new();
        let mut symbols = Symbols::default();
        let index = BoundObjects::compile_wire(
            &case["envelope"]["state"]["execution_stack"][0],
            &mut steps,
            &mut symbols,
        )
        .unwrap();
        assert_eq!(
            steps.len(),
            1,
            "return shell must not compile relocation children"
        );
        assert!(matches!(
            steps[index].instruction,
            Instruction::LiveReturnMinions
        ));
        let session =
            NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                .unwrap();
        assert!(
            session
                .steps
                .iter()
                .any(|step| matches!(step.instruction, Instruction::LiveReturnMinions))
        );
        assert!(!session.steps.iter().any(|step| matches!(
            step.instruction,
            Instruction::ReturnMinions { .. }
                | Instruction::Move { .. }
                | Instruction::MineChoice { .. }
                | Instruction::TriggerMines { .. }
        )));
    }
    #[test]
    fn return_generation_exhaustion_is_checked_before_pop_or_pending_mutation() {
        let case = fixture_case("object-execution-minion-return-walk");
        let mut session =
            NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                .unwrap();
        session.next_step = u64::MAX - 1;
        let before = serde_json::to_value(session.export_envelope().unwrap()).unwrap();
        assert!(
            session
                .advance_prefix(1)
                .unwrap_err()
                .to_string()
                .contains("generation")
        );
        assert_eq!(
            serde_json::to_value(session.export_envelope().unwrap()).unwrap(),
            before
        );
    }
    #[test]
    fn generated_destination_keeps_hex_identity_when_a_step_template_is_copied() {
        let case = fixture_case("object-execution-minion-return-walk");
        let mut session =
            NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap())
                .unwrap();
        session.advance_prefix(1).unwrap();
        let objects = session.objects.as_mut().unwrap();
        let ObjectValue::Object(child) = objects
            .arena
            .sequence(objects.stack)
            .unwrap()
            .last()
            .unwrap()
        else {
            panic!("missing child")
        };
        let target = objects
            .arena
            .field(*child, objects.phases.k(Field::TargetHex))
            .unwrap();
        let (copy, _) = objects.arena.deep_copy(&target).unwrap();
        assert!(matches!((target,copy),(ObjectValue::Object(a),ObjectValue::Object(b)) if a==b));
    }
}
