//! Ethereal wire protocol: every UI ↔ engine message.
//!
//! - UI → engine: [`ClientMessage`] = request id + optional gesture + [`Command`].
//! - Engine → UI: [`ServerMessage`] = [`Reply`] | [`Event`] | playhead | meters.
//! - Document types (`Project`, entities, `Patch`) come from `ether-model` and are
//!   re-exported as [`model`]; IDs as [`ids`].
//!
//! Commands are split per domain (one file each) so domain nodes rarely collide. The whole
//! crate is a frozen contract: changes go through a BCR. TypeScript types are generated
//! from here (`just gen-types` → `ui/src/generated/`), never hand-written.
//!
//! JSON conventions: field names are the Rust snake_case names; enums are tagged with
//! `type` (internally tagged) unless noted; newtype IDs are ULID strings; `Beats`/`Seconds`
//! are numbers.

pub use ether_model as model;
pub use ether_model::ids;

pub mod automation;
pub mod clips;
pub mod collab;
pub mod devices;
pub mod drum_rack;
pub mod engine;
pub mod export;
pub mod groove;
pub mod markers;
pub mod media;
pub mod message;
pub mod meters;
pub mod midi_map;
pub mod mixer;
pub mod notes;
pub mod plugins;
pub mod project;
pub mod recording;
pub mod remote;
pub mod social;
pub mod tempo;
pub mod tracks;
pub mod transport;
pub mod ts;
pub mod warp;

pub use message::*;
