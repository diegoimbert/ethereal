//! MIDI expression (v0.3, owned by the `midi-expression` node; model
//! `ether_model::expression`, protocol `ether_protocol::expression`, engine
//! `ether_core::expression`; CONTRACTS.md §13.2).
//!
//! # Model in one paragraph
//! A MIDI clip has at most one **lane** per `ExpressionKind` (CC 0..=119, pitch bend,
//! channel pressure): one channel-wide curve in content-relative beats, so it moves, loops
//! and duplicates with the clip. A note has at most one **note expression** per
//! `NoteExpressionKind` (`Pressure` here; `Pitch`/`Timbre` are `mpe`'s): a per-note curve
//! in beats from the note start. A curve is one document value (`points`), edited whole
//! (`SetPoints`, `SetNoteExpression`) or by range (`ReplaceRange`, a pencil stroke).
//!
//! # This module
//! - [`expression_command`] ([`commands`]): every `ExpressionCommand` (document commands
//!   dispatched from `doc::apply`, one undo step each; `SetTrackMpe` is delegated to
//!   [`crate::mpe`]).
//! - [`track_expression`] ([`compile`]): compile hook (`compile.rs`): a MIDI track's clip
//!   lanes and note expressions → `TrackDesc::expression` (note indices in the clip's
//!   compiled, sorted note list; `mpe` from `Track::mpe`).
//! - [`copy`]: note copies outside `DocCtx::copy_clip` keep their expressions
//!   (`Note::Duplicate`, clip split, paste, consolidate, comp flatten).
//! - [`record`]: CC / pitch bend / channel pressure played while recording become lanes of
//!   the recorded clip, poly pressure the notes' `Pressure` expressions (thinned, see
//!   [`record::thin`]).
//! - Cascades are done (`doc/mod.rs`): deleting a clip removes its lanes, deleting a note
//!   its expressions; `DocCtx::copy_clip` copies both.
//!
//! # For `mpe`
//! Per-note `Pitch`/`Timbre` curves already go through every path here (commands, compile,
//! copies, engine rendering). `mpe` adds `SetTrackMpe`, the MPE reading of the recorded
//! MIDI (member channels → note expressions, next to [`record::note_expressions`]) and the
//! receivers.

mod commands;
mod compile;
pub(crate) mod copy;
pub(crate) mod record;

pub(crate) use commands::expression_command;
pub(crate) use compile::track_expression;

