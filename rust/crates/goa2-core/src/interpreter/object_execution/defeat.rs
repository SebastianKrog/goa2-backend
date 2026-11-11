//! Defeat/protection reuse typed combat rules and commit through live arena handles.
use super::phases::Field;
use super::{
    Arc, BoundObjects, CombatEvent, Datum, GameOutcome, NativeEvent, NativeRequest, NativeSession,
    ObjectId, ObjectKind, ObjectValue, Phase, RequestChoices, RequestToken, Result, Symbol,
    combat_error, unsupported,
};
use crate::{
    cards::CardId,
    combat::DefeatResult,
    data::Registers,
    objects::{Mutation, Target},
    protection::{ProtectionBinding, ProtectionDecision},
    world::NativeWorld,
};

pub(super) enum DefeatPlan {
    Protection {
        minion: Symbol,
        killer: Option<Symbol>,
    },
    Commit(Box<DefeatCommit>),
}
pub(super) struct DefeatCommit {
    world: NativeWorld,
    result: DefeatResult,
    context: Registers,
    removed: Vec<usize>,
    hero: Option<(Symbol, ObjectId, Option<CardId>)>,
    normal: bool,
}
pub(super) enum ProtectionPlan {
    Missing,
    Next {
        spec: ProtectionBinding,
        effect: Symbol,
    },
    Sacrifice {
        minion: Symbol,
        effect: Symbol,
        hero: Symbol,
        token: Symbol,
    },
    Prompt {
        minion: Symbol,
        hero: Symbol,
        cards: Vec<CardId>,
    },
    Commit {
        commit: Box<DefeatCommit>,
        discard: Option<(Symbol, Symbol)>,
        protected: Option<(Symbol, Symbol, Symbol)>,
    },
}
impl NativeSession {
    fn combat_list_bound(&self, objects: &BoundObjects, field: Field) -> Result<ObjectId> {
        let list = objects.phases.root(&objects.arena, field)?;
        if !objects.arena.is_mutable_list(list) {
            return Err(unsupported("combat collection requires mutable list"));
        }
        Ok(list)
    }
    fn combat_root_ids_bound(&self, objects: &BoundObjects, field: Field) -> Result<Vec<Symbol>> {
        let list = self.combat_list_bound(objects, field)?;
        objects
            .arena
            .sequence(list)
            .unwrap()
            .iter()
            .map(|v| match v {
                ObjectValue::Atom(Datum::Text(id)) => Ok(*id),
                _ => Err(unsupported("combat identity list requires text")),
            })
            .collect()
    }
    fn combat_root_int_bound(&self, objects: &BoundObjects, field: Field) -> Result<i64> {
        match objects.arena.root_field(objects.phases.k(field)) {
            Some(ObjectValue::Atom(Datum::Int(value))) => Ok(value),
            _ => Err(unsupported("combat root integer missing")),
        }
    }
    fn combat_integer_bound(
        &self,
        objects: &BoundObjects,
        model: ObjectId,
        field: Field,
    ) -> Result<i64> {
        self.spatial_int_bound(objects, model, field)?
            .ok_or_else(|| unsupported("combat integer missing"))
    }
    /// Stage only the native mutable rows. Definitions remain shared; no arena,
    /// serialized state or execution context is copied to calculate consequences.
    fn combat_world_bound(&mut self, objects: &BoundObjects) -> Result<NativeWorld> {
        self.sync_card_collections_bound(objects)?;
        let mut world = self.world.staged_clone();
        for &(hero, model) in &objects.phases.heroes {
            let row = world
                .hero_index(hero)
                .ok_or_else(|| unsupported("combat hero cache missing"))?;
            world.heroes[row].gold = self.combat_integer_bound(objects, model, Field::Gold)?;
            world.heroes[row].level = self.combat_integer_bound(objects, model, Field::Level)?;
        }
        world.combat.turn = self.combat_root_int_bound(objects, Field::Turn)?;
        world.combat.unresolved = self.combat_root_ids_bound(objects, Field::Unresolved)?;
        world.combat.defeated = self.combat_root_ids_bound(objects, Field::DefeatedHeroes)?;
        let teams = objects.phases.root(&objects.arena, Field::Teams)?;
        for &(team, ref value) in objects
            .arena
            .fields(teams)
            .ok_or_else(|| unsupported("combat teams missing"))?
        {
            let ObjectValue::Object(model) = value else {
                return Err(unsupported("combat team model missing"));
            };
            world
                .combat
                .set_team_life(
                    team,
                    self.combat_integer_bound(objects, *model, Field::Life)?,
                )
                .map_err(unsupported)?;
        }
        let markers = objects.phases.root(&objects.arena, Field::Markers)?;
        for row in &mut world.markers {
            let Some(ObjectValue::Object(model)) = objects.arena.field(markers, row.key) else {
                return Err(unsupported("combat marker model missing"));
            };
            row.target = self.passive_optional_text_bound(objects, model, Field::TargetId)?;
            row.source = self.passive_optional_text_bound(objects, model, Field::SourceId)?;
            row.value = self.combat_integer_bound(objects, model, Field::Value)?;
        }
        let nodes = objects.effect_nodes()?;
        if nodes.len() != world.effects.instances.len() {
            return Err(unsupported("combat effect cache drift"));
        }
        for (row, node) in world.effects.instances.iter_mut().zip(nodes) {
            row.remaining_bounty = self.spatial_int_bound(objects, node, Field::MaxValue)?;
            row.arena_model = Some(node);
        }
        Ok(world)
    }
    fn preflight_combat_commit_bound(
        &self,
        objects: &BoundObjects,
        commit: &DefeatCommit,
    ) -> Result<()> {
        if let Some(value) = objects
            .arena
            .context_value(objects.phases.k(Field::DefeatLog))
            && !matches!(value, ObjectValue::Object(id) if objects.arena.is_mutable_list(id))
        {
            return Err(unsupported("defeat log requires mutable list"));
        }
        if let Some((_, model, _)) = commit.hero {
            if !(1..=1_000_000).contains(&commit.world.combat.turn) {
                return Err(unsupported("defeat turn slot outside supported bounds"));
            }
            let Some(ObjectValue::Object(list)) =
                objects.arena.field(model, objects.phases.k(Field::Played))
            else {
                return Err(unsupported("defeat played list missing"));
            };
            if !objects.arena.is_mutable_list(list) {
                return Err(unsupported("defeat played list must be mutable"));
            }
        }
        self.phase_generation_check(
            0,
            commit.result.removals.len() as u64 + u64::from(commit.result.outcome.is_some()),
        )
    }
    pub(super) fn preflight_defeat_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<DefeatPlan>> {
        let mut victim = self.truthy_symbol_bound(
            objects,
            objects.arena.field(step, objects.phases.k(Field::Victim)),
        )?;
        if victim.is_none()
            && let Some(key) = self.spatial_text_bound(objects, step, Field::VictimKey)?
        {
            victim = self.truthy_symbol_bound(objects, objects.arena.context_value(key))?;
        }
        let Some(victim) = victim else {
            return Ok(None);
        };
        let killer = self.passive_optional_text_bound(objects, step, Field::KillerId)?;
        let assist = self.combat_integer_bound(objects, step, Field::AssistMultiplier)?;
        let mut world = self.combat_world_bound(objects)?;
        let actual = world.hero(victim).map_or(victim, |hero| hero.id);
        let hero = self
            .hero_model_bound(objects, actual)
            .map(|model| {
                self.model_card_bound(objects, model, Field::Current)
                    .map(|current| (actual, model, current))
            })
            .transpose()?;
        let removed = world.expiring_source_rows(actual);
        let mut context = Registers::default();
        let result = self
            .combat_rules
            .defeat(&mut world, &mut context, victim, killer, assist)
            .map_err(combat_error)?;
        if result.protection {
            self.phase_generation_check(0, 1)?;
            return Ok(Some(DefeatPlan::Protection {
                minion: victim,
                killer,
            }));
        }
        if result.events.is_empty() && result.removals.is_empty() {
            return Ok(None);
        }
        let commit = DefeatCommit {
            world,
            result,
            context,
            removed,
            hero,
            normal: true,
        };
        self.preflight_combat_commit_bound(objects, &commit)?;
        Ok(Some(DefeatPlan::Commit(Box::new(commit))))
    }
    fn append_defeat_log_bound(
        &self,
        objects: &mut BoundObjects,
        context: &Registers,
    ) -> Result<()> {
        let key = objects.phases.k(Field::DefeatLog);
        let Some(Datum::List(rows)) = context.get(key) else {
            return Ok(());
        };
        let list = if let Some(ObjectValue::Object(list)) = objects.arena.context_value(key) {
            list
        } else {
            let value = objects.arena.list(Vec::new());
            let ObjectValue::Object(list) = value else {
                unreachable!()
            };
            objects.arena.set_context(key, value).map_err(unsupported)?;
            list
        };
        for row in rows {
            let row = objects.arena.allocate(row);
            objects.arena.append(list, row).map_err(unsupported)?;
        }
        Ok(())
    }
    fn commit_defeat_bound(
        &mut self,
        objects: &mut BoundObjects,
        commit: DefeatCommit,
    ) -> Result<Vec<NativeEvent>> {
        let DefeatCommit {
            world,
            result,
            context,
            removed,
            hero,
            normal,
        } = commit;
        let last = objects.phases.k(Field::LastDefeatedMinion);
        if !normal && let Some(value) = context.get(last) {
            objects
                .arena
                .set_context(last, ObjectValue::Atom(value.clone()))
                .map_err(unsupported)?;
        }
        self.append_defeat_log_bound(objects, &context)?;
        let effect_nodes = objects.effect_nodes()?;
        for (index, (node, before)) in effect_nodes
            .iter()
            .zip(&self.world.effects.instances)
            .enumerate()
        {
            if removed.contains(&index) {
                continue;
            }
            let after_index = index - removed.iter().filter(|old| **old < index).count();
            if let Some(after) = world.effects.instances.get(after_index)
                && after.remaining_bounty != before.remaining_bounty
            {
                objects
                    .arena
                    .set_field(
                        *node,
                        objects.phases.k(Field::MaxValue),
                        ObjectValue::Atom(after.remaining_bounty.map_or(Datum::Null, Datum::Int)),
                    )
                    .map_err(unsupported)?;
            }
        }
        let flags = self
            .world
            .cards()
            .iter()
            .map(|card| card.active)
            .collect::<Vec<_>>();
        let markers = objects.phases.root(&objects.arena, Field::Markers)?;
        for marker in &world.markers {
            let Some(ObjectValue::Object(model)) = objects.arena.field(markers, marker.key) else {
                return Err(unsupported("defeat marker model missing"));
            };
            for (field, value) in [
                (
                    Field::TargetId,
                    marker.target.map_or(Datum::Null, Datum::Text),
                ),
                (
                    Field::SourceId,
                    marker.source.map_or(Datum::Null, Datum::Text),
                ),
                (Field::Value, Datum::Int(marker.value)),
            ] {
                objects
                    .arena
                    .set_field(model, objects.phases.k(field), ObjectValue::Atom(value))
                    .map_err(unsupported)?;
            }
        }
        self.world = world;
        if normal {
            objects.remove_effect_rows(&removed)?;
        }
        self.sync_effect_card_flags(objects, &flags)?;
        if let Some((id, model, current)) = hero {
            self.sync_resolved_slot_bound(objects, id, model, current)?;
            let unresolved = self.combat_list_bound(objects, Field::Unresolved)?;
            if let Some(index) =
                objects.arena.sequence(unresolved).unwrap().iter().position(
                    |v| matches!(v, ObjectValue::Atom(Datum::Text(value)) if *value == id),
                )
            {
                objects
                    .arena
                    .apply(&Mutation::Pop {
                        target: Target::Object(unresolved),
                        index: index as i64,
                    })
                    .map_err(unsupported)?;
            }
            let defeated = self.combat_list_bound(objects, Field::DefeatedHeroes)?;
            if !objects
                .arena
                .sequence(defeated)
                .unwrap()
                .iter()
                .any(|v| matches!(v, ObjectValue::Atom(Datum::Text(value)) if *value == id))
            {
                objects
                    .arena
                    .append(defeated, ObjectValue::Atom(Datum::Text(id)))
                    .map_err(unsupported)?;
            }
            self.world.combat.unresolved =
                self.combat_root_ids_bound(objects, Field::Unresolved)?;
            self.world.combat.defeated =
                self.combat_root_ids_bound(objects, Field::DefeatedHeroes)?;
        }
        for &(id, model) in &objects.phases.heroes {
            let gold = self
                .world
                .hero(id)
                .ok_or_else(|| unsupported("defeat hero missing"))?
                .gold;
            objects
                .arena
                .set_field(
                    model,
                    objects.phases.k(Field::Gold),
                    ObjectValue::Atom(Datum::Int(gold)),
                )
                .map_err(unsupported)?;
        }
        let teams = objects.phases.root(&objects.arena, Field::Teams)?;
        for event in &result.events {
            if let CombatEvent::Life {
                team, remaining, ..
            } = event
            {
                let Some(ObjectValue::Object(model)) = objects.arena.field(teams, *team) else {
                    return Err(unsupported("defeat team missing"));
                };
                objects
                    .arena
                    .set_field(
                        model,
                        objects.phases.k(Field::Life),
                        ObjectValue::Atom(Datum::Int(*remaining)),
                    )
                    .map_err(unsupported)?;
            }
        }
        if normal && let Some(value) = context.get(last) {
            objects
                .arena
                .set_context(last, ObjectValue::Atom(value.clone()))
                .map_err(unsupported)?;
        }
        let mut children = Vec::new();
        for unit in result.removals {
            children.push(self.removal_step_bound(objects, unit)?);
        }
        if let Some(outcome) = result.outcome {
            let step = self.phase_step(objects, "game_over")?;
            self.set_outcome_step_bound(objects, step, outcome)?;
            children.push(step);
        }
        self.push_live_steps_bound(objects, &children)?;
        Ok(result.events.into_iter().map(NativeEvent::Combat).collect())
    }
    pub(super) fn defeat_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: Option<DefeatPlan>,
    ) -> Result<Vec<NativeEvent>> {
        match plan {
            None => Ok(Vec::new()),
            Some(DefeatPlan::Commit(commit)) => self.commit_defeat_bound(objects, *commit),
            Some(DefeatPlan::Protection { minion, killer }) => {
                let child = self.protection_step_bound(
                    objects,
                    &ProtectionBinding {
                        minion,
                        killer,
                        tried: Arc::from([]),
                    },
                    None,
                )?;
                self.push_live_steps_bound(objects, &[child])?;
                Ok(Vec::new())
            }
        }
    }
    fn removal_step_bound(&mut self, objects: &mut BoundObjects, unit: Symbol) -> Result<ObjectId> {
        let step = self.phase_step(objects, "remove_unit")?;
        objects
            .arena
            .set_field(
                step,
                objects.phases.k(Field::UnitId),
                ObjectValue::Atom(Datum::Text(unit)),
            )
            .map_err(unsupported)?;
        Ok(step)
    }
    pub(super) fn preflight_remove_unit_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<Symbol>> {
        let mut id = self.truthy_symbol_bound(
            objects,
            objects.arena.field(step, objects.phases.k(Field::UnitId)),
        )?;
        if let Some(key) = self.spatial_text_bound(objects, step, Field::UnitKey)? {
            id = self.truthy_symbol_bound(objects, objects.arena.context_value(key))?;
        }
        if let Some(id) = id {
            self.preflight_remove_spatial_bound(objects, id)?;
        }
        Ok(id)
    }
    pub(super) fn remove_unit_bound(
        &mut self,
        objects: &mut BoundObjects,
        id: Option<Symbol>,
    ) -> Result<Vec<NativeEvent>> {
        let Some(id) = id else {
            return Ok(Vec::new());
        };
        let from = self.remove_entity_spatial_bound(objects, id)?;
        Ok(vec![NativeEvent::Combat(CombatEvent::Removed {
            target: id,
            from,
        })])
    }
    fn set_outcome_step_bound(
        &self,
        objects: &mut BoundObjects,
        step: ObjectId,
        outcome: GameOutcome,
    ) -> Result<()> {
        for (field, value) in [
            (Field::Winner, outcome.team.map_or(Datum::Null, Datum::Text)),
            (
                Field::IndividualWinner,
                outcome.hero.map_or(Datum::Null, Datum::Text),
            ),
            (Field::Condition, Datum::Text(outcome.condition)),
        ] {
            objects
                .arena
                .set_field(step, objects.phases.k(field), ObjectValue::Atom(value))
                .map_err(unsupported)?;
        }
        Ok(())
    }
    pub(super) fn preflight_solo_win_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<Option<GameOutcome>> {
        let key = self.spatial_text_bound(objects, step, Field::HeroKey)?;
        let target = key
            .and_then(|key| objects.arena.context_value(key))
            .filter(|value| objects.arena.truthy(value));
        let hero = if let Some(target) = target {
            Some(self.resource_identity_bound(objects, target)?)
        } else {
            self.planning
                .actor
                .filter(|id| !self.symbols.name(*id).is_empty())
        };
        let Some((hero, model)) = hero.and_then(|hero| {
            self.hero_model_bound(objects, hero)
                .map(|model| (hero, model))
        }) else {
            return Ok(None);
        };
        let gold = self.combat_integer_bound(objects, model, Field::Gold)?;
        let threshold = self.combat_integer_bound(objects, step, Field::Threshold)?;
        if gold < threshold {
            return Ok(None);
        }
        self.phase_generation_check(0, 1)?;
        Ok(Some(GameOutcome {
            team: None,
            hero: Some(hero),
            condition: objects.phases.k(Field::FistfulVictory),
        }))
    }

    pub(super) fn solo_win_bound(
        &mut self,
        objects: &mut BoundObjects,
        outcome: GameOutcome,
    ) -> Result<()> {
        let step = self.phase_step(objects, "game_over")?;
        self.set_outcome_step_bound(objects, step, outcome)?;
        self.push_live_steps_bound(objects, &[step])
    }

    pub(super) fn preflight_game_over_bound(
        &self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<GameOutcome> {
        let team = self.passive_optional_text_bound(objects, step, Field::Winner)?;
        let hero = self.passive_optional_text_bound(objects, step, Field::IndividualWinner)?;
        if team.is_some() == hero.is_some() {
            return Err(unsupported("game over requires exactly one winner"));
        }
        self.combat_list_bound(objects, Field::InputStack)?;
        Ok(GameOutcome {
            team,
            hero,
            condition: self.passive_text_bound(objects, step, Field::Condition)?,
        })
    }
    pub(super) fn game_over_bound(
        &mut self,
        objects: &mut BoundObjects,
        outcome: GameOutcome,
    ) -> Result<NativeEvent> {
        for (field, value) in [
            (Field::Winner, outcome.team.map_or(Datum::Null, Datum::Text)),
            (
                Field::IndividualWinner,
                outcome.hero.map_or(Datum::Null, Datum::Text),
            ),
            (Field::VictoryCondition, Datum::Text(outcome.condition)),
            (
                Field::Phase,
                Datum::Text(objects.phases.k(Field::GameOverPhase)),
            ),
        ] {
            objects
                .arena
                .set_root(objects.phases.k(field), ObjectValue::Atom(value))
                .map_err(unsupported)?;
        }
        objects
            .arena
            .clear_list(objects.stack)
            .map_err(unsupported)?;
        let inputs = self.combat_list_bound(objects, Field::InputStack)?;
        objects.arena.clear_list(inputs).map_err(unsupported)?;
        self.stack.clear();
        self.world.combat.outcome = Some(outcome);
        self.planning.phase = Phase::GameOver;
        Ok(NativeEvent::Combat(CombatEvent::GameOver(outcome)))
    }
    fn protection_step_bound(
        &mut self,
        objects: &mut BoundObjects,
        spec: &ProtectionBinding,
        extra: Option<Symbol>,
    ) -> Result<ObjectId> {
        let step = self.phase_step(objects, "minion_protection")?;
        for (field, value) in [
            (Field::MinionId, Datum::Text(spec.minion)),
            (
                Field::KillerId,
                spec.killer.map_or(Datum::Null, Datum::Text),
            ),
        ] {
            objects
                .arena
                .set_field(step, objects.phases.k(field), ObjectValue::Atom(value))
                .map_err(unsupported)?;
        }
        let mut tried = spec
            .tried
            .iter()
            .copied()
            .map(|id| ObjectValue::Atom(Datum::Text(id)))
            .collect::<Vec<_>>();
        if let Some(id) = extra {
            tried.push(ObjectValue::Atom(Datum::Text(id)));
        }
        let tried = objects.arena.list(tried);
        objects
            .arena
            .set_field(step, objects.phases.k(Field::TriedEffects), tried)
            .map_err(unsupported)?;
        Ok(step)
    }
    pub(super) fn preflight_protection_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<ProtectionPlan> {
        let spec = ProtectionBinding {
            minion: self.passive_text_bound(objects, step, Field::MinionId)?,
            killer: self.passive_optional_text_bound(objects, step, Field::KillerId)?,
            tried: self
                .unit_filter_texts_bound(objects, step, Field::TriedEffects)?
                .into(),
        };
        let mut world = self.combat_world_bound(objects)?;
        let decision = world.protection_decision(&spec).map_err(unsupported)?;
        match decision {
            ProtectionDecision::Missing => Ok(ProtectionPlan::Missing),
            ProtectionDecision::Next(effect) => {
                self.phase_generation_check(0, 1)?;
                Ok(ProtectionPlan::Next { spec, effect })
            }
            ProtectionDecision::Sacrifice {
                effect,
                hero,
                token,
            } => {
                self.preflight_remove_spatial_bound(objects, token)?;
                Ok(ProtectionPlan::Sacrifice {
                    minion: spec.minion,
                    effect,
                    hero,
                    token,
                })
            }
            ProtectionDecision::Cards {
                effect,
                hero,
                cards,
            } => {
                let pending = objects.arena.field(step, objects.pending);
                if matches!(pending, None | Some(ObjectValue::Atom(Datum::Null))) {
                    return Ok(ProtectionPlan::Prompt {
                        minion: spec.minion,
                        hero,
                        cards,
                    });
                }
                let selection = if let Some(ObjectValue::Object(id)) = pending.as_ref().filter(|v| matches!(v, ObjectValue::Object(id) if objects.arena.kind(*id) == ObjectKind::Dict)) {
                    objects.arena.field(*id, objects.phases.k(Field::SelectedCardId)).filter(|v| objects.arena.truthy(v))
                        .or_else(|| objects.selected(step, self.selection_key))
                } else { pending };
                let selected = match selection {
                    Some(ObjectValue::Atom(Datum::Text(id))) => Some(id),
                    _ => None,
                };
                let card = selected.and_then(|id| {
                    cards
                        .iter()
                        .find(|card| {
                            world.definitions.cards[world.card(**card).unwrap().definition]
                                .printed_id
                                == id
                        })
                        .copied()
                });
                if let Some(card) = card {
                    let printed =
                        world.definitions.cards[world.card(card).unwrap().definition].printed_id;
                    let mut context = Registers::default();
                    let events = self
                        .combat_rules
                        .protection_defeat(
                            &mut world,
                            &mut context,
                            spec.minion,
                            spec.killer,
                            false,
                        )
                        .map_err(combat_error)?;
                    let commit = DefeatCommit {
                        world,
                        result: DefeatResult {
                            protection: false,
                            removals: Vec::new(),
                            events,
                            outcome: None,
                        },
                        context,
                        removed: Vec::new(),
                        hero: None,
                        normal: false,
                    };
                    self.preflight_combat_commit_bound(objects, &commit)?;
                    self.phase_generation_check(0, 1)?;
                    Ok(ProtectionPlan::Commit {
                        commit: Box::new(commit),
                        discard: Some((hero, printed)),
                        protected: Some((hero, effect, printed)),
                    })
                } else {
                    self.phase_generation_check(0, 1)?;
                    Ok(ProtectionPlan::Next { spec, effect })
                }
            }
            ProtectionDecision::Defeat => {
                let mut context = Registers::default();
                let events = self
                    .combat_rules
                    .protection_defeat(&mut world, &mut context, spec.minion, spec.killer, true)
                    .map_err(combat_error)?;
                let commit = DefeatCommit {
                    world,
                    result: DefeatResult {
                        protection: false,
                        removals: vec![spec.minion],
                        events,
                        outcome: None,
                    },
                    context,
                    removed: Vec::new(),
                    hero: None,
                    normal: false,
                };
                self.preflight_combat_commit_bound(objects, &commit)?;
                Ok(ProtectionPlan::Commit {
                    commit: Box::new(commit),
                    discard: None,
                    protected: None,
                })
            }
        }
    }
    pub(super) fn protection_bound(
        &mut self,
        objects: &mut BoundObjects,
        plan: ProtectionPlan,
    ) -> Result<(Option<NativeRequest>, Vec<NativeEvent>)> {
        match plan {
            ProtectionPlan::Missing => Ok((None, Vec::new())),
            ProtectionPlan::Next { spec, effect } => {
                let step = self.protection_step_bound(objects, &spec, Some(effect))?;
                self.push_live_steps_bound(objects, &[step])?;
                Ok((None, Vec::new()))
            }
            ProtectionPlan::Sacrifice {
                minion,
                effect,
                hero,
                token,
            } => {
                let before = self.world.effects.instances.len();
                let from = self
                    .remove_mine_spatial_bound(objects, token)?
                    .ok_or_else(|| unsupported("sacrifice position missing"))?;
                Ok((
                    None,
                    vec![
                        NativeEvent::TokenRemoved {
                            actor: Some(hero),
                            token,
                            from,
                            effects_removed: before - self.world.effects.instances.len(),
                        },
                        NativeEvent::Combat(CombatEvent::Protected {
                            hero,
                            minion,
                            effect,
                            card: None,
                            token: Some(token),
                        }),
                    ],
                ))
            }
            ProtectionPlan::Prompt {
                minion,
                hero,
                cards,
            } => Ok((
                Some(NativeRequest {
                    id: RequestToken::Generated(0),
                    controlled: None,
                    owner: hero,
                    prompt: minion,
                    movement_range: None,
                    pre_action_range: None,
                    choices: RequestChoices::Protection(cards),
                    can_skip: true,
                    discard: true,
                }),
                Vec::new(),
            )),
            ProtectionPlan::Commit {
                commit,
                discard,
                protected,
            } => {
                let minion = commit
                    .context
                    .get(objects.phases.k(Field::LastDefeatedMinion))
                    .cloned();
                let mut events = self.commit_defeat_bound(objects, *commit)?;
                if let Some((hero, card)) = discard {
                    let step = self.phase_step(objects, "discard_card")?;
                    for (field, value) in [
                        (Field::HeroId, Datum::Text(hero)),
                        (Field::CardId, Datum::Text(card)),
                    ] {
                        objects
                            .arena
                            .set_field(step, objects.phases.k(field), ObjectValue::Atom(value))
                            .map_err(unsupported)?;
                    }
                    self.push_live_steps_bound(objects, &[step])?;
                }
                if let Some((hero, effect, card)) = protected {
                    let Some(Datum::Text(minion)) = minion else {
                        return Err(unsupported("protected minion context missing"));
                    };
                    events.push(NativeEvent::Combat(CombatEvent::Protected {
                        hero,
                        minion,
                        effect,
                        card: Some(card),
                        token: None,
                    }));
                }
                Ok((None, events))
            }
        }
    }
}
