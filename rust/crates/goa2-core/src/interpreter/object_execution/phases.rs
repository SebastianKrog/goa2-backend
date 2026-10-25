//! Authoritative arena turn boundaries and staged round-end orchestration.
use super::*;

#[derive(Clone, Copy)]
enum Field {
    Phase,
    Turn,
    DiscardLog,
    Locations,
    Snapshot,
    Pending,
    Second,
    Done,
    Unresolved,
    Played,
    Current,
    ResolvedCount,
    CardState,
    ExpiryIds,
    Planning,
    Resolution,
    Cleanup,
    Resolved,
    Hand,
}
const NAMES: [&str; 19] = [
    "phase",
    "turn",
    "turn_discard_log",
    "entity_locations",
    "last_turn_positions",
    "pending_inputs",
    "pending_second_cards",
    "planning_done",
    "unresolved_hero_ids",
    "played_cards",
    "current_turn_card",
    "resolved_turn_count",
    "state",
    "expire_effect_ids",
    "PLANNING",
    "RESOLUTION",
    "CLEANUP",
    "RESOLVED",
    "hand",
];
#[derive(Clone, Debug)]
pub(super) struct PhaseBindings {
    keys: [Symbol; 19],
    heroes: Vec<(Symbol, ObjectId)>,
}
impl PhaseBindings {
    pub fn compile(source: &StateEnvelope, arena: &mut ObjectArena) -> Result<Self> {
        let keys = NAMES.map(|name| Arc::make_mut(&mut arena.symbols).intern(name));
        let mut heroes = Vec::new();
        for (team, value) in source.state["teams"]
            .as_object()
            .ok_or_else(|| unsupported("teams missing"))?
        {
            for (index, hero) in value["heroes"]
                .as_array()
                .ok_or_else(|| unsupported("heroes missing"))?
                .iter()
                .enumerate()
            {
                let id = arena
                    .symbols
                    .lookup(
                        hero["id"]
                            .as_str()
                            .ok_or_else(|| unsupported("hero ID missing"))?,
                    )
                    .ok_or_else(|| unsupported("hero ID symbol missing"))?;
                let ObjectValue::Object(object) = arena
                    .value_at_path(&[
                        ObjectPathPart::Key("teams".into()),
                        ObjectPathPart::Key(team.clone()),
                        ObjectPathPart::Key("heroes".into()),
                        ObjectPathPart::Index(index),
                    ])
                    .map_err(unsupported)?
                else {
                    return Err(unsupported("hero model binding missing"));
                };
                heroes.push((id, object));
            }
        }
        Ok(Self { keys, heroes })
    }
    fn k(&self, field: Field) -> Symbol {
        self.keys[field as usize]
    }
    fn root(&self, arena: &ObjectArena, field: Field) -> Result<ObjectId> {
        match arena.root_field(self.k(field)) {
            Some(ObjectValue::Object(id)) => Ok(id),
            _ => Err(unsupported("phase collection binding missing")),
        }
    }
    pub fn effect_ids(&self, arena: &ObjectArena, step: ObjectId) -> Result<Vec<Symbol>> {
        let Some(ObjectValue::Object(list)) = arena.field(step, self.k(Field::ExpiryIds)) else {
            return Err(unsupported("deferred expiry IDs missing"));
        };
        arena
            .sequence(list)
            .ok_or_else(|| unsupported("deferred expiry IDs must be a list"))?
            .iter()
            .map(|value| match value {
                ObjectValue::Atom(Datum::Text(id)) => Ok(*id),
                _ => Err(unsupported("deferred expiry ID must be text")),
            })
            .collect()
    }
}
impl NativeSession {
    fn phase_step(&mut self, objects: &mut BoundObjects, name: &str) -> Result<ObjectId> {
        let program = objects.scheduler.as_ref().unwrap().clone();
        let recipe = &program
            .phase_recipes
            .iter()
            .find(|(key, _)| *key == name)
            .ok_or_else(|| unsupported("phase recipe missing"))?
            .1;
        self.next_step = self
            .next_step
            .checked_add(1)
            .ok_or_else(|| unsupported("step generation counter exhausted"))?;
        objects.generated_step(recipe, &program, self.next_step, None)
    }
    pub(super) fn phase_generation_check(&self, groups: usize, tail: u64) -> Result<()> {
        u64::try_from(groups)
            .ok()
            .and_then(|count| count.checked_mul(2))
            .and_then(|count| count.checked_add(tail))
            .and_then(|count| self.next_step.checked_add(count))
            .filter(|next| *next < u64::MAX)
            .ok_or_else(|| unsupported("step generation counter exhausted"))?;
        Ok(())
    }
    fn finishing_payloads(
        &self,
        objects: &BoundObjects,
        finishing: &[(Symbol, ObjectId)],
    ) -> Result<Vec<(Symbol, Vec<ObjectId>)>> {
        finishing
            .iter()
            .map(|(source, list)| Ok((*source, objects.steps_in_list(*list)?)))
            .collect()
    }
    pub(super) fn wrap_finishings(
        &mut self,
        objects: &mut BoundObjects,
        groups: Vec<(Symbol, Vec<ObjectId>)>,
    ) -> Result<Vec<ObjectId>> {
        self.phase_generation_check(groups.len(), 0)?;
        let program = objects.scheduler.as_ref().unwrap().clone();
        let mut queued = Vec::new();
        for (actor, steps) in groups {
            self.next_step += 1;
            queued.push(objects.generated_step(
                &program.actor,
                &program,
                self.next_step,
                Some(actor),
            )?);
            queued.extend(steps);
            self.next_step += 1;
            queued.push(objects.generated_step(&program.marker, &program, self.next_step, None)?);
        }
        Ok(queued)
    }
    fn phase_set(&mut self, objects: &mut BoundObjects, phase: Phase) -> Result<()> {
        let field = match phase {
            Phase::Planning => Field::Planning,
            Phase::Resolution => Field::Resolution,
            Phase::Cleanup => Field::Cleanup,
            _ => return Err(unsupported("unbound phase header")),
        };
        objects
            .arena
            .set_root(
                objects.phases.k(Field::Phase),
                ObjectValue::Atom(Datum::Text(objects.phases.k(field))),
            )
            .map_err(unsupported)?;
        self.planning.phase = phase;
        Ok(())
    }
    fn clear_discard_log_bound(&mut self, objects: &mut BoundObjects) -> Result<()> {
        let empty = objects.arena.dictionary(Vec::new()).map_err(unsupported)?;
        objects
            .arena
            .set_root(objects.phases.k(Field::DiscardLog), empty)
            .map_err(unsupported)?;
        self.world.history.discarded.clear();
        objects.readonly = Arc::new(objects.arena.readonly_objects());
        Ok(())
    }
    pub(super) fn preflight_bound_advance(&self, objects: &BoundObjects) -> Result<()> {
        let p = &objects.phases;
        let pending = p.root(&objects.arena, Field::Pending)?;
        let mut pending = objects
            .arena
            .fields(pending)
            .ok_or_else(|| unsupported("pending inputs must be a dictionary"))?
            .to_vec();
        let locations = p.root(&objects.arena, Field::Locations)?;
        if objects.arena.fields(locations).is_none() {
            return Err(unsupported("positions must be a dictionary"));
        }
        let mut auto_passed = false;
        for (hero, model) in &p.heroes {
            let Some(ObjectValue::Object(hand)) = objects.arena.field(*model, p.k(Field::Hand))
            else {
                return Err(unsupported("hero hand missing"));
            };
            if objects
                .arena
                .sequence(hand)
                .is_some_and(|hand| hand.is_empty())
            {
                auto_passed = true;
                if let Some((_, value)) = pending.iter_mut().find(|(id, _)| id == hero) {
                    *value = ObjectValue::Atom(Datum::Null);
                } else {
                    pending.push((*hero, ObjectValue::Atom(Datum::Null)));
                }
            }
        }
        if self.world.combat.turn < 4
            && auto_passed
            && pending.len() >= p.heroes.len()
            && pending
                .iter()
                .any(|(_, card)| !matches!(card, ObjectValue::Atom(Datum::Null)))
        {
            return Err(unsupported(
                "automatic revelation of committed cards not yet arena-bound",
            ));
        }
        self.phase_generation_check(0, 1)
    }
    pub(super) fn check_bound_phase(
        &self,
        objects: &BoundObjects,
        instruction: &Instruction,
    ) -> Result<()> {
        use crate::turn::FlowStep;
        match instruction {
            Instruction::Battle(_) | Instruction::Flow(FlowStep::Cleanup) => Err(unsupported(
                "arena round battle/cleanup consumers not yet bound",
            )),
            Instruction::Flow(FlowStep::Next) => {
                let unresolved = objects.phases.root(&objects.arena, Field::Unresolved)?;
                if objects
                    .arena
                    .sequence(unresolved)
                    .is_none_or(|list| !list.is_empty())
                {
                    return Err(unsupported(
                        "arena initiative/card-resolution continuation not yet bound",
                    ));
                }
                self.preflight_end_turn_bound(objects)
            }
            Instruction::Flow(FlowStep::Advance) => self.preflight_bound_advance(objects),
            Instruction::Flow(FlowStep::EndPhase) => {
                let indices = self
                    .world
                    .effects
                    .duration_indices(crate::effects::Duration::ThisRound);
                let finishing = objects.finishing_for_indices(&self.world.effects, &indices)?;
                self.finishing_payloads(objects, &finishing)?;
                self.phase_generation_check(finishing.len(), 3)
            }
            _ => Ok(()),
        }
    }
    fn preflight_end_turn_bound(&self, objects: &BoundObjects) -> Result<()> {
        let expiry = self.collect_turn_expiry_bound(objects)?;
        self.finishing_payloads(objects, &expiry.finishing)?;
        self.phase_generation_check(expiry.finishing.len(), 1)?;
        self.preflight_bound_advance(objects)
    }
    /// The source end_turn helper, including rule-v9 deferred effect removal.
    pub fn end_turn(&mut self) -> Result<()> {
        self.with_live_objects(|session, objects| session.end_turn_bound(objects))
    }
    pub(super) fn end_turn_bound(&mut self, objects: &mut BoundObjects) -> Result<()> {
        self.preflight_end_turn_bound(objects)?;
        let expiry = self.collect_turn_expiry_bound(objects)?;
        let groups = self.finishing_payloads(objects, &expiry.finishing)?;
        self.clear_discard_log_bound(objects)?;
        if groups.is_empty() {
            self.remove_effects_bound(objects, &expiry.ids)?;
        }
        if !groups.is_empty() {
            let mut queued = self.wrap_finishings(objects, groups)?;
            let advance = self.phase_step(objects, "advance_turn")?;
            let ids = objects.arena.list(
                expiry
                    .ids
                    .into_iter()
                    .map(|id| ObjectValue::Atom(Datum::Text(id)))
                    .collect(),
            );
            objects
                .arena
                .set_field(advance, objects.phases.k(Field::ExpiryIds), ids)
                .map_err(unsupported)?;
            queued.push(advance);
            return self.push_live_steps_bound(objects, &queued);
        }
        self.advance_turn_bound(objects)
    }
    pub(super) fn advance_turn_bound(&mut self, objects: &mut BoundObjects) -> Result<()> {
        self.preflight_bound_advance(objects)?;
        if self.world.combat.turn >= 4 {
            self.phase_set(objects, Phase::Cleanup)?;
            let step = self.phase_step(objects, "end_phase")?;
            return self.push_live_steps_bound(objects, &[step]);
        }
        self.world.combat.turn += 1;
        objects
            .arena
            .set_root(
                objects.phases.k(Field::Turn),
                ObjectValue::Atom(Datum::Int(self.world.combat.turn)),
            )
            .map_err(unsupported)?;
        let locations = objects.phases.root(&objects.arena, Field::Locations)?;
        let fields = objects
            .arena
            .fields(locations)
            .ok_or_else(|| unsupported("position fields missing"))?
            .to_vec();
        let snapshot = objects.arena.dictionary(fields).map_err(unsupported)?;
        objects
            .arena
            .set_root(objects.phases.k(Field::Snapshot), snapshot)
            .map_err(unsupported)?;
        self.boundary.snapshot(&self.world);
        self.phase_set(objects, Phase::Planning)?;
        let pending = objects.phases.root(&objects.arena, Field::Pending)?;
        let mut passed = false;
        for (hero, model) in objects.phases.heroes.clone() {
            let Some(ObjectValue::Object(hand)) =
                objects.arena.field(model, objects.phases.k(Field::Hand))
            else {
                return Err(unsupported("hand binding missing"));
            };
            if objects
                .arena
                .sequence(hand)
                .is_some_and(|hand| hand.is_empty())
            {
                objects
                    .arena
                    .set_field(pending, hero, ObjectValue::Atom(Datum::Null))
                    .map_err(unsupported)?;
                passed = true;
            }
        }
        if passed
            && objects
                .arena
                .fields(pending)
                .is_some_and(|entries| entries.len() >= objects.phases.heroes.len())
        {
            return self.reveal_all_passes_bound(objects, pending);
        }
        objects.readonly = Arc::new(objects.arena.readonly_objects());
        Ok(())
    }
    fn reveal_all_passes_bound(
        &mut self,
        objects: &mut BoundObjects,
        pending: ObjectId,
    ) -> Result<()> {
        // Every first commit is None in this bound path; committed-card revelation
        // remains explicitly guarded until its physical/header bindings land.
        let entries = objects
            .arena
            .fields(pending)
            .ok_or_else(|| unsupported("pending input fields missing"))?
            .to_vec();
        for (hero, value) in entries {
            if !matches!(value, ObjectValue::Atom(Datum::Null)) {
                return Err(unsupported("committed-card revelation not yet bound"));
            }
            if let Some((_, model)) = objects
                .phases
                .heroes
                .iter()
                .find(|(id, _)| *id == hero)
                .copied()
            {
                let current = objects
                    .arena
                    .field(model, objects.phases.k(Field::Current))
                    .ok_or_else(|| unsupported("current card missing"))?;
                self.world
                    .resolve_current_card(hero, Some(self.world.combat.turn))
                    .map_err(card_error)?;
                let Some(ObjectValue::Object(played)) =
                    objects.arena.field(model, objects.phases.k(Field::Played))
                else {
                    return Err(unsupported("played list missing"));
                };
                while objects
                    .arena
                    .sequence(played)
                    .is_some_and(|cards| cards.len() < self.world.combat.turn as usize)
                {
                    objects
                        .arena
                        .append(played, ObjectValue::Atom(Datum::Null))
                        .map_err(unsupported)?;
                }
                if let ObjectValue::Atom(Datum::Instance(card)) = current {
                    objects
                        .arena
                        .set_physical_field(
                            card,
                            objects.phases.k(Field::CardState),
                            Datum::Text(objects.phases.k(Field::Resolved)),
                        )
                        .map_err(unsupported)?;
                    objects
                        .arena
                        .set_index(
                            played,
                            self.world.combat.turn as usize - 1,
                            ObjectValue::Atom(Datum::Instance(card)),
                        )
                        .map_err(unsupported)?;
                    objects
                        .arena
                        .set_field(
                            model,
                            objects.phases.k(Field::Current),
                            ObjectValue::Atom(Datum::Null),
                        )
                        .map_err(unsupported)?;
                }
                let count = self
                    .world
                    .hero(hero)
                    .ok_or_else(|| unsupported("hero lookup missing"))?
                    .resolved_turns;
                objects
                    .arena
                    .set_field(
                        model,
                        objects.phases.k(Field::ResolvedCount),
                        ObjectValue::Atom(Datum::Int(count)),
                    )
                    .map_err(unsupported)?;
            }
        }
        for field in [Field::Pending, Field::Second] {
            let empty = objects.arena.dictionary(Vec::new()).map_err(unsupported)?;
            objects
                .arena
                .set_root(objects.phases.k(field), empty)
                .map_err(unsupported)?;
        }
        for field in [Field::Done, Field::Unresolved] {
            let empty = objects.arena.list(Vec::new());
            objects
                .arena
                .set_root(objects.phases.k(field), empty)
                .map_err(unsupported)?;
        }
        self.world.combat.unresolved.clear();
        self.phase_set(objects, Phase::Resolution)?;
        let mut steps = self.collect_after_cards_played_bound(objects)?;
        if !steps.is_empty() {
            steps.push(self.phase_step(objects, "find_actor")?);
            self.push_live_steps_bound(objects, &steps)
        } else {
            self.end_turn_bound(objects)
        }
    }
    pub(super) fn end_phase_bound(&mut self, objects: &mut BoundObjects) -> Result<()> {
        self.check_bound_phase(objects, &Instruction::Flow(crate::turn::FlowStep::EndPhase))?;
        let finishing = self.expire_effects_bound(objects, crate::effects::Duration::ThisRound)?;
        let groups = self.finishing_payloads(objects, &finishing)?;
        let mut steps = self.wrap_finishings(objects, groups)?;
        for name in ["minion_battle", "check_lane_push", "end_cleanup"] {
            steps.push(self.phase_step(objects, name)?);
        }
        self.push_live_steps_bound(objects, &steps)
    }
}
