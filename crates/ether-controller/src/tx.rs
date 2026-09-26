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
    CommandError {
        code,
        message: message.into(),
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
}

impl<'a> Tx<'a> {
    pub fn new(project: &'a mut Project) -> Self {
        Self {
            project,
            ops: Vec::new(),
            inverses: Vec::new(),
        }
    }

    pub fn p(&self) -> &Project {
        self.project
    }

    pub fn apply(&mut self, op: Op) -> CmdResult<()> {
        let inverse = self.project.apply(&op).map_err(model_err)?;
        self.ops.push(op);
        self.inverses.push(inverse);
        Ok(())
    }

    pub fn insert(&mut self, entity: Entity) -> CmdResult<()> {
        self.apply(Op::Insert { entity })
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

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// Undo everything applied so far (error path).
    pub fn rollback(mut self) {
        self.unwind();
    }

    fn unwind(&mut self) {
        while let Some(inv) = self.inverses.pop() {
            // Inverses of applied ops always apply (model guarantee).
            let _ = self.project.apply(&inv);
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
