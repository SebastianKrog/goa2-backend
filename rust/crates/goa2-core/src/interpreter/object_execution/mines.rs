//! Mine choices and ordered triggers use live operands and retained aliases.
use super::phases::Field;
use super::*;
use crate::mines::MinePath;

pub(super) struct MineChoicePlan {
    output: Symbol,
    options: Vec<MinePath>,
    selected: Option<usize>,
}
pub(super) struct MineTriggerPlan {
    ids: Vec<Symbol>,
    victim: Option<Symbol>,
    ids_key: Symbol,
    victim_key: Symbol,
}
impl NativeSession {
    pub(super) fn mine_symbols_bound(
        &self,
        objects: &BoundObjects,
        value: ObjectValue,
    ) -> Result<Vec<Symbol>> {
        fn symbol(value: &Datum) -> Result<Symbol> {
            match value {
                Datum::Text(id) => Ok(*id),
                _ => Err(unsupported("mine identity must be text")),
            }
        }
        match value {
            ObjectValue::Object(id) => objects
                .arena
                .sequence(id)
                .ok_or_else(|| unsupported("mine queue must be a sequence"))?
                .iter()
                .map(|value| match value {
                    ObjectValue::Atom(value) => symbol(value),
                    _ => Err(unsupported("mine identity must be text")),
                })
                .collect(),
            ObjectValue::Atom(Datum::List(values)) => values.iter().map(symbol).collect(),
            _ => Err(unsupported("mine queue must be a sequence")),
        }
    }

    fn mine_key_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        field: Field,
    ) -> Result<Symbol> {
        match objects.arena.field(step, objects.phases.k(field)) {
            Some(ObjectValue::Atom(Datum::Text(key))) => Ok(key),
            _ => Err(unsupported("mine context key must be text")),
        }
    }
    pub(super) fn write_mine_list_bound(
        &self,
        objects: &mut BoundObjects,
        key: Symbol,
        ids: &[Symbol],
    ) -> Result<ObjectValue> {
        let value = objects.arena.list(
            ids.iter()
                .copied()
                .map(|id| ObjectValue::Atom(Datum::Text(id)))
                .collect(),
        );
        objects
            .arena
            .set_context(key, value.clone())
            .map_err(unsupported)?;
        Ok(value)
    }
    pub(super) fn preflight_mine_choice_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<MineChoicePlan> {
        let output = self.mine_key_bound(objects, step, Field::Output)?;
        let empty = || MineChoicePlan {
            output,
            options: Vec::new(),
            selected: Some(0),
        };
        let key = self.mine_key_bound(objects, step, Field::DestinationKey)?;
        let Some(dest) = objects
            .arena
            .context_value(key)
            .filter(|v| objects.arena.truthy(v))
        else {
            return Ok(empty());
        };
        let (destination, _) = self.spatial_hex_bound(objects, &dest)?;
        let mut unit = self.truthy_symbol_bound(
            objects,
            objects.arena.field(step, objects.phases.k(Field::UnitId)),
        )?;
        if unit.is_none()
            && let Some(key) = self.truthy_symbol_bound(
                objects,
                objects.arena.field(step, objects.phases.k(Field::UnitKey)),
            )?
        {
            unit = self.truthy_symbol_bound(objects, objects.arena.context_value(key))?;
        }
        let actor = self
            .planning
            .actor
            .filter(|id| !self.symbols.name(*id).is_empty());
        let Some(unit) = unit.or(actor) else {
            return Ok(empty());
        };
        let team = self
            .world
            .entity(unit)
            .filter(|e| matches!(e.kind, EntityKind::Hero | EntityKind::Piece))
            .and_then(|e| e.team);
        let Some((start, team)) = self.world.position(unit).zip(team) else {
            return Ok(empty());
        };
        let Some(ObjectValue::Atom(Datum::Int(range))) =
            objects.arena.field(step, objects.phases.k(Field::Range))
        else {
            return Err(unsupported("mine range missing"));
        };
        let range = i32::try_from(range).map_err(|_| unsupported("mine range overflow"))?;
        let mut paths = self
            .world
            .mine_paths(start, range, actor, Some(team), &[unit])
            .map_err(unsupported)?;
        let options = self
            .world
            .tile_index(destination)
            .map(|index| std::mem::take(&mut paths[index]))
            .unwrap_or_default();
        let mut selected = (options.len() <= 1).then_some(0);
        if selected.is_none()
            && objects
                .arena
                .field(step, objects.pending)
                .is_some_and(|v| objects.arena.truthy(&v))
        {
            let Some(ObjectValue::Object(pending)) = objects.arena.field(step, objects.pending)
            else {
                return Err(unsupported("mine pending input must be a mapping"));
            };
            if objects.arena.kind(pending) != ObjectKind::Dict {
                return Err(unsupported("mine pending input must be a mapping"));
            }
            if let Some(ObjectValue::Atom(Datum::Text(id))) =
                objects.selected(step, self.selection_key)
            {
                selected = (0..options.len()).find(|i| i.to_string() == self.symbols.name(id));
            }
        }
        Ok(MineChoicePlan {
            output,
            options,
            selected,
        })
    }
    pub(super) fn mine_choice_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: MineChoicePlan,
    ) -> Result<Option<NativeRequest>> {
        if let Some(index) = plan.selected {
            let mines = plan
                .options
                .get(index)
                .map(|p| p.mines.as_ref())
                .unwrap_or(&[]);
            self.write_mine_list_bound(objects, plan.output, mines)?;
            // Successful MinePathChoice does not clear its retained pending input.
            return Ok(None);
        }
        if objects
            .arena
            .field(step, objects.pending)
            .is_some_and(|v| objects.arena.truthy(&v))
        {
            objects
                .arena
                .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
        }
        Ok(Some(NativeRequest {
            controlled: None,
            id: RequestToken::Generated(0),
            owner: self
                .planning
                .actor
                .filter(|id| !self.symbols.name(*id).is_empty())
                .unwrap_or(self.world.definitions.effect_empty),
            prompt: objects.phases.k(Field::MinePrompt),
            movement_range: None,
            pre_action_range: None,
            choices: RequestChoices::Mines(plan.options),
            can_skip: false,
            discard: false,
        }))
    }
    pub(super) fn preflight_mine_trigger_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<MineTriggerPlan> {
        let ids_key = self.mine_key_bound(objects, step, Field::MinesKey)?;
        let victim_key = self.mine_key_bound(objects, step, Field::VictimKey)?;
        let ids = self.mine_trigger_ids_bound(objects, step, ids_key)?;
        let mut victim = None;
        if !ids.is_empty() {
            victim = self.truthy_symbol_bound(
                objects,
                objects.arena.field(step, objects.phases.k(Field::Victim)),
            )?;
            if victim.is_none() && !self.symbols.name(victim_key).is_empty() {
                victim =
                    self.truthy_symbol_bound(objects, objects.arena.context_value(victim_key))?;
            }
            if objects.effect_nodes()?.len() != self.world.effects.instances.len() {
                return Err(unsupported("mine effect binding count drift"));
            }
            let mut blasts = 0;
            for id in &ids {
                if self.world.position(*id).is_some() {
                    self.preflight_remove_spatial_bound(objects, *id)?;
                }
                if self
                    .world
                    .entity(*id)
                    .is_some_and(|e| e.token_type == Some(objects.phases.k(Field::Blast)))
                {
                    blasts += 1;
                }
            }
            self.phase_generation_check(0, blasts)?;
        }
        Ok(MineTriggerPlan {
            ids,
            victim,
            ids_key,
            victim_key,
        })
    }
    fn mine_trigger_ids_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        ids_key: Symbol,
    ) -> Result<Vec<Symbol>> {
        match objects
            .arena
            .field(step, objects.phases.k(Field::MinesLiteral))
        {
            None | Some(ObjectValue::Atom(Datum::Null)) => match objects
                .arena
                .context_value(ids_key)
                .filter(|v| objects.arena.truthy(v))
            {
                None => Ok(Vec::new()),
                Some(value) => self.mine_symbols_bound(objects, value),
            },
            Some(value) => self.mine_symbols_bound(objects, value),
        }
    }

    pub(super) fn trigger_reads_concealed_mine_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
        viewer: Symbol,
    ) -> Result<bool> {
        let key = self.mine_key_bound(objects, step, Field::MinesKey)?;
        for id in self.mine_trigger_ids_bound(objects, step, key)? {
            if self.world.position(id).is_some()
                && self.world.entity(id).is_some_and(|entity| entity.owner != Some(viewer))
                && !self.token_is_public_bound(objects, id)?
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn preflight_remove_spatial_bound(
        &self,
        objects: &BoundObjects,
        id: Symbol,
    ) -> Result<()> {
        let locations = objects.phases.root(&objects.arena, Field::Locations)?;
        if objects.arena.kind(locations) != ObjectKind::Dict {
            return Err(unsupported("mine locations must be mutable"));
        }
        if let Some(from) = self.world.position(id) {
            self.tile_bound(objects, from)?;
            let value = objects
                .arena
                .field(locations, id)
                .ok_or_else(|| unsupported("mine location missing"))?;
            let (point, shared) = self.spatial_hex_bound(objects, &value)?;
            if point != from || shared.is_none() {
                return Err(unsupported("mine location cache drift"));
            }
        }
        Ok(())
    }
    pub(super) fn remove_entity_spatial_bound(
        &mut self,
        objects: &mut BoundObjects,
        id: Symbol,
    ) -> Result<Option<Hex>> {
        let Some(from) = self.world.position(id) else {
            return Ok(None);
        };
        let tile = self.tile_bound(objects, from)?;
        let locations = objects.phases.root(&objects.arena, Field::Locations)?;
        self.world.remove_entity(id).map_err(EngineError::Rule)?;
        objects
            .arena
            .apply(&crate::objects::Mutation::Delete {
                target: crate::objects::Target::Object(locations),
                key: id,
            })
            .map_err(unsupported)?;
        if let Some(tile) = tile
            && matches!(objects.arena.field(tile, objects.phases.k(Field::Occupant)), Some(ObjectValue::Atom(Datum::Text(unit))) if unit == id)
        {
            objects
                .arena
                .set_field(
                    tile,
                    objects.phases.k(Field::Occupant),
                    ObjectValue::Atom(Datum::Null),
                )
                .map_err(unsupported)?;
        }
        if matches!(objects.arena.root_field(objects.phases.k(Field::Snapshot)), Some(ObjectValue::Object(snapshot)) if snapshot == locations)
        {
            self.boundary.snapshot(&self.world);
        }
        Ok(Some(from))
    }
    pub(super) fn remove_mine_spatial_bound(
        &mut self,
        objects: &mut BoundObjects,
        id: Symbol,
    ) -> Result<Option<Hex>> {
        let Some(from) = self.remove_entity_spatial_bound(objects, id)? else {
            return Ok(None);
        };
        let nodes = objects.effect_nodes()?;
        let retained = self
            .world
            .effects
            .instances
            .iter()
            .zip(nodes)
            .filter(|(e, _)| e.token != Some(id) && e.source != id && e.scope_origin != Some(id))
            .map(|(_, node)| ObjectValue::Object(node))
            .collect();
        self.world.remove_token_effects(id);
        // Source replaces active_effects even when no matching effect exists.
        let list = objects.arena.list(retained);
        objects
            .arena
            .set_root(objects.effects, list)
            .map_err(unsupported)?;
        Ok(Some(from))
    }
    pub(super) fn mine_trigger_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: MineTriggerPlan,
    ) -> Result<Vec<NativeEvent>> {
        if plan.ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        let mut sources = Vec::new();
        for id in plan.ids {
            let entity = self.world.entity(id);
            let exists = entity.is_some();
            let kind = entity.and_then(|e| e.token_type);
            let blast = kind == Some(objects.phases.k(Field::Blast));
            let owner = entity
                .and_then(|e| e.owner)
                .filter(|id| !self.symbols.name(*id).is_empty());
            let from = self.remove_mine_spatial_bound(objects, id)?;
            if blast {
                sources.push(owner);
            }
            if let Some(from) = from.filter(|_| exists) {
                events.push(NativeEvent::MineTriggered {
                    actor: self
                        .planning
                        .actor
                        .filter(|id| !self.symbols.name(*id).is_empty()),
                    target: id,
                    from,
                    kind,
                    blast,
                });
            }
        }
        self.write_mine_list_bound(objects, plan.ids_key, &[])?;
        objects
            .arena
            .set_context(
                objects.phases.k(Field::RollbackReanchor),
                ObjectValue::Atom(Datum::Bool(true)),
            )
            .map_err(unsupported)?;
        let mut children = Vec::new();
        let immunity = self
            .symbols
            .lookup("immunity_source_id")
            .ok_or_else(|| unsupported("discard immunity binding missing"))?;
        for owner in sources {
            let child = self.phase_step(objects, "force_discard")?;
            for (key, value) in [
                (
                    objects.phases.k(Field::Victim),
                    plan.victim.map_or(Datum::Null, Datum::Text),
                ),
                (
                    objects.phases.k(Field::VictimKey),
                    Datum::Text(plan.victim_key),
                ),
                (immunity, owner.map_or(Datum::Null, Datum::Text)),
            ] {
                objects
                    .arena
                    .set_field(child, key, ObjectValue::Atom(value))
                    .map_err(unsupported)?;
            }
            children.push(child);
        }
        self.push_live_steps_bound(objects, &children)?;
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session(name: &str) -> NativeSession {
        let case = fixture_case(&format!("object-execution-{name}"));
        NativeSession::compile(serde_json::from_value(case["envelope"].clone()).unwrap()).unwrap()
    }
    #[test]
    fn sampled_search_stops_before_an_opponents_concealed_mine() {
        let mut root = session("mines-trigger-ordered");
        let viewer = root.symbols.lookup("hero_arien").unwrap();
        let before = root.semantic_state().unwrap();
        let result = root.advance_search_objects(&ExecutionBudget::default(), viewer).unwrap();
        assert_eq!(result.steps, 0);
        assert_eq!(root.semantic_state().unwrap(), before);

        // The underlying game rules still trigger the mine normally.
        assert_eq!(root.advance_prefix(1).unwrap().steps, 1);
        assert_ne!(root.semantic_state().unwrap(), before);
    }

    #[test]
    fn mine_choice_workers_keep_pending_and_selected_lists_private() {
        let mut root = session("mines-choice-one-dud-first");
        root.advance_prefix(1).unwrap();
        let before = serde_json::to_value(root.export_envelope().unwrap()).unwrap();
        let token = root.last_request.as_ref().unwrap().id;
        let mut a = root.clone();
        let mut b = root.clone();
        for (worker, value) in [(&mut a, "0"), (&mut b, "1")] {
            let selection = worker.import_selection(&json!(value)).unwrap();
            worker.submit(token, selection).unwrap();
            worker.advance_prefix(1).unwrap();
        }
        assert_ne!(
            a.semantic_state().unwrap()["execution_context"]["triggered_mine_ids"],
            b.semantic_state().unwrap()["execution_context"]["triggered_mine_ids"]
        );
        assert_eq!(
            serde_json::to_value(root.export_envelope().unwrap()).unwrap(),
            before
        );
    }
    #[test]
    fn movement_mine_requeue_is_a_shallow_step_copy() {
        let mut session = session("mines-movement-dud-choice-first");
        let objects = session.objects.as_mut().unwrap();
        let original = session.stack.last().unwrap().object.unwrap();
        let pending = objects.arena.allocate(&Datum::Object(Vec::new()));
        objects
            .arena
            .set_field(original, objects.pending, pending)
            .unwrap();
        let unit = session.symbols.lookup("hero_arien").unwrap();
        let from = session.world.position(unit);
        let result = session.advance_prefix(1).unwrap();
        assert!(result.events.is_empty());
        assert_eq!(session.world.position(unit), from);
        let objects = session.objects.as_ref().unwrap();
        let copy = session.stack[session.stack.len() - 2].object.unwrap();
        assert_ne!(original, copy);
        match (
            objects.arena.field(original, objects.pending),
            objects.arena.field(copy, objects.pending),
        ) {
            (Some(ObjectValue::Object(a)), Some(ObjectValue::Object(b))) => assert_eq!(a, b),
            _ => panic!("shallow-copy pending mapping missing"),
        }
        assert!(matches!(
            objects
                .arena
                .field(copy, objects.phases.k(Field::Prechosen)),
            Some(ObjectValue::Atom(Datum::Bool(true)))
        ));
        assert!(matches!(
            objects
                .arena
                .field(original, objects.phases.k(Field::Prechosen)),
            Some(ObjectValue::Atom(Datum::Bool(false)))
        ));
    }
    #[test]
    fn blast_queue_keeps_literal_victim_and_restores_discard_passive() {
        let mut session = session("mines-movement-blast-corridor-first");
        session.advance_prefix(1).unwrap();
        let objects = session.objects.as_mut().unwrap();
        let key = objects.phases.k(Field::MineIds);
        let empty = objects.arena.list(Vec::new());
        objects.arena.set_context(key, empty).unwrap();
        objects
            .arena
            .set_context(
                objects.phases.k(Field::MineVictim),
                ObjectValue::Atom(Datum::Text(session.symbols.lookup("hero_min").unwrap())),
            )
            .unwrap();
        session.advance_prefix(1).unwrap();
        assert!(
            session
                .world
                .position(session.symbols.lookup("mine_a").unwrap())
                .is_none()
        );
        session.advance_prefix(1).unwrap();
        assert_eq!(
            session.semantic_state().unwrap()["execution_stack"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["context_hero_id"],
            "hero_arien"
        );
        let request = session.advance_prefix(1).unwrap().request.unwrap();
        let RequestChoices::CardIds { values, .. } = request.choices else {
            panic!("expected card choices");
        };
        session.submit(request.id, Datum::Text(values[0])).unwrap();
        session.advance_prefix(1).unwrap();
        session.advance_prefix(1).unwrap(); // Actual discard, followed by passive check.
        let before = session.export_envelope().unwrap();
        assert_eq!(
            before.state["execution_context"]["discarded_card_owner_id"],
            "hero_arien"
        );
        let mut restored = NativeSession::compile(before.clone()).unwrap();
        session.advance_prefix(1).unwrap();
        restored.advance_prefix(1).unwrap();
        assert_eq!(
            session.export_envelope().unwrap().state,
            restored.export_envelope().unwrap().state
        );
        assert_eq!(
            session.export_envelope().unwrap().object_bindings,
            restored.export_envelope().unwrap().object_bindings
        );
    }
}
