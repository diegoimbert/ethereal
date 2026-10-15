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
    /// The file's format version is newer than this app reads (and no down path exists).
    #[error(
        "this file was made with a newer version of Ethereal (format version {found}; this \
         version reads up to {supported}). Update Ethereal to open it"
    )]
    TooNew { found: u32, supported: u32 },
    /// An `.ether` document from a pre-release build (`"version"` instead of
    /// `"format_version"`): not migrated (format 1 is the public-release baseline).
    #[error(
        "this project was saved by a pre-release build of Ethereal; its file format is no \
         longer supported"
    )]
    PreRelease,
    #[error("migration from version {from} failed: {message}")]
    Migration { from: u32, message: String },
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid project: {0}")]
    Invalid(#[from] ModelError),
}
