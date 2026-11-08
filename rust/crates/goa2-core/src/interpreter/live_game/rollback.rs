//! Live rollback anchors and authorization. Search sessions never keep anchors.
use super::{ExecutionBudget, GameTransition, LiveGame, NativeSession, Result, invalid};
use crate::{
    codec::{Phase, StateEnvelope},
    data::{Datum, Symbol},
    objects::ObjectValue,
};
use std::sync::Arc;

const FROZEN: &str = "rollback_frozen";
const REANCHOR: &str = "rollback_reanchor_pending";

impl LiveGame {
    pub(super) fn restore_rollback(
        &mut self,
        envelope: Option<StateEnvelope>,
        budget: &ExecutionBudget<'_>,
    ) -> Result<()> {
        let Some(envelope) = envelope else {
            return Ok(());
        };
        let (anchor, _) = Self::open(envelope, 0, None, budget)?;
        if !anchor.session.owner_prompt()
            || anchor.session.confirmation_prompt()
            || anchor.session.rollback_flag(FROZEN)
            || anchor.session.rollback_flag(REANCHOR)
        {
            return Err(invalid("Invalid rollback anchor"));
        }
        self.rollback = Some(Arc::new(anchor.session));
        if !self.can_rollback() {
            return Err(invalid(
                "Rollback anchor belongs to another decision segment",
            ));
        }
        Ok(())
    }

    pub(super) fn can_rollback(&self) -> bool {
        self.session.owner_prompt()
            && !self.session.rollback_flag(FROZEN)
            && !self.session.rollback_flag(REANCHOR)
            && self.anchor_matches_turn()
    }

    fn anchor_matches_turn(&self) -> bool {
        self.rollback.as_ref().is_some_and(|anchor| {
            self.session.resolution_owner() == anchor.resolution_owner()
                && self.session.round.round == anchor.round.round
                && self.session.world.combat.turn == anchor.world.combat.turn
        })
    }

    pub(super) fn update_rollback(&mut self) -> Result<()> {
        if self.session.resolution_owner().is_none() || self.session.rollback_flag(FROZEN) {
            self.rollback = None;
            return Ok(());
        }
        if !self.anchor_matches_turn() || self.session.rollback_flag(REANCHOR) {
            self.rollback = None;
        }
        if self.session.last_request.is_none() {
            return Ok(());
        }
        if !self.session.owner_prompt() {
            self.rollback = None;
            return self.session.set_reanchor(true);
        }
        if self.rollback.is_none() && !self.session.confirmation_prompt() {
            self.session.set_reanchor(false)?;
            self.rollback = Some(Arc::new(self.session.clone()));
        }
        Ok(())
    }

    /// Restore a verified journal position while continuing the live revision
    /// and request generations. The caller commits before publishing it.
    pub fn prepare_table_rewind(
        &self,
        mut target: Self,
        now: u64,
    ) -> Result<(Self, GameTransition)> {
        if target.revision > self.revision {
            return Err(invalid("Rewind target is in the future"));
        }
        target.revision = self
            .revision
            .checked_add(1)
            .ok_or_else(|| invalid("game revision exhausted"))?;
        target.session.next_request = target.session.next_request.max(self.session.next_request);
        target.session.next_step = target.session.next_step.max(self.session.next_step);
        if let Some(request) = &target.session.last_request {
            let previous = target.session.request_token(request.id);
            target.session.refresh_pending_request()?;
            let next = target
                .session
                .request_token(target.session.last_request.as_ref().unwrap().id);
            target.preserve_response_credit(&previous, next);
        }
        target.resume_rewound_clock(now)?;
        let transition = GameTransition {
            position: target.position(),
            events: Vec::new(),
            previous_phase: self.position().phase,
        };
        Ok((target, transition))
    }

    pub(super) fn rewind(&mut self, actor: Symbol) -> Result<()> {
        if !self.can_rollback()
            || !self
                .session
                .last_request
                .as_ref()
                .is_some_and(|request| request.owner == actor)
        {
            return Err(invalid("No rollback available for this player"));
        }
        let anchor = self
            .rollback
            .as_ref()
            .ok_or_else(|| invalid("No rollback anchor"))?;
        let mut restored = (**anchor).clone();
        let anchor_request = restored
            .last_request
            .as_ref()
            .ok_or_else(|| invalid("Missing rollback request"))?;
        let previous_id = restored.request_token(anchor_request.id);
        restored.next_request = restored.next_request.max(self.session.next_request);
        restored.next_step = restored.next_step.max(self.session.next_step);
        restored.refresh_pending_request()?;
        let next_id = restored.request_token(restored.last_request.as_ref().unwrap().id);
        self.preserve_response_credit(&previous_id, next_id);
        self.session = restored;
        Ok(())
    }
}

impl NativeSession {
    fn resolution_owner(&self) -> Option<&str> {
        if self.planning.phase != Phase::Resolution || self.planning.actor.is_none() {
            return None;
        }
        let owner = self.planning.owner.or(self.planning.actor)?;
        Some(self.symbols.name(self.normalized_owner(owner)))
    }

    fn owner_prompt(&self) -> bool {
        self.resolution_owner().is_some_and(|owner| {
            self.last_request.as_ref().is_some_and(|request| {
                self.symbols.name(request.owner) == owner
                    || request
                        .controlled
                        .is_some_and(|actor| self.symbols.name(actor) == owner)
            })
        })
    }

    fn confirmation_prompt(&self) -> bool {
        self.last_request.as_ref().is_some_and(|request| {
            matches!(request.choices, super::super::RequestChoices::ConfirmAction)
        })
    }

    fn rollback_flag(&self, name: &str) -> bool {
        self.symbols.lookup(name).is_some_and(|key| {
            if let Some(objects) = &self.objects {
                objects
                    .arena
                    .context_value(key)
                    .is_some_and(|value| objects.arena.truthy(&value))
            } else {
                self.context
                    .get(key)
                    .is_some_and(|value| value.truthy(&self.symbols))
            }
        })
    }

    fn set_reanchor(&mut self, pending: bool) -> Result<()> {
        self.set_rollback_flag(REANCHOR, pending)
    }

    pub(super) fn freeze_rollback(&mut self) -> Result<()> {
        self.set_rollback_flag(FROZEN, true)
    }

    fn set_rollback_flag(&mut self, name: &str, pending: bool) -> Result<()> {
        let key = self
            .symbols
            .lookup(name)
            .unwrap_or_else(|| Arc::make_mut(&mut self.symbols).intern(name));
        if let Some(objects) = &mut self.objects {
            let result = if pending {
                objects
                    .arena
                    .set_context(key, ObjectValue::Atom(Datum::Bool(true)))
            } else {
                objects.arena.remove_context(key)
            };
            return result.map_err(super::super::unsupported);
        }
        if pending {
            self.context.set(key, Datum::Bool(true));
        } else {
            self.context.remove(key);
        }
        Ok(())
    }

    fn refresh_pending_request(&mut self) -> Result<()> {
        use super::super::RequestToken;
        let id = self
            .next_request
            .checked_add(1)
            .ok_or_else(|| invalid("request generation counter exhausted"))?;
        let frame = self
            .stack
            .last_mut()
            .ok_or_else(|| invalid("Missing rollback step"))?;
        let request = self
            .last_request
            .as_mut()
            .ok_or_else(|| invalid("Missing rollback request"))?;
        let token = RequestToken::Generated(id);
        if let Some(objects) = &mut self.objects {
            objects.refresh_pending_request(
                frame
                    .object
                    .ok_or_else(|| invalid("Missing rollback step binding"))?,
                id,
            )?;
        }
        frame.request = Some(token);
        request.id = token;
        self.next_request = id;
        Ok(())
    }
}
