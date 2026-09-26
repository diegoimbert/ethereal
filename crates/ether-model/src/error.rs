use crate::entity::EntityKey;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ModelError {
    #[error("entity not found: {0:?}")]
    NotFound(EntityKey),
    #[error("entity already exists: {0:?}")]
    AlreadyExists(EntityKey),
    #[error("{entity:?} references missing {missing:?}")]
    DanglingReference {
        entity: EntityKey,
        missing: EntityKey,
    },
    #[error("cannot remove {0:?}: it still has children")]
    HasChildren(EntityKey),
    #[error("invariant violated: {0}")]
    Invariant(String),
    #[error("invalid value: {0}")]
    InvalidValue(String),
}

#[derive(Debug, thiserror::Error)]
pub enum FileError {
    #[error("not an .ether file: {0}")]
    NotAnEtherFile(String),
    #[error("file version {found} is newer than supported version {supported}")]
    TooNew { found: u32, supported: u32 },
    #[error("migration from version {from} failed: {message}")]
    Migration { from: u32, message: String },
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid project: {0}")]
    Invalid(#[from] ModelError),
}
