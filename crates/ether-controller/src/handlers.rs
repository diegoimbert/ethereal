//! Message dispatch and everything that is not project lifecycle: edits (commit → patch →
//! engine effects), transport, recording, plugins, media, and the tick.

use std::sync::Arc;

use ether_core::TransportControl;
use ether_core::plugin::PluginNotification;
use ether_core::protocol::devices::DeviceCommand;
use ether_core::protocol::media::{
    BrowseLocation, BrowseRoot, DirectoryListing, FileKind, MediaCommand, MediaEvent, MediaSource,
};
use ether_core::protocol::meters::MeterFrame;
use ether_core::protocol::model::file::MEDIA_DIR;
use ether_core::protocol::model::*;
use ether_core::protocol::plugins::{PluginCommand, PluginEvent};
use ether_core::protocol::project::{EditCommand, ProjectCommand, ProjectEvent};
use ether_core::protocol::recording::RecordingEvent;
use ether_core::protocol::transport::{PlayheadUpdate, TransportCommand, TransportState};
use ether_core::protocol::warp::WarpCommand;
use ether_core::protocol::{
    ClientMessage, Command, CommandError, ErrorCode, Event, NotificationLevel, PlayheadFrame,
    ReplyValue, ServerMessage,
};

use crate::doc::{self, DocCtx, DocHost};
use crate::engine::{EngineCtx, bridge_err};
use crate::media::{IncrementalDecoder, chained_ogg_warning, extension_of, is_chained_ogg};
use crate::saving::SaveKind;
use crate::store::{Library, ProjectStore, StoreError, check_relative_path};
use crate::tx::{
    CmdResult, Tx, cmd_err, internal, invalid, invalid_state, model_err, not_found, unsupported,
};
use crate::{EngineBridge, EtherController, HostServices, MessageSink, content_hash};

/// Taps further apart than this start a new tap-tempo sequence.
const TAP_RESET_MS: u64 = 2_000;
const MAX_TAPS: usize = 8;
/// Autosave waits for the transport to stop, but not longer than this after the last edit.
pub(crate) const AUTOSAVE_PLAYING_MAX: u64 = 10 * 60_000;

pub(crate) fn no_project() -> CommandError {
    crate::tx::keyed_err(
        ErrorCode::InvalidState,
        crate::text::msg("error.project.none_open"),
    )
}

/// The plugin devices whose plugin `ops` set, with their state in `project` before `ops`.
pub(crate) fn plugin_states_set_by(
    project: &Project,
    ops: &[Op],
) -> Vec<(DeviceId, Option<Base64Bytes>)> {
    let mut out: Vec<(DeviceId, Option<Base64Bytes>)> = Vec::new();
    for op in ops {
        if let Op::Update {
            update:
                EntityUpdate::Device {
                    id,
                    change: DeviceChange::Plugin(_),
                },
        } = op
            && !out.iter().any(|(d, _)| d == id)
            && let Some(DeviceKind::Plugin { plugin }) = project.devices.get(id).map(|d| &d.kind)
        {
            out.push((*id, plugin.state.clone()));
        }
    }
    out
}

pub(crate) fn store_err(e: StoreError) -> CommandError {
    let code = match &e {
        StoreError::NotFound(_) => ErrorCode::NotFound,
        StoreError::AlreadyExists(_) | StoreError::InvalidPath(_) => ErrorCode::InvalidArgument,
        StoreError::Io(_) => ErrorCode::Io,
        StoreError::Unsupported(_) => ErrorCode::Unsupported,
    };
    let key = match &e {
        StoreError::NotFound(what) => {
            crate::text::msg("error.store.not_found").arg("what", what.as_str())
        }
        StoreError::AlreadyExists(what) => {
            crate::text::msg("error.store.already_exists").arg("what", what.as_str())
        }
        StoreError::InvalidPath(what) => {
            crate::text::msg("error.store.invalid_path").arg("what", what.as_str())
        }
        StoreError::Io(error) => crate::text::msg("error.store.io").arg("error", error.as_str()),
        StoreError::Unsupported(error) => {
            crate::text::msg("error.store.unsupported").arg("error", error.as_str())
        }
    };
    crate::tx::keyed_err(code, key)
}

/// Patch changes as they go on the wire (audit #4): plugin state blobs stripped, and
/// `DeviceParam`s turned back into device upserts while a receiver predates them.
pub(crate) fn wire_changes(
    compat: &crate::PatchCompat,
    project: &Project,
    changes: &mut Vec<PatchChange>,
) {
    strip_plugin_states(changes);
    if !compat.field_patches() {
        upsert_device_params(project, changes);
    }
}

pub(crate) fn event(out: &mut dyn MessageSink, e: Event) {
    out.send(ServerMessage::Event(e));
}

/// A toast. `message` is a catalog message (`TextKey`, shown in the user's language) or, for
/// text from deeper layers, plain English.
pub(crate) fn notify(
    out: &mut dyn MessageSink,
    level: NotificationLevel,
    message: impl Into<crate::text::Msg>,
) {
    let m = message.into();
    event(
        out,
        Event::Notification {
            level,
            message: m.text,
            key: m.key,
        },
    );
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// File name for an imported copy: `media/<id>-<sanitized name>`.
pub(crate) fn media_file_name(id: MediaId, name: &str) -> String {
    let mut clean: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if clean.len() > 80 {
        let ext = extension_of(&clean)
            .map(|e| format!(".{e}"))
            .unwrap_or_default();
        clean.truncate(80usize.saturating_sub(ext.len()));
        clean.push_str(&ext);
    }
    let clean = clean.trim_start_matches('.');
    format!("{MEDIA_DIR}/{id}-{clean}")
}

/// base-115: commands that edit the shared document (refused on a listen link). Local
/// playback (transport, monitoring) stays available on the read-only copy.
fn share_edits(command: &Command, current: Option<ProjectId>) -> bool {
    use ether_core::protocol::audio_to_midi::AudioToMidiCommand;
    use ether_core::protocol::capture::CaptureCommand;
    use ether_core::protocol::drum_rack::DrumRackCommand;
    use ether_core::protocol::media::MediaCommand;
    use ether_core::protocol::mixer::MixerCommand;
    use ether_core::protocol::presets::PresetCommand;
    use ether_core::protocol::undo_history::HistoryCommand;
    match command {
        Command::Transport(_) | Command::Recording(_) => false,
        // #418: mute and solo are per site (COLLAB.md §2.1): never sent, so a listener can
        // solo and mute tracks of its read-only copy to hear them on their own. While it
        // listens to the host they are the host's (§9.8: the host applies them only with
        // remote control on).
        Command::Mixer(MixerCommand::SetMute { .. } | MixerCommand::SetSolo { .. })
        | Command::DrumRack(
            DrumRackCommand::SetPadMute { .. } | DrumRackCommand::SetPadSolo { .. },
        ) => false,
        Command::Edit(EditCommand::Batch { commands, .. }) => {
            commands.iter().any(|c| share_edits(c, current))
        }
        Command::Edit(
            EditCommand::Undo | EditCommand::Redo | EditCommand::CancelGesture { .. },
        ) => true,
        Command::TimeEdit(_)
        | Command::Freeze(_)
        | Command::MediaRef(_)
        | Command::Resolve(ether_core::protocol::resolver::ResolveCommand::Apply { .. })
        | Command::Resolve(ether_core::protocol::resolver::ResolveCommand::Replace {
            preview: false,
            ..
        })
        | Command::Media(MediaCommand::Import { .. }) => true,
        // #583: these edit the document outside `doc::apply` (one undo step each).
        Command::Preset(PresetCommand::Load { .. })
        | Command::Capture(CaptureCommand::Capture { .. })
        | Command::AudioToMidi(AudioToMidiCommand::Start { .. })
        | Command::History(HistoryCommand::JumpTo { .. }) => true,
        c => doc::is_document_command(c, current),
    }
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    pub(crate) fn dispatch(
        &mut self,
        msg: &ClientMessage,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let current = self.doc.as_ref().map(|d| d.project.id);
        let command = &msg.command;
        // `agent-api`: remember what the agent tools report (the UI's selection).
        self.agent_observe(command);
        // base-53: while listening on a peer, transport commands go to the host and
        // recording is refused (loop changes are document commands: intercept first).
        if let Some(r) = self.collab_transport_intercept(command, out) {
            return r;
        }
        if let Some(doc) = self.doc.as_ref() {
            // v0.2 (`freeze-bounce`): frozen tracks can't be edited.
            crate::freeze::check_editable(&doc.project, command)?;
        }
        // base-62: a note added by this command is authored by our session identity (a no-op
        // in the OSS build: the private overlay's session tracks it).
        self.social_note_scope(command);
        // base-115: a listen link is view only (docs/SHARING.md §2.3).
        if self.share_view_only() && share_edits(command, current) {
            return Err(crate::tx::keyed_err(
                ether_core::protocol::ErrorCode::InvalidState,
                crate::text::msg("error.share.view_only"),
            ));
        }
        // v0.3 (`templates`): `Template::Insert` loads its file and imports its samples
        // first (alone: in one gesture; in a `Batch`: scoped for the batch).
        if let Some(r) = self.template_insert_command(msg, now, out) {
            return r;
        }
        let _templates = self.template_scope(command, msg.gesture, now, out)?;
        if doc::is_document_command(command, current) {
            let label = doc::label_of(command, self.doc.as_ref().map(|d| &d.project));
            self.edit_with(&label, msg.gesture, now, out, |ctx| {
                doc::apply(ctx, command).map(drop)
            })?;
            return Ok(ReplyValue::Unit);
        }
        match command {
            Command::Edit(e) => self.edit_command(e, msg.gesture, now, out),
            Command::Project(p) => {
                if let ProjectCommand::Save { autosave } = p {
                    // Collab: replicate changed plugin states once (no-op outside a session).
                    let kind = if *autosave {
                        SaveKind::Routine
                    } else {
                        SaveKind::Full
                    };
                    self.collab_before_save(now, kind, out);
                }
                self.project_command(p, now, out)
            }
            Command::Transport(t) => {
                let r = self.transport_command(t, now, out);
                // base-62: peers see our playhead move at once.
                self.social_transport_changed();
                r
            }
            Command::Device(DeviceCommand::ListBuiltin) => Ok(ReplyValue::DeviceTypes {
                devices: ether_devices::all_descriptors(),
            }),
            Command::Device(DeviceCommand::GetDescriptor { device }) => {
                self.get_descriptor(*device)
            }
            // v0.3 (`fx-space`).
            Command::Device(DeviceCommand::ListFactoryIrs) => crate::fx_space::list_factory_irs(),
            Command::Recording(r) => self.recording_command(r, now, out),
            Command::Plugin(p) => self.plugin_command(p, msg.gesture, now, out),
            Command::Warp(WarpCommand::DetectTempo { clip }) => self.detect_tempo(*clip),
            Command::Media(m) => self.media_command(m, msg.gesture, now, out),
            Command::Engine(_) => Err(unsupported(
                "audio engine configuration is handled by the host",
            )),
            // Roadmap v2 (non-document parts; document parts go through `doc::apply`).
            Command::Export(c) => self.export_command(c, out),
            Command::MidiMap(c) => self.midi_map_command(c, out),
            // Collaboration is private (the overlay). `SetPresence` is a local selection
            // notification the agent tools read (`agent_observe`); keep it a no-op.
            Command::Collab(ether_core::protocol::collab::CollabCommand::SetPresence {
                ..
            }) => Ok(ReplyValue::Unit),
            Command::Collab(_) => Err(unsupported("collaboration is not available in this build")),
            // v0.2 (contracts-3; document parts of `Take`, `Rack`, `Modulation` and the new
            // `Track` commands go through `doc::apply`).
            Command::Freeze(c) => self.freeze_command(c, now, out),
            Command::TimeEdit(c) => self.time_edit_command(c, msg.gesture, now, out),
            Command::Preset(c) => self.preset_command(c, msg.gesture, now, out),
            Command::Browser(c) => self.browser_command(c, now, out),
            Command::Analysis(c) => self.analysis_command(c),
            Command::MediaRef(c) => self.media_ref_command(c, now, out),
            Command::Modulation(
                ether_core::protocol::racks::ModulationCommand::ListModulatorKinds,
            ) => Ok(ReplyValue::ModulatorKinds {
                kinds: ether_devices::modulators::all(),
            }),
            Command::Chat(_) => Err(unsupported("chat is not available in this build")),
            // v0.3 (contracts-4; document parts of `Expression`, `External` and `Template`
            // go through `doc::apply`).
            Command::Capture(c) => self.capture_command(c, now, out),
            Command::AudioToMidi(c) => self.audio_to_midi_command(c, now, out),
            Command::External(c) => self.external_command(c, now, out),
            Command::History(c) => self.history_command(c, now, out),
            Command::Template(c) => self.template_command(c, now, out),
            Command::Version(c) => self.version_command(c, now, out),
            Command::Keymap(c) => self.keymap_command(c, out),
            Command::Settings(ether_core::protocol::message::SettingsCommand::SetLanguage {
                language,
            }) => {
                crate::text::set_names_language(language);
                Ok(ReplyValue::Unit)
            }
            // base-115 (docs/SHARING.md) is private (the overlay).
            Command::Share(_) => Err(unsupported("sharing is not available in this build")),
            // `agent-api`: LLM tools (each edit tool call is one undo step).
            Command::Agent(c) => self.agent_command(c, now, out),
            // `import-resolver`: missing plugins and audio files (`Apply` is one undo step).
            Command::Resolve(c) => self.resolve_command(c, now, out),
            other => Err(internal(format!(
                "unhandled command {}",
                doc::label_of(other, None)
            ))),
        }
    }

    // ─── Edits ──────────────────────────────────────────────────────────────────────────

    /// Run `f` as one transaction and commit it (one undo step, merged with the previous
    /// one when `gesture` is still open). Emits the patch and applies engine effects.
    pub(crate) fn edit_with(
        &mut self,
        label: impl Into<doc::UndoLabel>,
        gesture: Option<GestureId>,
        now: u64,
        out: &mut dyn MessageSink,
        f: impl FnOnce(&mut DocCtx) -> CmdResult<()>,
    ) -> CmdResult<()> {
        let label = label.into();
        let doc = self.doc.as_mut().ok_or_else(no_project)?;
        let mut host = EngineCtx {
            bridge: &mut self.bridge,
            eng: &mut self.engine,
            media: &self.media,
        };
        let mut ctx = DocCtx {
            tx: Tx::new(&mut doc.project),
            ids: &mut self.ids,
            now,
            position: self.transport.position,
            host: &mut host,
            warnings: Vec::new(),
        };
        let result = f(&mut ctx);
        let warnings = std::mem::take(&mut ctx.warnings);
        let ops = match result {
            Ok(()) => ctx.tx.finish(),
            Err(e) => {
                ctx.tx.rollback();
                // A plugin may have been instantiated for a device that is not there.
                if self.engine.has_orphans(&doc.project) {
                    self.engine.graph_dirty = true;
                }
                return Err(e);
            }
        };
        if ops.is_empty() {
            return Ok(());
        }
        let tx = Transaction {
            label: label.text.clone(),
            ops,
            key: label.key.clone(),
        };
        doc.history.set_now_ms(now);
        let (applied, inverse) = doc
            .history
            .commit_with_inverse(&mut doc.project, tx, gesture)
            .map_err(model_err)?;
        // Collab: stamp and send (no-op outside a session).
        self.collab_local_commit(&label, &applied, &inverse);
        self.after_ops(&applied, now, out);
        for w in warnings {
            notify(out, NotificationLevel::Warning, w);
        }
        Ok(())
    }

    /// Patch + dirty flag + engine effects of ops applied to the document.
    pub(crate) fn after_ops(&mut self, applied: &[Op], now: u64, out: &mut dyn MessageSink) {
        self.after_ops_from(applied, None, now, out);
    }

    /// [`Self::after_ops`] for ops made by another site (`origin`, collab).
    pub(crate) fn after_ops_from(
        &mut self,
        applied: &[Op],
        origin: Option<OpOrigin>,
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        let Some(doc) = self.doc.as_mut() else { return };
        self.revision += 1;
        let mut changes = changes_for(&doc.project, applied);
        wire_changes(&self.config.patch_compat, &doc.project, &mut changes);
        let patch = Patch {
            revision: self.revision,
            changes,
            history: doc.history.state(),
            origin,
        };
        event(out, Event::Patch { patch });
        doc.last_edit_ms = now;
        let renamed = applied.iter().any(|op| {
            matches!(
                op,
                Op::Settings {
                    change: SettingsChange::Name(_)
                }
            )
        });
        let touches_media = applied
            .iter()
            .any(|op| matches!(op.key(), Some(EntityKey::Media(_))));
        // A device edit (params, plugin swap, preset) can change a plugin's state without
        // a `StateDirty`: the next save reads it again.
        for op in applied {
            if let Some(EntityKey::Device(d)) = op.key() {
                self.plugin_states.invalidate(d);
            }
        }
        let armed_before = self.armed.len();
        self.armed.retain(|t| doc.project.tracks.contains_key(t));
        let disarmed = self.armed.len() != armed_before;
        self.engine
            .apply_effects(&mut self.bridge, &doc.project, applied);
        if touches_media {
            self.media.sync(&mut self.bridge, Some(&doc.project));
        }
        // Dedicated engines run for other sites follow the document too (docs/COLLAB.md §16).
        self.collab_engine_ops(applied);
        self.set_dirty(true, out);
        if renamed {
            self.emit_list_changed(out);
        }
        if disarmed {
            self.emit_armed(out);
        }
    }

    pub(crate) fn set_dirty(&mut self, dirty: bool, out: &mut dyn MessageSink) {
        if let Some(doc) = self.doc.as_mut()
            && doc.dirty != dirty
        {
            doc.dirty = dirty;
            event(
                out,
                Event::Project {
                    event: ProjectEvent::DirtyChanged { dirty },
                },
            );
        }
    }

    pub(crate) fn emit_armed(&mut self, out: &mut dyn MessageSink) {
        event(
            out,
            Event::Recording {
                event: RecordingEvent::ArmChanged {
                    armed: self.armed.iter().copied().collect(),
                },
            },
        );
    }

    /// Undo or redo one step without emitting anything (`Edit::{Undo, Redo}`, and
    /// `History::JumpTo` step by step). Returns the applied ops, `None` if there was
    /// nothing to undo/redo.
    pub(crate) fn undo_redo_step(&mut self, undo: bool) -> CmdResult<Option<Vec<Op>>> {
        // Plugin states the step sets, as they were before it.
        let mut before = Vec::new();
        let applied = if self.collab_active() {
            // Collab: per-site undo (only this site's steps; peers' later changes win),
            // stamped and sent like an edit.
            self.collab_undo_redo(undo, &mut before)?
        } else {
            let doc = self.doc.as_mut().ok_or_else(no_project)?;
            let f = |p: &mut Project, ops: &[Op], _: &[Op]| {
                before = plugin_states_set_by(p, ops);
                let reverted = p.apply_all(ops)?;
                Ok((ops.to_vec(), reverted))
            };
            if undo {
                doc.history.undo_with(&mut doc.project, f)
            } else {
                doc.history.redo_with(&mut doc.project, f)
            }
            .map_err(model_err)?
        };
        self.reload_changed_plugin_states(&before);
        Ok(applied)
    }

    /// Plugins whose document state an undo/redo changed (a preset load, a peer's state
    /// it reverts...): the running instances are re-created from the document's state, as a
    /// preset load does, instead of keeping their live state. A step that leaves the blob as
    /// it was (a sandbox toggle) keeps the live state.
    pub(crate) fn reload_changed_plugin_states(
        &mut self,
        before: &[(DeviceId, Option<Base64Bytes>)],
    ) {
        let Some(doc) = self.doc.as_ref() else { return };
        let changed: Vec<DeviceId> = before
            .iter()
            .filter(|(id, old)| {
                matches!(
                    doc.project.devices.get(id).map(|d| &d.kind),
                    Some(DeviceKind::Plugin { plugin }) if plugin.state != *old
                )
            })
            .map(|(id, _)| *id)
            .collect();
        for id in changed {
            // No node (safe-mode placeholder, frozen track, failed load): nothing runs; the
            // next creation reads the document anyway.
            if self.engine.node(id).is_some() {
                self.engine.request_reload_from_doc(id);
            }
            self.collab_engine_reload_plugin(id);
        }
    }

    fn edit_command(
        &mut self,
        c: &EditCommand,
        gesture: Option<GestureId>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match c {
            EditCommand::Undo | EditCommand::Redo => {
                let undo = matches!(c, EditCommand::Undo);
                let applied = self.undo_redo_step(undo)?.ok_or_else(|| {
                    crate::tx::keyed_err(
                        ErrorCode::InvalidState,
                        if matches!(c, EditCommand::Undo) {
                            crate::text::msg("error.edit.nothing_to_undo")
                        } else {
                            crate::text::msg("error.edit.nothing_to_redo")
                        },
                    )
                })?;
                self.transport.tap_gesture = None;
                self.after_ops(&applied, now, out);
                Ok(ReplyValue::Unit)
            }
            EditCommand::EndGesture { gesture } => {
                if let Some(doc) = self.doc.as_mut() {
                    doc.history.end_gesture(*gesture);
                }
                Ok(ReplyValue::Unit)
            }
            EditCommand::CancelGesture { gesture } => {
                // #717: the gesture's step is the last one while it is open: undo it, and
                // (outside collab, where undo is per site and replicated) leave nothing to
                // redo, as an agent call rolls back.
                if !self
                    .doc
                    .as_ref()
                    .is_some_and(|d| d.history.is_open_gesture(*gesture))
                {
                    return Ok(ReplyValue::Unit);
                }
                self.transport.tap_gesture = None;
                if let Some(applied) = self.undo_redo_step(true)? {
                    if !self.collab_active()
                        && let Some(doc) = self.doc.as_mut()
                    {
                        doc.history.discard_redo();
                    }
                    self.after_ops(&applied, now, out);
                }
                Ok(ReplyValue::Unit)
            }
            EditCommand::Batch {
                label,
                commands,
                key,
            } => {
                let current = self.doc.as_ref().map(|d| d.project.id);
                for sub in commands {
                    if !doc::is_document_command(sub, current)
                        || matches!(sub, Command::Transport(_))
                    {
                        return Err(invalid(format!(
                            "{} is not allowed in a batch",
                            doc::label_of(sub, None)
                        )));
                    }
                }
                self.edit_with(
                    doc::UndoLabel::batch(label, key.as_ref()),
                    gesture,
                    now,
                    out,
                    |ctx| {
                        for sub in commands {
                            doc::apply(ctx, sub)?;
                        }
                        Ok(())
                    },
                )?;
                Ok(ReplyValue::Unit)
            }
        }
    }

    // ─── Transport ──────────────────────────────────────────────────────────────────────

    fn engine_transport(&mut self, control: TransportControl) -> CmdResult<()> {
        self.bridge.transport(control).map_err(bridge_err)
    }

    fn transport_command(
        &mut self,
        c: &TransportCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match c {
            TransportCommand::Play => self.play()?,
            TransportCommand::Stop => self.transport_stop(now, out)?,
            TransportCommand::TogglePlay => {
                if self.transport.playing {
                    self.transport_stop(now, out)?
                } else {
                    self.play()?
                }
            }
            TransportCommand::Locate { position } => {
                if !(position.0.is_finite() && position.0 >= 0.0) {
                    return Err(invalid("position must be >= 0"));
                }
                self.engine_transport(TransportControl::Locate {
                    position: *position,
                })?;
                self.transport.position = *position;
                if !self.transport.playing {
                    self.transport.start_position = *position;
                }
            }
            TransportCommand::TapTempo => self.tap_tempo(now, out)?,
            other => return Err(internal(format!("unhandled transport command {other:?}"))),
        }
        Ok(ReplyValue::Unit)
    }

    fn play(&mut self) -> CmdResult<()> {
        self.require_engine()?;
        self.engine_transport(TransportControl::Play)?;
        self.transport.playing = true;
        Ok(())
    }

    pub(crate) fn stop(&mut self) -> CmdResult<()> {
        if self.transport.playing {
            self.engine_transport(TransportControl::Stop)?;
            self.transport.playing = false;
        } else {
            let start = self.transport.start_position;
            self.engine_transport(TransportControl::Locate { position: start })?;
            self.transport.position = start;
        }
        Ok(())
    }

    fn tap_tempo(&mut self, now: u64, out: &mut dyn MessageSink) -> CmdResult<()> {
        let t = &mut self.transport;
        if t.taps
            .last()
            .is_some_and(|last| now.saturating_sub(*last) > TAP_RESET_MS || now < *last)
        {
            t.taps.clear();
            t.tap_gesture = None;
        }
        t.taps.push(now);
        if t.taps.len() > MAX_TAPS {
            t.taps.remove(0);
        }
        if t.taps.len() < 2 {
            return Ok(());
        }
        let span = (t.taps[t.taps.len() - 1] - t.taps[0]) as f64;
        let avg = span / (t.taps.len() - 1) as f64;
        if avg <= 0.0 {
            return Ok(());
        }
        let bpm = (60_000.0 / avg).clamp(doc::MIN_BPM, doc::MAX_BPM);
        let gesture = match t.tap_gesture {
            Some(g) => g,
            None => {
                let g = self.new_gesture();
                self.transport.tap_gesture = Some(g);
                g
            }
        };
        let cmd = TransportCommand::SetTempo { bpm };
        self.edit_with(
            crate::doc::undo("undo.transport.tap_tempo"),
            Some(gesture),
            now,
            out,
            |ctx| doc::apply(ctx, &Command::Transport(cmd)).map(drop),
        )
    }

    pub(crate) fn new_gesture(&mut self) -> GestureId {
        let g = GestureId(self.next_gesture);
        self.next_gesture = self.next_gesture.wrapping_add(1).max(0x8000_0000);
        g
    }

    pub(crate) fn transport_state(&self) -> Option<TransportState> {
        if let Some(s) = self.collab_listen_transport_state() {
            return Some(s);
        }
        let doc = self.doc.as_ref()?;
        let p = &doc.project;
        let map = p.tempo_map();
        let at = self.transport.position;
        Some(TransportState {
            playing: self.transport.playing,
            recording: self.transport.recording,
            loop_enabled: p.settings.loop_enabled,
            loop_region: p.settings.loop_region,
            bpm: map.bpm_at(at),
            time_signature: map.signature_at(at),
            metronome: p.settings.metronome,
            start_position: self.transport.start_position,
        })
    }

    pub(crate) fn emit_transport_if_changed(&mut self, out: &mut dyn MessageSink) {
        let state = self.transport_state();
        if let Some(state) = state
            && Some(&state) != self.last_transport.as_ref()
        {
            self.last_transport = Some(state.clone());
            event(out, Event::Transport { state });
        }
    }

    // ─── Devices / plugins ──────────────────────────────────────────────────────────────

    fn get_descriptor(&mut self, device: DeviceId) -> CmdResult<ReplyValue> {
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        let d = doc
            .project
            .devices
            .get(&device)
            .cloned()
            .ok_or_else(|| not_found(format!("device {device}")))?;
        let mut host = EngineCtx {
            bridge: &mut self.bridge,
            eng: &mut self.engine,
            media: &self.media,
        };
        host.descriptor(d.id, &d.kind)
            .map(|descriptor| ReplyValue::Descriptor { descriptor })
            .ok_or_else(|| not_found(format!("descriptor of device {device} (plugin not loaded)")))
    }

    fn plugin_device(&self, device: DeviceId) -> CmdResult<PluginInstance> {
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        match doc.project.devices.get(&device).map(|d| &d.kind) {
            Some(DeviceKind::Plugin { plugin }) => Ok(plugin.clone()),
            Some(_) => Err(invalid(format!("device {device} is not a plugin"))),
            None => Err(not_found(format!("device {device}"))),
        }
    }

    fn plugin_command(
        &mut self,
        c: &PluginCommand,
        gesture: Option<GestureId>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match c {
            PluginCommand::Rescan { .. }
            | PluginCommand::List
            | PluginCommand::ListFolders
            | PluginCommand::AddFolder { .. }
            | PluginCommand::RemoveFolder { .. }
            | PluginCommand::SetIncludeDefaults { .. }
            | PluginCommand::OpenEditor { .. }
            | PluginCommand::CloseEditor { .. }
            | PluginCommand::SetEditorKeyCapture { .. }
            | PluginCommand::ListScanFailures
            | PluginCommand::RetryScan { .. }
            | PluginCommand::ParamTexts { .. } => Err(unsupported(
                "plugin scanning, folders, editors and param texts are handled by the host",
            )),
            PluginCommand::SetSandboxed { device, sandboxed } => {
                if self.plugin_device(*device)?.sandboxed == *sandboxed {
                    return Ok(ReplyValue::Unit);
                }
                let device = *device;
                let sandboxed = *sandboxed;
                // The live state goes to the document outside the history: the toggle and
                // its undo both carry it, so neither changes the state blob (an undo that
                // did would re-create the instance from it, `reload_changed_plugin_states`).
                self.keep_running_state(device);
                let mut plugin = self.plugin_device(device)?;
                let label = if sandboxed {
                    "sandbox plugin"
                } else {
                    "unsandbox plugin"
                };
                self.edit_with(label, gesture, now, out, |ctx| {
                    plugin.sandboxed = sandboxed;
                    ctx.set_device(device, DeviceChange::Plugin(plugin))
                })?;
                Ok(ReplyValue::Unit)
            }
            PluginCommand::Reload { device } => {
                self.plugin_device(*device)?;
                self.engine.request_recreate(*device);
                self.publish_if_due(now, true, out);
                Ok(ReplyValue::Unit)
            }
        }
    }

    /// One plugin notification other than `ParamEdited` (those are coalesced per poll by
    /// `plugin_notifications`, `plugins/edits.rs`).
    pub(crate) fn plugin_notification(
        &mut self,
        device: DeviceId,
        n: PluginNotification,
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        let exists = self
            .doc
            .as_ref()
            .is_some_and(|d| d.project.devices.contains_key(&device));
        match n {
            PluginNotification::GestureBegin { param } => {
                let g = self.new_gesture();
                self.plugin_gestures.insert((device, param), g);
            }
            PluginNotification::GestureEnd { param } => {
                if let Some(g) = self.plugin_gestures.remove(&(device, param))
                    && let Some(doc) = self.doc.as_mut()
                {
                    doc.history.end_gesture(g);
                }
            }
            PluginNotification::LatencyChanged { samples } => {
                self.engine.graph_dirty = true;
                event(
                    out,
                    Event::Plugin {
                        event: PluginEvent::LatencyChanged { device, samples },
                    },
                );
            }
            PluginNotification::RestartRequested if exists => {
                match self.plugins.allow_restart(device, now) {
                    (true, _) => self.engine.request_recreate(device),
                    (false, first_refusal) => {
                        if first_refusal {
                            let name = self
                                .doc
                                .as_ref()
                                .and_then(|d| d.project.devices.get(&device))
                                .map(|d| d.name.clone())
                                .unwrap_or_default();
                            notify(
                                out,
                                NotificationLevel::Warning,
                                crate::text::msg("notice.plugin.reload_loop").arg("device", name),
                            );
                        }
                    }
                }
            }
            PluginNotification::ParamsChanged => {
                let desc = self.bridge.descriptor(device);
                self.engine.set_plugin_descriptor(device, desc);
                self.engine.graph_dirty = true;
            }
            // Not a document edit (the UI's quick autosave would save the whole project
            // for it, and plugins report this often): the state is re-read and written by
            // the next save, or by the slow timer in `tick` (`crate::saving`).
            PluginNotification::StateDirty if exists => {
                self.plugin_states.state_changed(device, now);
            }
            PluginNotification::EditorClosed => event(
                out,
                Event::Plugin {
                    event: PluginEvent::EditorClosed { device },
                },
            ),
            PluginNotification::EditorKey(key) => event(
                out,
                Event::Plugin {
                    event: PluginEvent::EditorKey { device, key },
                },
            ),
            PluginNotification::Crashed { message } => event(
                out,
                Event::Plugin {
                    event: PluginEvent::Crashed { device, message },
                },
            ),
            _ => {}
        }
    }

    // ─── Warp ───────────────────────────────────────────────────────────────────────────

    /// `WarpCommand::DetectTempo` (the BPM stub lives in `crate::warp`).
    fn detect_tempo(&self, clip: ClipId) -> CmdResult<ReplyValue> {
        crate::warp::detect_tempo(&self.doc.as_ref().ok_or_else(no_project)?.project, clip)
    }

    // ─── Media ──────────────────────────────────────────────────────────────────────────

    pub(crate) fn locations(&self) -> Vec<BrowseRoot> {
        let mut roots = self.library.roots();
        if self.doc.is_some() {
            roots.push(BrowseRoot {
                location: BrowseLocation::ProjectMedia,
                name: "project media".into(),
            });
        }
        roots
    }

    fn media_command(
        &mut self,
        c: &MediaCommand,
        gesture: Option<GestureId>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match c {
            MediaCommand::Import { id, source } => self.import(*id, source, gesture, now, out),
            MediaCommand::GetPeaks { request } => {
                if let Some(peaks) = self.media.peaks(request.media) {
                    return Ok(ReplyValue::Peaks {
                        peaks: peaks.query(request),
                    });
                }
                let exists = self
                    .doc
                    .as_ref()
                    .is_some_and(|d| d.project.media.contains_key(&request.media));
                if !exists {
                    Err(not_found(format!("media {}", request.media)))
                } else if self.media.is_pending(request.media) {
                    Err(invalid_state(
                        "peaks are not ready yet (wait for PeaksReady)",
                    ))
                } else {
                    Err(cmd_err(ErrorCode::Decode, "the media could not be decoded"))
                }
            }
            MediaCommand::ListLocations => Ok(ReplyValue::Locations {
                locations: self.locations(),
            }),
            MediaCommand::ListDirectory { location, path } => {
                check_relative_path(path).map_err(store_err)?;
                let listing = match location {
                    BrowseLocation::Library { id } => {
                        self.library.list_dir(id, path).map_err(store_err)?
                    }
                    BrowseLocation::ProjectMedia => {
                        let pid = self.doc.as_ref().ok_or_else(no_project)?.project.id;
                        let rel = if path.is_empty() {
                            MEDIA_DIR.to_string()
                        } else {
                            format!("{MEDIA_DIR}/{path}")
                        };
                        let prefix = format!("{MEDIA_DIR}/");
                        let mut listing = match self.store.list_dir(pid, &rel) {
                            Ok(l) => l,
                            Err(StoreError::NotFound(_)) if path.is_empty() => DirectoryListing {
                                location: BrowseLocation::ProjectMedia,
                                path: String::new(),
                                entries: Vec::new(),
                            },
                            Err(e) => return Err(store_err(e)),
                        };
                        for e in &mut listing.entries {
                            if let Some(rest) = e.path.strip_prefix(&prefix) {
                                e.path = rest.to_string();
                            }
                        }
                        listing.location = BrowseLocation::ProjectMedia;
                        listing.path = path.clone();
                        listing
                    }
                };
                // `base-145`: the user's file filter (browser/filter.rs).
                let mut listing = listing;
                let filter = self.browser_filter();
                // Hidden files never show (the disk stores skip them already).
                listing.entries.retain(|e| {
                    if e.name.starts_with('.') {
                        false
                    } else if e.kind == FileKind::Directory {
                        filter.allows_dir(&e.name)
                    } else {
                        filter.allows_file(&e.name)
                    }
                });
                Ok(ReplyValue::Directory { listing })
            }
            MediaCommand::Preview { .. } | MediaCommand::StopPreview => {
                self.preview_command(c, out)
            }
            MediaCommand::BeginUpload { .. }
            | MediaCommand::UploadChunk { .. }
            | MediaCommand::CancelUpload { .. } => self.upload_command(c, out),
            MediaCommand::ReadMidiFile { source } => self.read_midi_file(source),
            MediaCommand::WriteMidiFile { request } => self.write_midi_file(request),
        }
    }

    fn import(
        &mut self,
        id: MediaId,
        source: &MediaSource,
        gesture: Option<GestureId>,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        let doc = self.doc.as_ref().ok_or_else(no_project)?;
        if let Some(m) = doc.project.media.get(&id) {
            return Ok(ReplyValue::Media { media: m.clone() });
        }
        let pid = doc.project.id;
        // v0.2 (`media-references`, CONTRACTS.md §12.9): library files and OS paths are
        // referenced in place where the host can (`External`, nothing copied); the rest is
        // copied into the project's `media/`.
        let mut location = MediaLocation::Project;
        let (bytes, name, existing_file) = match source {
            MediaSource::Location { location: at, path } => {
                check_relative_path(path).map_err(store_err)?;
                if path.is_empty() {
                    return Err(invalid("path must name a file"));
                }
                match at {
                    BrowseLocation::Library { id: root } => {
                        let bytes = self.library.read(root, path).map_err(store_err)?;
                        if let Some(path) = self.library.external_path(root, path) {
                            location = MediaLocation::External { path };
                        }
                        (bytes, basename(path).to_string(), None)
                    }
                    BrowseLocation::ProjectMedia => {
                        let rel = format!("{MEDIA_DIR}/{path}");
                        let bytes = self.store.read(pid, &rel).map_err(store_err)?;
                        (bytes, basename(path).to_string(), Some(rel))
                    }
                }
            }
            MediaSource::Project { media } => {
                let m = doc
                    .project
                    .media
                    .get(media)
                    .cloned()
                    .ok_or_else(|| not_found(format!("media {media}")))?;
                let bytes =
                    crate::media::read_media_bytes(&mut self.store, &mut self.library, pid, &m)
                        .map_err(store_err)?;
                // The same file: referenced where the original is.
                location = m.location.clone();
                (bytes, m.name, Some(m.file))
            }
            MediaSource::Upload { upload } => {
                let (bytes, name) =
                    crate::upload::take_upload(&mut self.uploads, &mut self.store, upload)?;
                (bytes, name, None)
            }
            // v0.2 (`file-import`): an OS file of the engine machine (desktop), referenced in
            // place (`media-references`).
            MediaSource::Path { path } => {
                let (bytes, name) = crate::file_import::read_path(&mut self.library, path)?;
                location = MediaLocation::External { path: path.clone() };
                (bytes, name, None)
            }
        };
        let hash = content_hash(&bytes);
        let chained = is_chained_ogg(&bytes);
        let bytes: Arc<[u8]> = bytes.into();
        let mut decoder = IncrementalDecoder::new(bytes.clone(), extension_of(&name))
            .map_err(|e| cmd_err(ErrorCode::Decode, e.to_string()))?;
        if decoder.sample_rate == 0 || decoder.channels == 0 {
            // Rate/channels come with the first packet for some formats: decode just that.
            decoder
                .step(1)
                .map_err(|e| cmd_err(ErrorCode::Decode, e.to_string()))?;
        }
        // Never decode the whole file here: an unknown length (e.g. VBR MP3 without a
        // Xing header) is `frames: 0` until the background decode fills it in.
        if decoder.sample_rate == 0 || decoder.channels == 0 {
            return Err(cmd_err(ErrorCode::Decode, "no decodable audio"));
        }
        let external = matches!(location, MediaLocation::External { .. });
        let file = match existing_file {
            Some(f) => f,
            // Referenced in place: `file` is where "collect all" (or a collaborator) puts it.
            None if external => media_file_name(id, &name),
            None => match doc
                .project
                .media
                .values()
                .filter(|m| m.hash.as_deref() == Some(hash.as_str()))
                .map(|m| m.file.clone())
                .find(|f| {
                    self.store
                        .read(pid, f)
                        .is_ok_and(|existing| existing[..] == bytes[..])
                }) {
                // Same content already in the project (verified byte for byte): share it.
                Some(f) => f,
                None => {
                    let file = media_file_name(id, &name);
                    self.store.write(pid, &file, &bytes).map_err(store_err)?;
                    file
                }
            },
        };
        let media = MediaRef {
            location,
            id,
            name: name.clone(),
            file,
            sample_rate: decoder.sample_rate,
            channels: decoder.channels as u16,
            frames: decoder.n_frames.unwrap_or(0),
            hash: Some(hash),
        };
        let entity = Entity::Media(media.clone());
        // In the caller's gesture, so "import + create clip" (a browser drop) is one undo step.
        self.edit_with(
            crate::doc::undo("undo.media.import"),
            gesture,
            now,
            out,
            |ctx| ctx.tx.insert(entity),
        )?;
        if chained {
            notify(out, NotificationLevel::Warning, chained_ogg_warning(&name));
        }
        self.media.queue_import(media.clone(), decoder, chained);
        event(
            out,
            Event::Media {
                event: MediaEvent::ImportProgress {
                    media: id,
                    progress: 0.0,
                },
            },
        );
        Ok(ReplyValue::Media { media })
    }

    /// Metadata fix-up (not an edit, not undoable): the length of media imported with an
    /// unknown length, learned by the background decode. Sent as a patch.
    fn fill_media_length(&mut self, media: MediaId, frames: u64, out: &mut dyn MessageSink) {
        let Some(doc) = self.doc.as_mut() else { return };
        let Some(m) = doc.project.media.get_mut(&media) else {
            return;
        };
        if m.frames != 0 {
            return;
        }
        m.frames = frames;
        let entity = Entity::Media(m.clone());
        self.revision += 1;
        event(
            out,
            Event::Patch {
                patch: Patch {
                    revision: self.revision,
                    changes: vec![PatchChange::Upsert { entity }],
                    history: doc.history.state(),
                    origin: None,
                },
            },
        );
    }

    // ─── Tick ───────────────────────────────────────────────────────────────────────────

    pub(crate) fn tick_impl(&mut self, now: u64, out: &mut dyn MessageSink) {
        // Engine outputs.
        let mut outputs = std::mem::take(&mut self.outputs);
        self.bridge.poll(&mut outputs);
        if let Some(ph) = outputs.playhead {
            // Adopt the engine's play state on its own transitions only (a command may not
            // have reached the audio thread yet).
            if self.transport.last_engine_playing != Some(ph.playing) {
                if self.transport.last_engine_playing.is_some() {
                    self.transport.playing = ph.playing;
                }
                self.transport.last_engine_playing = Some(ph.playing);
            }
            self.transport.position = ph.position;
            self.transport.seconds = ph.seconds;
            let frame = PlayheadUpdate {
                position: ph.position,
                seconds: Seconds(ph.seconds),
                playing: ph.playing,
                bpm: ph.bpm,
            };
            if self.transport.last_frame != Some(frame) {
                self.transport.last_frame = Some(frame);
                out.send(ServerMessage::Playhead(PlayheadFrame { transport: frame }));
            }
        }
        if !outputs.meters.is_empty() {
            out.send(ServerMessage::Meters(MeterFrame {
                tracks: outputs.meters.clone(),
                cpu_load: outputs.cpu_load,
            }));
        }
        self.outputs = outputs;

        // Plugin notifications.
        let mut notes = Vec::new();
        self.bridge.poll_plugins(&mut notes);
        self.plugin_notifications(notes, now, out);
        self.plugins_tick(now, out);
        // Roadmap v2 hooks.
        self.preview_tick(now, out);
        self.stream_tick(now, out);
        self.midi_learn_tick(now, out);
        self.export_tick(now, out);
        self.recording_tick(now, out);
        self.collab_tick(now, out);
        // base-115: sharing (hub, signaling, joiner links) after the collab session.
        self.share_tick(now, out);
        // v0.2 hooks.
        self.analysis_tick(out);
        self.freeze_tick(now, out);
        self.browser_tick(now, out);
        self.media_refs_tick(now, out);
        self.resolve_tick(now, out);
        // v0.3 hooks.
        self.capture_tick(now, out);
        self.audio_to_midi_tick(now, out);
        self.history_tick(now, out);
        self.versions_tick(now, out);

        // Media jobs (and media decoded again for the bridge to build reverb IRs, #529).
        if let Some(doc) = self.doc.as_ref() {
            self.media.queue_ir_decodes(&mut self.bridge, &doc.project);
        }
        if let Some(pid) = self.doc.as_ref().map(|d| d.project.id)
            && self.media.has_jobs()
        {
            let mut events = Vec::new();
            let loaded = self.media.step(
                pid,
                &mut self.store,
                &mut self.library,
                &mut self.bridge,
                self.config.engine_sample_rate,
                self.config.media_frames_per_tick,
                &mut events,
            );
            for e in events {
                event(out, e);
            }
            for (media, frames) in self.media.take_frame_fixups() {
                self.fill_media_length(media, frames, out);
            }
            if !loaded.is_empty() {
                self.engine.graph_dirty = true;
                // Samplers are built with their sample: rebuild those that use new media.
                let samplers: Vec<DeviceId> = self
                    .doc
                    .as_ref()
                    .map(|d| {
                        d.project
                            .devices
                            .values()
                            .filter(|dev| {
                                matches!(&dev.kind, DeviceKind::Builtin {
                                    device: BuiltinDevice::Sampler { sample: Some(m), .. }
                                } if loaded.contains(m))
                                    || crate::multisampler::uses_media(&dev.kind, &loaded)
                                    || crate::fx_space::uses_media(&dev.kind, &loaded)
                            })
                            .map(|dev| dev.id)
                            .collect()
                    })
                    .unwrap_or_default();
                for s in samplers {
                    self.engine.request_recreate(s);
                }
            }
        }

        // Autosave.
        self.autosave_tick(now, out);

        self.publish_if_due(now, true, out);
        self.emit_transport_if_changed(out);
    }

    /// Routine autosaves: a dirty document after `autosave_after_ms` without edits (a
    /// backstop: the UI saves sooner), and plugin state changes alone on their slower
    /// timer (`saving::PluginStates::save_due`). Also reports failed background writes.
    fn autosave_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        if let Some((id, e)) = self.store.take_save_error() {
            self.background_save_failed(id, e, now, out);
        }
        let Some(after) = self.config.autosave_after_ms else {
            return;
        };
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        // Not while playing (a document change up to `AUTOSAVE_PLAYING_MAX`): a save reads
        // plugin states on the main thread, which on a big project stalls the playhead, the
        // meters, every queued command and (on desktop) the UI thread.
        let idle = now.saturating_sub(doc.last_edit_ms);
        let playing = self.transport.playing;
        let doc_due =
            doc.dirty && idle >= after && (!playing || idle >= after.max(AUTOSAVE_PLAYING_MAX));
        let states_due = !doc_due && !playing && self.plugin_states.save_due(now);
        if !doc_due && !states_due {
            return;
        }
        if states_due && !doc.dirty {
            // Plugins often report changes that aren't (state identical to the saved one,
            // e.g. right after loading): no write then.
            let doc = self.doc.take().expect("checked");
            self.plugin_states
                .retain(|d| doc.project.devices.contains_key(&d));
            self.read_plugin_states(&doc.project, SaveKind::Routine);
            self.doc = Some(doc);
            if !self.plugin_states.has_unwritten() {
                self.plugin_states.nothing_unsaved();
                return;
            }
        }
        self.collab_before_save(now, SaveKind::Routine, out);
        if let Err(e) = self.save_current_with(SaveKind::Routine, out) {
            notify(
                out,
                NotificationLevel::Error,
                crate::text::msg("notice.project.autosave_failed")
                    .arg("error", crate::text::error_arg(&e)),
            );
            // Don't retry every tick.
            if let Some(doc) = self.doc.as_mut() {
                doc.last_edit_ms = now;
            }
            self.plugin_states.postpone(now);
        }
    }

    /// Publish the graph if dirty (from `handle`: only if the last publish is old enough).
    pub(crate) fn publish_if_due(&mut self, now: u64, force: bool, out: &mut dyn MessageSink) {
        if !self.engine.graph_dirty || self.engine.off {
            self.engine.graph_dirty = false;
            return;
        }
        if !force
            && let Some(last) = self.engine.last_publish_ms
            && now.saturating_sub(last) < self.config.publish_interval_ms
        {
            return;
        }
        let project = self.doc.as_ref().map(|d| &d.project);
        let host = &self.host;
        for (level, message) in
            self.engine
                .publish(&mut self.bridge, project, &self.armed, now, &|| {
                    host.now_ms()
                })
        {
            notify(out, level, message);
        }
    }

    // ─── Collaboration / sharing no-ops (private overlay) ─────────────────────────────
    //
    // The collab session, share links, presence and pinned notes live in the private
    // collaboration overlay. In this build every hook the shared code reaches is a no-op so
    // ordinary single-user projects behave exactly as before: no session is ever active, so
    // the guarded branches below (undo/redo, relink, version restore, ...) take their
    // single-user path.

    /// `Some` intercepts a command the session handles (transport to a listened host); in
    /// this build nothing is intercepted.
    fn collab_transport_intercept(
        &mut self,
        _command: &Command,
        _out: &mut dyn MessageSink,
    ) -> Option<CmdResult<ReplyValue>> {
        None
    }

    /// Records the note author a chat/note command carries (no session: nothing to record).
    fn social_note_scope(&mut self, _command: &Command) {}

    /// Replicate plugin states changed since the last save (no session: nothing to send).
    pub(crate) fn collab_before_save(
        &mut self,
        _now: u64,
        _kind: SaveKind,
        _out: &mut dyn MessageSink,
    ) {
    }

    /// Tell peers the transport moved (no session: nobody to tell).
    fn social_transport_changed(&mut self) {}

    /// Stamp and send a locally committed transaction (no session: nothing to send).
    fn collab_local_commit(&mut self, _label: &doc::UndoLabel, _ops: &[Op], _inverse: &[Op]) {}

    /// Mirror applied ops to dedicated engines (no dedicated engines in this build).
    fn collab_engine_ops(&mut self, _ops: &[Op]) {}

    /// Re-create a replicated plugin state on a dedicated engine (none exist here).
    fn collab_engine_reload_plugin(&mut self, _device: DeviceId) {}

    /// Per-site undo/redo inside a session; unreachable (a session is never active).
    fn collab_undo_redo(
        &mut self,
        _undo: bool,
        _plugin_states_before: &mut Vec<(DeviceId, Option<Base64Bytes>)>,
    ) -> CmdResult<Option<Vec<Op>>> {
        Ok(None)
    }

    /// The host's transport while listening on a peer (`None`: not listening).
    fn collab_listen_transport_state(&self) -> Option<TransportState> {
        None
    }

    fn collab_tick(&mut self, _now: u64, _out: &mut dyn MessageSink) {}

    /// React to a project load in dedicated engines (none exist here).
    pub(crate) fn collab_engine_project_changed(&mut self, _out: &mut dyn MessageSink) {}

    /// `true` while joined through a view-only link (never in this build).
    pub(crate) fn share_view_only(&self) -> bool {
        false
    }

    fn share_tick(&mut self, _now: u64, _out: &mut dyn MessageSink) {}

    /// Resume/pause sharing of another project (nothing is shared here).
    pub(crate) fn share_project_loaded(&mut self, _pid: ProjectId, _out: &mut dyn MessageSink) {}

    /// `true` while a collaboration session is active (never in this build).
    pub(crate) fn collab_active(&self) -> bool {
        false
    }

    // ─── Local engine status (kept protocol; the remote-engine choice is private) ──────

    /// Whether the local engine is off. Always `false` here: only the private overlay's
    /// remote-engine choice turns it off (the native host then closes the audio device).
    pub fn local_engine_off(&self) -> bool {
        self.engine.off
    }

    /// Refuse what needs a running engine (play, record). The local engine always runs in
    /// this build, so this only guards against the overlay's off state.
    pub(crate) fn require_engine(&self) -> CmdResult<()> {
        if self.engine.off {
            return Err(internal("the audio engine is off"));
        }
        Ok(())
    }

    /// Emit `CollabEvent::Engine` when it changed (`force`: always, on `Get`). The status is
    /// part of the kept protocol even without the remote-engine choice: report the local
    /// engine (always Mine here, unless the overlay turned it off).
    pub(crate) fn engine_emit_status(&mut self, force: bool, out: &mut dyn MessageSink) {
        use ether_core::protocol::collab::{CollabEvent, LocalEngine, LocalEngineStatus};
        let status = LocalEngineStatus {
            local: if self.engine.off {
                LocalEngine::None
            } else {
                LocalEngine::Mine
            },
            error: self.engine.switch_error.clone(),
            // Dedicated remote engines are private; none exist here.
            dedicated: None,
        };
        let last = self.engine.emitted_status.clone().unwrap_or_default();
        if !force && last == status {
            return;
        }
        self.engine.emitted_status = Some(status.clone());
        event(
            out,
            Event::Collab {
                event: CollabEvent::Engine { status },
            },
        );
    }

    /// A project is about to load: clear any stale local-engine-off state so it opens with
    /// the local engine on (only the private overlay ever turns it off).
    pub(crate) fn engine_on_project_load(&mut self, id: ProjectId, _now: u64) {
        if self.engine.off && self.engine.off_project != Some(id) {
            self.engine.off = false;
            self.engine.off_project = None;
            self.engine.switch_error = None;
            self.media.set_engine_off(&mut self.bridge, false);
        }
        if self.engine.off_next == Some(id) {
            self.engine.off_next = None;
            self.engine.off_project = Some(id);
        }
    }
}
