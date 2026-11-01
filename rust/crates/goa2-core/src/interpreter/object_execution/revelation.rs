//! Live committed-card reveal, two-card retrieval and initiative handoff.
use super::phases::Field;
use super::*;
use crate::{
    cards::CardId,
    planning::{ActorChoice, TieResolution},
};

impl NativeSession {
    fn hero_model_bound(&self, objects: &BoundObjects, hero: Symbol) -> Option<ObjectId> {
        objects
            .phases
            .heroes
            .iter()
            .find_map(|(id, model)| (*id == hero).then_some(*model))
    }
    fn model_card_bound(
        &self,
        objects: &BoundObjects,
        model: ObjectId,
        field: Field,
    ) -> Result<Option<CardId>> {
        match objects.arena.field(model, objects.phases.k(field)) {
            Some(ObjectValue::Atom(Datum::Null)) => Ok(None),
            Some(ObjectValue::Atom(Datum::Instance(card)))
                if self.world.card(CardId(card)).is_some() =>
            {
                Ok(Some(CardId(card)))
            }
            _ => Err(unsupported("phase physical-card binding missing")),
        }
    }
    fn sequence_ids_bound(&self, objects: &BoundObjects, list: ObjectId) -> Result<Vec<Symbol>> {
        objects
            .arena
            .sequence(list)
            .ok_or_else(|| unsupported("hero ID list missing"))?
            .iter()
            .map(|value| match value {
                ObjectValue::Atom(Datum::Text(id)) => Ok(*id),
                _ => Err(unsupported("hero ID must be text")),
            })
            .collect()
    }
    pub(super) fn actor_choice_bound(&self, objects: &BoundObjects) -> Result<ActorChoice> {
        let list = objects.phases.root(&objects.arena, Field::Unresolved)?;
        let ids = self.sequence_ids_bound(objects, list)?;
        let reversed = self
            .world
            .effects
            .initiative_reversed(self.round.round, self.world.combat.turn);
        crate::planning::rank_actors(&self.world, &ids, reversed).map_err(card_error)
    }
    fn hero_phase_step_bound(
        &mut self,
        objects: &mut BoundObjects,
        name: &str,
        hero: Symbol,
    ) -> Result<ObjectId> {
        let step = self.phase_step(objects, name)?;
        objects
            .arena
            .set_field(
                step,
                objects.phases.k(Field::HeroId),
                ObjectValue::Atom(Datum::Text(hero)),
            )
            .map_err(unsupported)?;
        Ok(step)
    }
    fn install_actor_bound(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
        tied: bool,
        flip: bool,
    ) -> Result<()> {
        let respawn = if tied {
            self.world.position(hero).is_none()
        } else {
            !self.world.has_board_presence(hero)
        };
        self.phase_generation_check(0, 3 + u64::from(respawn) + u64::from(flip))?;
        let list = objects.phases.root(&objects.arena, Field::Unresolved)?;
        let ids = self.sequence_ids_bound(objects, list)?;
        if let Some(index) = ids.iter().position(|id| *id == hero) {
            objects
                .arena
                .apply(&crate::objects::Mutation::Pop {
                    target: crate::objects::Target::Object(list),
                    index: index as i64,
                })
                .map_err(unsupported)?;
        }
        self.world.combat.unresolved = self.sequence_ids_bound(objects, list)?;
        objects
            .arena
            .set_root(objects.actor, ObjectValue::Atom(Datum::Text(hero)))
            .map_err(unsupported)?;
        objects
            .arena
            .set_root(
                objects.phases.k(Field::Owner),
                ObjectValue::Atom(Datum::Text(hero)),
            )
            .map_err(unsupported)?;
        self.planning.actor = Some(hero);
        self.planning.owner = Some(hero);
        let mut steps = Vec::new();
        if respawn {
            steps.push(self.hero_phase_step_bound(objects, "respawn", hero)?);
        }
        steps.push(self.hero_phase_step_bound(objects, "resolve_card", hero)?);
        steps.push(self.hero_phase_step_bound(objects, "confirm_resolution", hero)?);
        if flip {
            steps.push(self.phase_step(objects, "flip_coin")?);
        }
        steps.push(self.hero_phase_step_bound(objects, "finalize_hero", hero)?);
        self.push_live_steps_bound(objects, &steps)
    }
    pub(super) fn next_actor_bound(&mut self, objects: &mut BoundObjects) -> Result<()> {
        let choice = self.actor_choice_bound(objects)?;
        self.apply_actor_choice_bound(objects, choice)
    }
    pub(super) fn apply_actor_choice_bound(
        &mut self,
        objects: &mut BoundObjects,
        choice: ActorChoice,
    ) -> Result<()> {
        match choice {
            ActorChoice::Finished => self.end_turn_bound(objects),
            ActorChoice::Waiting => Ok(()),
            ActorChoice::Winner(hero) => self.install_actor_bound(objects, hero, false, false),
            ActorChoice::Tie(ids) => {
                self.phase_generation_check(0, 1)?;
                let step = self.phase_step(objects, "tie")?;
                let ids = objects.arena.list(
                    ids.into_iter()
                        .map(|id| ObjectValue::Atom(Datum::Text(id)))
                        .collect(),
                );
                objects
                    .arena
                    .set_field(step, objects.phases.k(Field::Tied), ids)
                    .map_err(unsupported)?;
                self.push_live_steps_bound(objects, &[step])
            }
        }
    }
    /// Source resolve_next_action, keeping the actual mutable unresolved list.
    pub fn resolve_next_actor(&mut self) -> Result<()> {
        self.with_live_objects(|session, objects| session.next_actor_bound(objects))
    }
    pub(super) fn tie_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
    ) -> Result<Option<NativeRequest>> {
        let Some(ObjectValue::Object(list)) =
            objects.arena.field(step, objects.phases.k(Field::Tied))
        else {
            return Err(unsupported("tied hero list missing"));
        };
        let ids = self.sequence_ids_bound(objects, list)?;
        let favored = objects.phases.k(if self.team == Team::Blue {
            Field::Blue
        } else {
            Field::Red
        });
        let selected = match objects.selected(step, self.selection_key) {
            Some(ObjectValue::Atom(value)) => Some(value),
            _ => None,
        };
        let resolution = crate::planning::choose_tie(&self.world, &ids, favored, selected.as_ref())
            .map_err(card_error)?;
        let consumes = matches!(
            &resolution,
            TieResolution::Choice(_) | TieResolution::Winner { prompted: true, .. }
        );
        if consumes
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
        match resolution {
            TieResolution::Finished => Ok(None),
            TieResolution::Winner { hero, flip, .. } => {
                self.install_actor_bound(objects, hero, true, flip)?;
                Ok(None)
            }
            TieResolution::Choice(choice) => Ok(Some(NativeRequest {
                id: RequestToken::Generated(0),
                controlled: None,
                owner: objects
                    .phases
                    .k(if choice.team == objects.phases.k(Field::Blue) {
                        Field::TeamBlue
                    } else {
                        Field::TeamRed
                    }),
                prompt: self.no_actor,
                movement_range: None,
                pre_action_range: None,
                choices: RequestChoices::Actors {
                    team: choice.team,
                    values: choice.candidates,
                },
                can_skip: false,
                discard: false,
            })),
        }
    }
    pub(super) fn pending_planning_open_bound(&self, objects: &BoundObjects) -> Result<bool> {
        let p = &objects.phases;
        let pending = p.root(&objects.arena, Field::Pending)?;
        let second = p.root(&objects.arena, Field::Second)?;
        let done = p.root(&objects.arena, Field::Done)?;
        let done = self.sequence_ids_bound(objects, done)?;
        for (hero, value) in objects
            .arena
            .fields(pending)
            .ok_or_else(|| unsupported("planning fields missing"))?
        {
            if !matches!(value, ObjectValue::Atom(Datum::Null))
                && self.planning.two_cards(&self.world, *hero)
                && objects.arena.field(second, *hero).is_none()
                && !done.contains(hero)
                && self
                    .world
                    .hero(*hero)
                    .is_some_and(|hero| !hero.hand.is_empty())
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
    fn reveal_knowledge_bound(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
        card: Symbol,
    ) -> Result<()> {
        let p = &objects.phases;
        let list = p.root(&objects.arena, Field::Revealed)?;
        let values = objects
            .arena
            .sequence(list)
            .ok_or_else(|| unsupported("revealed-card tuple missing"))?;
        if values.iter().any(|value| matches!(value, ObjectValue::Object(record)
            if objects.arena.field(*record,p.k(Field::HeroId)).is_some_and(|value| matches!(value,ObjectValue::Atom(Datum::Text(id)) if id==hero))
            && objects.arena.field(*record,p.k(Field::CardId)).is_some_and(|value| matches!(value,ObjectValue::Atom(Datum::Text(id)) if id==card)))) { return Ok(()); }
        let mut values = values.to_vec();
        let record = objects
            .arena
            .frozen_model(vec![
                (p.k(Field::HeroId), ObjectValue::Atom(Datum::Text(hero))),
                (p.k(Field::CardId), ObjectValue::Atom(Datum::Text(card))),
            ])
            .map_err(unsupported)?;
        values.push(record);
        let tuple = objects.arena.tuple(values);
        objects
            .arena
            .set_root(p.k(Field::Revealed), tuple)
            .map_err(unsupported)?;
        self.world.history.reveal(hero, card);
        Ok(())
    }
    fn reveal_card_bound(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
        card: CardId,
    ) -> Result<()> {
        let row = self
            .world
            .card(card)
            .ok_or_else(|| unsupported("revealed card missing"))?;
        let physical = row.source_instance;
        let printed = self.world.definitions.cards[row.definition].printed_id;
        self.world.card_mut(card).unwrap().facedown = false;
        objects
            .arena
            .set_physical_field(
                physical,
                objects.phases.k(Field::Facedown),
                Datum::Bool(false),
            )
            .map_err(unsupported)?;
        self.reveal_knowledge_bound(objects, hero, printed)
    }
    fn complete_pass_bound(
        &mut self,
        objects: &mut BoundObjects,
        hero: Symbol,
        model: ObjectId,
    ) -> Result<()> {
        let current = self.model_card_bound(objects, model, Field::Current)?;
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
        if let Some(card) = current {
            let physical = self
                .world
                .card(card)
                .ok_or_else(|| unsupported("pass card missing"))?
                .source_instance;
            objects
                .arena
                .set_physical_field(
                    physical,
                    objects.phases.k(Field::CardState),
                    Datum::Text(objects.phases.k(Field::Resolved)),
                )
                .map_err(unsupported)?;
            objects
                .arena
                .remove_physical_field(physical, objects.phases.k(Field::InitiativeOverride))
                .map_err(unsupported)?;
            objects
                .arena
                .set_index(
                    played,
                    self.world.combat.turn as usize - 1,
                    ObjectValue::Atom(Datum::Instance(physical)),
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
            .ok_or_else(|| unsupported("pass hero missing"))?
            .resolved_turns;
        objects
            .arena
            .set_field(
                model,
                objects.phases.k(Field::ResolvedCount),
                ObjectValue::Atom(Datum::Int(count)),
            )
            .map_err(unsupported)
    }
    /// Source start_revelation_phase, including two-card retrieval ordering.
    pub fn start_revelation(&mut self) -> Result<()> {
        self.with_live_objects(|session, objects| session.start_revelation_bound(objects))
    }
    pub(super) fn start_revelation_bound(&mut self, objects: &mut BoundObjects) -> Result<()> {
        let pending = objects.phases.root(&objects.arena, Field::Pending)?;
        let entries = objects
            .arena
            .fields(pending)
            .ok_or_else(|| unsupported("pending fields missing"))?
            .to_vec();
        self.phase_generation_check(
            self.world.effects.instances.len(),
            objects.phases.heroes.len() as u64 + 5,
        )?;
        self.phase_set(objects, Phase::Revelation)?;
        let ObjectValue::Object(unresolved) = objects.arena.list(Vec::new()) else {
            unreachable!()
        };
        objects
            .arena
            .set_root(
                objects.phases.k(Field::Unresolved),
                ObjectValue::Object(unresolved),
            )
            .map_err(unsupported)?;
        self.world.combat.unresolved.clear();
        let second = objects.phases.root(&objects.arena, Field::Second)?;
        for (hero, value) in entries {
            let Some(model) = self.hero_model_bound(objects, hero) else {
                continue;
            };
            match value {
                ObjectValue::Atom(Datum::Null) => self.complete_pass_bound(objects, hero, model)?,
                ObjectValue::Atom(Datum::Instance(instance)) => {
                    let card = CardId(instance);
                    self.reveal_card_bound(objects, hero, card)?;
                    objects
                        .arena
                        .set_field(
                            model,
                            objects.phases.k(Field::Current),
                            ObjectValue::Atom(Datum::Instance(instance)),
                        )
                        .map_err(unsupported)?;
                    let index = self
                        .world
                        .hero_index(hero)
                        .ok_or_else(|| unsupported("revealed hero missing"))?;
                    self.world.heroes[index].current = Some(card);
                    if let Some(ObjectValue::Atom(Datum::Instance(extra))) =
                        objects.arena.field(second, hero)
                    {
                        self.reveal_card_bound(objects, hero, CardId(extra))?;
                        objects
                            .arena
                            .set_field(
                                model,
                                objects.phases.k(Field::Extra),
                                ObjectValue::Atom(Datum::Instance(extra)),
                            )
                            .map_err(unsupported)?;
                        self.world.heroes[index].extra = Some(CardId(extra));
                    }
                    objects
                        .arena
                        .append(unresolved, ObjectValue::Atom(Datum::Text(hero)))
                        .map_err(unsupported)?;
                    self.world.combat.unresolved.push(hero);
                }
                _ => return Err(unsupported("pending card must be physical or null")),
            }
        }
        for field in [Field::Pending, Field::Second] {
            let empty = objects.arena.dictionary(Vec::new()).map_err(unsupported)?;
            objects
                .arena
                .set_root(objects.phases.k(field), empty)
                .map_err(unsupported)?;
        }
        let done = objects.arena.list(Vec::new());
        objects
            .arena
            .set_root(objects.phases.k(Field::Done), done)
            .map_err(unsupported)?;
        self.world.refresh_card_auras();
        self.phase_set(objects, Phase::Resolution)?;
        let mut steps = Vec::new();
        for hero in self.world.combat.unresolved.clone() {
            if self
                .world
                .hero(hero)
                .is_some_and(|hero| hero.extra.is_some())
            {
                steps.push(self.hero_phase_step_bound(objects, "retrieve_unresolved", hero)?);
            }
        }
        steps.extend(self.collect_after_cards_played_bound(objects)?);
        if steps.is_empty() {
            self.next_actor_bound(objects)
        } else {
            steps.push(self.phase_step(objects, "find_actor")?);
            self.push_live_steps_bound(objects, &steps)
        }
    }
    pub(super) fn retrieve_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        prompt: Symbol,
    ) -> Result<(Option<NativeRequest>, Option<NativeEvent>)> {
        let Some(ObjectValue::Atom(Datum::Text(hero))) =
            objects.arena.field(step, objects.phases.k(Field::HeroId))
        else {
            return Err(unsupported("retrieval hero missing"));
        };
        let Some(model) = self.hero_model_bound(objects, hero) else {
            return Ok((None, None));
        };
        let (Some(first), Some(second)) = (
            self.model_card_bound(objects, model, Field::Current)?,
            self.model_card_bound(objects, model, Field::Extra)?,
        ) else {
            return Ok((None, None));
        };
        self.world.lifecycle_hero(hero).map_err(card_error)?;
        let card_data = |id: CardId| -> Result<(Symbol, Symbol, i64)> {
            let card = self
                .world
                .card(id)
                .ok_or_else(|| unsupported("retrieval physical card missing"))?;
            let def = &self.world.definitions.cards[card.definition];
            Ok((
                def.printed_id,
                def.name,
                card.initiative_override.unwrap_or(def.initiative),
            ))
        };
        let a = card_data(first)?;
        let b = card_data(second)?;
        if let Some(ObjectValue::Atom(Datum::Text(chosen))) =
            objects.selected(step, self.selection_key)
            && (chosen == a.0 || chosen == b.0)
        {
            let (retrieved, kept, data, kept_printed) = if chosen == a.0 {
                (first, second, a, b.0)
            } else {
                (second, first, b, a.0)
            };
            let physical = self.world.card(retrieved).unwrap().source_instance;
            let hero_index = self.world.hero_index(hero).unwrap();
            self.world.heroes[hero_index].extra = None;
            self.world
                .return_card_to_hand(hero, retrieved)
                .map_err(card_error)?;
            let index = self.world.hero_index(hero).unwrap();
            self.world.heroes[index].current = Some(kept);
            objects
                .arena
                .set_field(
                    model,
                    objects.phases.k(Field::Extra),
                    ObjectValue::Atom(Datum::Null),
                )
                .map_err(unsupported)?;
            objects
                .arena
                .set_physical_field(
                    physical,
                    objects.phases.k(Field::CardState),
                    Datum::Text(objects.phases.k(Field::HandState)),
                )
                .map_err(unsupported)?;
            objects
                .arena
                .set_physical_field(
                    physical,
                    objects.phases.k(Field::Facedown),
                    Datum::Bool(false),
                )
                .map_err(unsupported)?;
            objects
                .arena
                .remove_physical_field(physical, objects.phases.k(Field::InitiativeOverride))
                .map_err(unsupported)?;
            let Some(ObjectValue::Object(hand)) =
                objects.arena.field(model, objects.phases.k(Field::Hand))
            else {
                return Err(unsupported("retrieval hand missing"));
            };
            objects
                .arena
                .append(hand, ObjectValue::Atom(Datum::Instance(physical)))
                .map_err(unsupported)?;
            for field in [Field::Played, Field::Discard] {
                let Some(ObjectValue::Object(list)) =
                    objects.arena.field(model, objects.phases.k(field))
                else {
                    return Err(unsupported("retrieval collection missing"));
                };
                if let Some(position)=objects.arena.sequence(list).and_then(|values|values.iter().position(|value|matches!(value,ObjectValue::Atom(Datum::Instance(id)) if *id==physical))) {
                    if matches!(field,Field::Played) { objects.arena.set_index(list,position,ObjectValue::Atom(Datum::Null)).map_err(unsupported)?; }
                    else { objects.arena.apply(&crate::objects::Mutation::Pop { target:crate::objects::Target::Object(list),index:position as i64 }).map_err(unsupported)?; }
                }
            }
            let kept_physical = self.world.card(kept).unwrap().source_instance;
            objects
                .arena
                .set_field(
                    model,
                    objects.phases.k(Field::Current),
                    ObjectValue::Atom(Datum::Instance(kept_physical)),
                )
                .map_err(unsupported)?;
            objects.readonly = Arc::new(objects.arena.readonly_objects());
            return Ok((
                None,
                Some(NativeEvent::CardRetrieved {
                    hero,
                    card: data.0,
                    name: data.1,
                    kept: Some(kept_printed),
                }),
            ));
        }
        if objects
            .arena
            .field(step, objects.pending)
            .is_some_and(|value| objects.arena.truthy(&value))
        {
            objects
                .arena
                .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
        }
        Ok((
            Some(NativeRequest {
                id: RequestToken::Generated(0),
                controlled: None,
                owner: hero,
                prompt,
                movement_range: None,
                pre_action_range: None,
                choices: RequestChoices::Cards(vec![a, b]),
                can_skip: false,
                discard: false,
            }),
            None,
        ))
    }
}
