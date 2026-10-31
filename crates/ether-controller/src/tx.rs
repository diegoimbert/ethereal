//! Transaction building and command errors.
//!
//! Document commands are translated into ops by applying each op to the live project right
//! away (so later steps of the same command, or later commands of the same `Batch`, see its
//! effect) while recording inverses. When the command finishes, [`Tx::finish`] rolls the
//! project back and hands the op list to `History::commit`, which re-applies it atomically
//! and records the undo step. On error [`Tx::rollback`] restores the project exactly.

use ether_core::protocol::model::{
    Entity, EntityKey, EntityUpdate, ModelError, Op, Project, SettingsChange,
};
use ether_core::protocol::{CommandError, ErrorCode};

pub type CmdResult<T> = Result<T, CommandError>;

pub fn cmd_err(code: ErrorCode, message: impl Into<String>) -> CommandError {
    CommandError::new(code, message)
}

/// An error a person meets in the UI: its catalog message (`error.*`), shown in their
/// language, with the English for agents and logs.
pub fn keyed_err(code: ErrorCode, key: ether_core::protocol::model::text::TextKey) -> CommandError {
    CommandError {
        code,
        message: crate::text::english(&key),
        key: Some(key),
    }
}

pub fn not_found(what: impl std::fmt::Display) -> CommandError {
    cmd_err(ErrorCode::NotFound, format!("{what} not found"))
}

pub fn invalid(message: impl Into<String>) -> CommandError {
    cmd_err(ErrorCode::InvalidArgument, message)
}

pub fn invalid_state(message: impl Into<String>) -> CommandError {
    cmd_err(ErrorCode::InvalidState, message)
}

pub fn unsupported(message: impl Into<String>) -> CommandError {
    cmd_err(ErrorCode::Unsupported, message)
}

pub fn internal(message: impl Into<String>) -> CommandError {
    cmd_err(ErrorCode::Internal, message)
}

/// Map a model error to the wire error code.
pub fn model_err(e: ModelError) -> CommandError {
    let code = match &e {
        ModelError::NotFound(_) | ModelError::DanglingReference { .. } => ErrorCode::NotFound,
        ModelError::AlreadyExists(_) | ModelError::Invariant(_) | ModelError::InvalidValue(_) => {
            ErrorCode::InvalidArgument
        }
        // The controller removes children first; this is a controller bug.
        ModelError::HasChildren(_) => ErrorCode::Internal,
    };
    cmd_err(code, e.to_string())
}

/// Ops applied to the live project, with their inverses.
pub struct Tx<'a> {
    pub project: &'a mut Project,
    ops: Vec<Op>,
    inverses: Vec<Op>,
    /// While [`Self::defer_checks`] is on: the kinds of table whose document-wide checks are
    /// left to [`Self::end_deferred`].
    deferred: Option<Vec<EntityKey>>,
}

impl<'a> Tx<'a> {
    pub fn new(project: &'a mut Project) -> Self {
        // Commands read children through the parent → children indexes.
        project.ensure_index();
        Self {
            project,
            ops: Vec::new(),
            inverses: Vec::new(),
            deferred: None,
        }
    }

    pub fn p(&self) -> &Project {
        self.project
    }

    /// The ops applied so far.
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn apply(&mut self, op: Op) -> CmdResult<()> {
        let inverse = match &mut self.deferred {
            Some(kinds) => self.project.apply_deferred(&op, kinds),
            None => self.project.apply(&op),
        }
        .map_err(model_err)?;
        self.ops.push(op);
        self.inverses.push(inverse);
        Ok(())
    }

    pub fn insert(&mut self, entity: Entity) -> CmdResult<()> {
        self.apply(Op::Insert { entity })
    }

    /// Apply many ops at once, all or nothing: each is checked on its own and the
    /// document-wide invariants (routing, sidechains) once at the end ([`Project::apply_all`]),
    /// where one [`Self::apply`] per op re-checks them over the whole project every time.
    pub fn apply_all(&mut self, ops: Vec<Op>) -> CmdResult<()> {
        let inverses = self.project.apply_all(&ops).map_err(model_err)?;
        self.ops.extend(ops);
        // `apply_all` returns them last op first; `unwind` takes them from the end.
        self.inverses.extend(inverses.into_iter().rev());
        Ok(())
    }

    /// From now on each op gets only its own checks; the document-wide ones (routing,
    /// sidechains, ...) run once in [`Self::end_deferred`]. For commands made of many
    /// structural ops (deleting or duplicating a big group): checking the whole project
    /// after each op made them quadratic. Returns false when already deferring (the
    /// outer scope ends it).
    pub fn defer_checks(&mut self) -> bool {
        if self.deferred.is_some() {
            return false;
        }
        self.deferred = Some(Vec::new());
        true
    }

    /// End what [`Self::defer_checks`] started (`started` = what it returned) and run the
    /// deferred checks. On error the project may break document-wide invariants: the
    /// caller propagates it and the transaction is rolled back.
    pub fn end_deferred(&mut self, started: bool) -> CmdResult<()> {
        if !started {
            return Ok(());
        }
        let kinds = self.deferred.take().unwrap_or_default();
        self.project.check_deferred(&kinds).map_err(model_err)
    }

    pub fn remove(&mut self, key: EntityKey) -> CmdResult<()> {
        self.apply(Op::Remove { key })
    }

    pub fn update(&mut self, update: EntityUpdate) -> CmdResult<()> {
        self.apply(Op::Update { update })
    }

    pub fn settings(&mut self, change: SettingsChange) -> CmdResult<()> {
        self.apply(Op::Settings { change })
    }

    /// Undo everything applied so far (error path).
    pub fn rollback(mut self) {
        self.unwind();
    }

    fn unwind(&mut self) {
        // Inverses of applied ops always apply (model guarantee). As one batch: the
        // document-wide checks once, not once per op (every command's ops are unwound here,
        // and checking routing after each one made big edits quadratic).
        let inverses: Vec<Op> = self.inverses.drain(..).rev().collect();
        if self.project.apply_all(&inverses).is_err() {
            // All or nothing: nothing was applied. Op by op, as before.
            for inv in &inverses {
                let _ = self.project.apply(inv);
            }
        }
        self.ops.clear();
    }

    /// Roll the project back and return the ops, ready for `History::commit`.
    pub fn finish(mut self) -> Vec<Op> {
        let ops = std::mem::take(&mut self.ops);
        self.unwind();
        ops
    }
}
