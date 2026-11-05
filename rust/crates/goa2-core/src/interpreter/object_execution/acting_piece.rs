//! Bind acting pieces using the same per-piece action legality as the menu.
use super::phases::Field;
use super::*;
use crate::{data::Registers, world::ActionKind};
pub(super) enum PiecePlan {
    Done,
    Bind(Symbol),
    Prompt { request: NativeRequest, clear: bool },
}
fn action(name: &str) -> Result<ActionKind> {
    Ok(match name {
        "MOVEMENT" => ActionKind::Movement,
        "ATTACK" => ActionKind::Attack,
        "SKILL" => ActionKind::Skill,
        "DEFENSE" => ActionKind::Defense,
        "DEFENSE_SKILL" => ActionKind::DefenseSkill,
        "HOLD" => ActionKind::Hold,
        "CLEAR" => ActionKind::Clear,
        "FAST_TRAVEL" => ActionKind::FastTravel,
        _ => return Err(unsupported("acting-piece action type not bound")),
    })
}
impl NativeSession {
    pub(super) fn remove_piece_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        events: &mut Vec<NativeEvent>,
    ) -> Result<Option<NativeRequest>> {
        let owner = self.passive_text_bound(objects, step, Field::HeroId)?;
        if !self
            .world
            .entity(owner)
            .is_some_and(|entity| entity.multi_piece)
        {
            return Ok(None);
        }
        let mut pieces: Vec<_> = self.world.piece_ids(owner).collect();
        let mode_field = self
            .symbols
            .lookup("mode")
            .ok_or_else(|| unsupported("piece mode missing"))?;
        let Some(ObjectValue::Atom(Datum::Text(mode))) = objects.arena.field(step, mode_field)
        else {
            return Err(unsupported("piece removal mode invalid"));
        };
        let mut continuation = None;
        let selected = if self.symbols.name(mode) == "all_others" {
            let keep = self
                .world
                .acting_piece
                .filter(|id| pieces.contains(id))
                .or_else(|| pieces.first().copied());
            pieces.retain(|id| Some(*id) != keep);
            pieces
        } else {
            let min = self
                .symbols
                .lookup("min_remaining")
                .and_then(|key| objects.arena.field(step, key));
            let Some(ObjectValue::Atom(Datum::Int(min))) = min else {
                return Err(unsupported("minimum remaining pieces missing"));
            };
            if pieces.len() as i64 <= min {
                return Ok(None);
            }
            let answer = objects.selected(step, self.selection_key);
            if matches!(answer, Some(ObjectValue::Atom(Datum::Text(value))) if value == self.skip) {
                return Ok(None);
            }
            if let Some(ObjectValue::Atom(Datum::Text(piece))) = answer
                && pieces.contains(&piece)
            {
                if self.symbols.name(mode) == "choose_any" {
                    self.phase_generation_check(0, 1)?;
                    let (next, _) = objects.copy_template(step)?;
                    let program = objects.scheduler.as_ref().unwrap();
                    self.next_step += 1;
                    for (field, value) in [
                        (program.step_id, ObjectValue::GeneratedStep(self.next_step)),
                        (objects.pending, ObjectValue::Atom(Datum::Null)),
                        (
                            objects.phases.k(Field::ActiveIf),
                            ObjectValue::Atom(Datum::Null),
                        ),
                        (
                            objects.phases.k(Field::SkipIf),
                            ObjectValue::Atom(Datum::Null),
                        ),
                        (
                            objects.phases.k(Field::Mandatory),
                            ObjectValue::Atom(Datum::Bool(false)),
                        ),
                    ] {
                        objects
                            .arena
                            .set_field(next, field, value)
                            .map_err(unsupported)?;
                    }
                    continuation = Some(next);
                }
                vec![piece]
            } else {
                objects
                    .arena
                    .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                    .map_err(unsupported)?;
                let prompt = format!("Remove one of you? ({} in play)", pieces.len());
                let prompt = Arc::make_mut(&mut self.symbols).intern(&prompt);
                pieces.push(self.skip);
                return Ok(Some(NativeRequest {
                    controlled: None,
                    id: RequestToken::Generated(0),
                    owner,
                    prompt,
                    movement_range: None,
                    pre_action_range: None,
                    choices: RequestChoices::Units {
                        tokens: false,
                        values: pieces.into_iter().map(|piece| (piece, None)).collect(),
                    },
                    can_skip: true,
                    discard: false,
                }));
            }
        };
        let markers = objects.phases.root(&objects.arena, Field::Markers)?;
        for piece in selected {
            for marker in self
                .world
                .markers
                .iter_mut()
                .filter(|row| row.target == Some(piece))
            {
                let Some(ObjectValue::Object(model)) = objects.arena.field(markers, marker.key)
                else {
                    return Err(unsupported("piece marker missing"));
                };
                for (field, value) in [
                    (Field::TargetId, Datum::Null),
                    (Field::SourceId, Datum::Null),
                    (Field::Value, Datum::Int(0)),
                ] {
                    objects
                        .arena
                        .set_field(model, objects.phases.k(field), ObjectValue::Atom(value))
                        .map_err(unsupported)?;
                }
                marker.target = None;
                marker.source = None;
                marker.value = 0;
            }
            let from = self.remove_entity_spatial_bound(objects, piece)?;
            if self.world.acting_piece == Some(piece) {
                self.world.acting_piece = None;
                objects
                    .arena
                    .set_root(
                        objects.phases.k(Field::Piece),
                        ObjectValue::Atom(Datum::Null),
                    )
                    .map_err(unsupported)?;
            }
            events.push(NativeEvent::HeroPieceRemoved { owner, piece, from });
        }
        if let Some(next) = continuation {
            self.push_live_steps_bound(objects, &[next])?;
        }
        Ok(None)
    }

    pub(super) fn spawn_piece_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        events: &mut Vec<NativeEvent>,
    ) -> Result<Option<NativeRequest>> {
        let owner = self.passive_text_bound(objects, step, Field::HeroId)?;
        let Some(hero) = self.world.hero(owner) else {
            return Ok(None);
        };
        let count = self
            .spatial_int_bound(objects, step, Field::MaxCount)?
            .unwrap_or(0);
        if count <= 0 {
            return Ok(None);
        }
        let mut supply = hero
            .pieces
            .iter()
            .copied()
            .filter(|id| self.world.position(*id).is_none());
        let Some(piece) = supply.next() else {
            return Ok(None);
        };
        let available = 1 + supply.count();
        let origin_id = self
            .passive_optional_text_bound(objects, step, Field::OriginKey)?
            .map(|key| self.truthy_symbol_bound(objects, objects.arena.context_value(key)))
            .transpose()?
            .flatten()
            .unwrap_or(owner);
        let Some(origin) = self.world.resolved_position(origin_id) else {
            return Ok(None);
        };
        let radius_key = self
            .symbols
            .lookup("radius")
            .ok_or_else(|| unsupported("spawn radius missing"))?;
        let Some(ObjectValue::Atom(Datum::Int(radius))) = objects.arena.field(step, radius_key)
        else {
            return Err(unsupported("spawn radius must be numeric"));
        };
        let actor = self.planning.actor.or(Some(owner));
        let mut choices = Vec::new();
        for &hex in self.world.tile_hexes() {
            if hex != origin
                && crate::queries::topology_distance(&self.world, origin, hex)
                    .map_err(unsupported)?
                    .threshold_order(radius)
                    .is_le()
                && !self
                    .world
                    .obstacle_for_actor(hex, actor)
                    .map_err(unsupported)?
            {
                choices.push(hex);
            }
        }
        if choices.is_empty() {
            return Ok(None);
        }
        if let Some(selection) = objects.selected(step, self.selection_key) {
            if matches!(selection, ObjectValue::Atom(Datum::Text(value)) if value == self.skip) {
                return Ok(None);
            }
            if let Ok((hex, _)) = self.spatial_hex_bound(objects, &selection)
                && choices.contains(&hex)
            {
                self.phase_generation_check(0, 1)?;
                let placement = self.prepare_spatial_bound(objects, piece, hex, None)?;
                let (next, _) = objects.copy_template(step)?;
                let program = objects.scheduler.as_ref().unwrap();
                self.next_step += 1;
                for (field, value) in [
                    (program.step_id, ObjectValue::GeneratedStep(self.next_step)),
                    (objects.pending, ObjectValue::Atom(Datum::Null)),
                    (
                        objects.phases.k(Field::ActiveIf),
                        ObjectValue::Atom(Datum::Null),
                    ),
                    (
                        objects.phases.k(Field::SkipIf),
                        ObjectValue::Atom(Datum::Null),
                    ),
                    (
                        objects.phases.k(Field::Mandatory),
                        ObjectValue::Atom(Datum::Bool(false)),
                    ),
                    (
                        objects.phases.k(Field::MaxCount),
                        ObjectValue::Atom(Datum::Int(count - 1)),
                    ),
                ] {
                    objects
                        .arena
                        .set_field(next, field, value)
                        .map_err(unsupported)?;
                }
                self.apply_spatial_bound(objects, &placement)?;
                events.push(NativeEvent::HeroPieceSpawned {
                    owner,
                    piece,
                    to: hex,
                });
                self.push_live_steps_bound(objects, &[next])?;
                return Ok(None);
            }
            objects
                .arena
                .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                .map_err(unsupported)?;
        }
        let prompt = format!(
            "Spawn another {}? ({available} in supply)",
            self.symbols.name(self.world.entity(owner).unwrap().name)
        );
        let prompt = Arc::make_mut(&mut self.symbols).intern(&prompt);
        Ok(Some(NativeRequest {
            controlled: None,
            id: RequestToken::Generated(0),
            owner,
            prompt,
            movement_range: None,
            pre_action_range: None,
            choices: RequestChoices::Hexes(choices),
            can_skip: true,
            discard: false,
        }))
    }

    pub(super) fn preflight_acting_piece_bound(
        &mut self,
        objects: &BoundObjects,
        step: ObjectId,
    ) -> Result<PiecePlan> {
        let hero = self.passive_text_bound(objects, step, Field::HeroId)?;
        let Some(owner) = self.world.hero(hero).map(|h| h.id) else {
            return Ok(PiecePlan::Done);
        };
        if !self.world.entity(owner).is_some_and(|e| e.multi_piece) {
            return Ok(PiecePlan::Done);
        }
        self.sync_card_collections_bound(objects)?;
        let mut pieces = self.world.piece_ids(hero).collect::<Vec<_>>();
        if pieces.is_empty() {
            return Ok(PiecePlan::Done);
        }
        if let Some(value) = objects
            .arena
            .context_value(objects.action_keys[0])
            .filter(|v| !matches!(v, ObjectValue::Atom(Datum::Null)))
        {
            let ObjectValue::Atom(Datum::Text(name)) = value else {
                return Err(unsupported("acting-piece action must be text"));
            };
            let kind = action(self.symbols.name(name))?;
            let card = self.world.hero(owner).unwrap().current;
            let mut legal = Vec::new();
            for piece in &pieces {
                if self
                    .world
                    .action_legality(*piece, kind, card, &Registers::default())
                    .map_err(unsupported)?
                    .allowed
                {
                    legal.push(*piece);
                }
            }
            if !legal.is_empty() {
                pieces = legal;
            }
        }
        if pieces.len() == 1 {
            return Ok(PiecePlan::Bind(pieces[0]));
        }
        if let Some(ObjectValue::Atom(Datum::Text(id))) = objects.selected(step, self.selection_key)
            && pieces.contains(&id)
        {
            return Ok(PiecePlan::Bind(id));
        }
        let clear = objects
            .arena
            .field(step, objects.pending)
            .is_some_and(|value| objects.arena.truthy(&value));
        if self.next_request == u64::MAX && objects.token(step)?.is_none() {
            return Err(unsupported("request generation counter exhausted"));
        }
        let name = self.world.entity(owner).unwrap().name;
        let prompt = self
            .symbols
            .lookup("Choose which of you performs this action.")
            .ok_or_else(|| unsupported("acting-piece prompt missing"))?;
        Ok(PiecePlan::Prompt {
            request: NativeRequest {
                controlled: None,
                id: RequestToken::Generated(0),
                owner: hero,
                prompt,
                movement_range: None,
                pre_action_range: None,
                choices: RequestChoices::Units {
                    tokens: false,
                    values: pieces
                        .into_iter()
                        .map(|id| (id, self.world.position(id).map(|hex| (name, hex))))
                        .collect(),
                },
                can_skip: false,
                discard: false,
            },
            clear,
        })
    }
    pub(super) fn acting_piece_bound(
        &mut self,
        objects: &mut BoundObjects,
        step: ObjectId,
        plan: PiecePlan,
    ) -> Result<Option<NativeRequest>> {
        match plan {
            PiecePlan::Done => {}
            PiecePlan::Bind(piece) => {
                objects
                    .arena
                    .set_root(
                        objects.phases.k(Field::Piece),
                        ObjectValue::Atom(Datum::Text(piece)),
                    )
                    .map_err(unsupported)?;
                self.world.acting_piece = Some(piece);
            }
            PiecePlan::Prompt { request, clear } => {
                if clear {
                    objects
                        .arena
                        .set_field(step, objects.pending, ObjectValue::Atom(Datum::Null))
                        .map_err(unsupported)?;
                }
                return Ok(Some(request));
            }
        }
        Ok(None)
    }
}
