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
