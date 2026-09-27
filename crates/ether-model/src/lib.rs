//! Canonical document model of Ethereal.
//!
//! - [`ids`]: typed ULID IDs + [`ids::IdGen`].
//! - Entities: [`track`], [`clip`], [`note`], [`automation`], [`device`], [`mixer`],
//!   [`tempo`], [`warp`], [`media`], gathered in [`project::Project`].
//! - Roadmap v2 entities (`.ether` v3): [`marker`], [`midi_map`], [`drum_rack`]; the
//!   reserved collaboration envelope is [`collab`].
//! - [`op`]: the op set (the only way to mutate), [`history`]: undo/redo,
//!   [`patch`]: UI mirror updates, [`file`]: `.ether` format + migrations.
//!
//! Pure data + logic: no I/O, no threads, no clock, no randomness (native + wasm32).
//! This crate's *types* are a frozen contract (changes via BCR); the logic (`todo!()`
//! bodies) is implemented by the `model` node.

mod apply;
pub mod automation;
pub mod clip;
pub mod collab;
pub mod device;
pub mod drum_rack;
pub mod entity;
pub mod error;
pub mod file;
pub mod history;
pub mod ids;
pub mod marker;
pub mod media;
pub mod midi_map;
pub mod mixer;
pub mod note;
pub mod op;
pub mod patch;
pub mod plugins;
pub mod project;
pub mod recording;
pub mod tempo;
pub mod track;
pub mod value;
pub mod warp;

pub use automation::*;
pub use clip::*;
pub use collab::*;
pub use device::*;
pub use drum_rack::*;
pub use entity::*;
pub use error::*;
pub use history::*;
pub use ids::*;
pub use marker::*;
pub use media::*;
pub use midi_map::*;
pub use mixer::*;
pub use note::*;
pub use op::*;
pub use patch::*;
pub use project::*;
pub use tempo::*;
pub use track::*;
pub use value::*;
pub use warp::*;
