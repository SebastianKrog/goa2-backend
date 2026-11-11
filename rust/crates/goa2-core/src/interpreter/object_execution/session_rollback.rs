//! Speculative session branches and restoration of the live undo journals.

use std::{collections::HashMap, sync::Arc};

use super::{NativeSession, Result, unsupported};
use crate::objects::ObjectId;

pub(in crate::interpreter) struct RollbackCheckpoint {
    snapshot: NativeSession,
    rows: crate::world::WorldRowsCheckpoint,
    arena: crate::objects::ArenaCheckpoint,
    readonly: std::sync::OnceLock<Arc<crate::objects::ProtectedObjects>>,
    programs: Arc<HashMap<ObjectId, usize>>,
    copied_bindings: usize,
}

impl NativeSession {
    /// Explore an operation and restore the complete native session on exit.
    ///
    /// The arena, copied bindings, and card and hero rows use undo journals. The
    /// remaining typed session projection is checkpointed by value.
    pub fn with_rollback<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.rollback_scope(false, operation)
    }

    /// The operation must begin by submitting an already selected input.
    /// Its token is checked against the live step; the old request belongs
    /// to the rollback snapshot and need not be cloned into the branch.
    pub(in crate::interpreter) fn with_consumed_request<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        self.rollback_scope(true, operation)
    }

    fn rollback_scope<T>(
        &mut self,
        consume_request: bool,
        operation: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let checkpoint = self.begin_rollback_checkpoint(consume_request)?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(self)));
        self.restore_rollback_checkpoint(checkpoint);
        match result {
            Ok(value) => value,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }

    pub(in crate::interpreter) fn begin_rollback_checkpoint(
        &mut self,
        consume_request: bool,
    ) -> Result<RollbackCheckpoint> {
        let mut objects = self
            .objects
            .take()
            .ok_or_else(|| unsupported("session rollback requires live object bindings"))?;
        let request = if consume_request {
            self.last_request.take()
        } else {
            None
        };
        let rows = self.world.take_rows();
        let mut snapshot = self.clone();
        if consume_request {
            snapshot.last_request = request;
        }
        self.world.put_rows(rows);
        let rows_checkpoint = self.world.begin_rows_rollback();
        if self.count_observer.observed() {
            snapshot.count_observer.record();
        }
        let checkpoint = objects.arena.begin_rollback();
        let readonly = objects.readonly.clone();
        let programs = objects.programs.clone();
        let copied_bindings = objects.copied_binding_undo.0.len();
        self.objects = Some(objects);
        Ok(RollbackCheckpoint {
            snapshot,
            rows: rows_checkpoint,
            arena: checkpoint,
            readonly,
            programs,
            copied_bindings,
        })
    }

    pub(in crate::interpreter) fn restore_rollback_checkpoint(
        &mut self,
        checkpoint: RollbackCheckpoint,
    ) {
        let RollbackCheckpoint {
            mut snapshot,
            rows,
            arena,
            readonly,
            programs,
            copied_bindings,
        } = checkpoint;
        let mut objects = self
            .objects
            .take()
            .expect("speculation must preserve live bindings");
        objects.arena.rollback_to(arena);
        objects.readonly = readonly;
        objects.programs = programs;
        objects.restore_copied_bindings(copied_bindings);
        self.world.restore_rows(rows, &mut snapshot.world);
        snapshot.objects = Some(objects);
        *self = snapshot;
    }
}
