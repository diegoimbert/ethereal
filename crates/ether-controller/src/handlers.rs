//! Message dispatch and everything that is not project lifecycle: edits (commit → patch →
//! engine effects), transport, recording, plugins, media, and the tick.

use std::sync::Arc;

use ether_core::TransportControl;
use ether_core::plugin::PluginNotification;
use ether_core::protocol::devices::DeviceCommand;
use ether_core::protocol::media::{
    BrowseLocation, BrowseRoot, DirectoryListing, MediaCommand, MediaEvent, MediaSource,
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
use crate::store::{Library, ProjectStore, StoreError, check_relative_path};
use crate::tx::{
    CmdResult, Tx, cmd_err, internal, invalid, invalid_state, model_err, not_found, unsupported,
};
use crate::{EngineBridge, EtherController, HostServices, MessageSink, content_hash};

/// Taps further apart than this start a new tap-tempo sequence.
const TAP_RESET_MS: u64 = 2_000;
const MAX_TAPS: usize = 8;

pub(crate) fn no_project() -> CommandError {
    invalid_state("no project is open")
}

pub(crate) fn store_err(e: StoreError) -> CommandError {
    let code = match &e {
        StoreError::NotFound(_) => ErrorCode::NotFound,
        StoreError::AlreadyExists(_) | StoreError::InvalidPath(_) => ErrorCode::InvalidArgument,
        StoreError::Io(_) => ErrorCode::Io,
        StoreError::Unsupported(_) => ErrorCode::Unsupported,
    };
    cmd_err(code, e.to_string())
}

pub(crate) fn event(out: &mut dyn MessageSink, e: Event) {
    out.send(ServerMessage::Event(e));
}

pub(crate) fn notify(
    out: &mut dyn MessageSink,
    level: NotificationLevel,
    message: impl Into<String>,
) {
    event(
        out,
        Event::Notification {
            level,
            message: message.into(),
        },
    );
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// File name for an imported copy: `media/<id>-<sanitized name>`.
fn media_file_name(id: MediaId, name: &str) -> String {
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
        if let Some(doc) = self.doc.as_ref() {
            // v0.2 (`freeze-bounce`): frozen tracks can't be edited.
            crate::freeze::check_editable(&doc.project, command)?;
        }
        if doc::is_document_command(command, current) {
            let label = doc::label_of(command);
            self.edit_with(&label, msg.gesture, now, out, |ctx| {
                doc::apply(ctx, command).map(drop)
            })?;
            return Ok(ReplyValue::Unit);
        }
        match command {
            Command::Edit(e) => self.edit_command(e, msg.gesture, now, out),
            Command::Project(p) => {
                if matches!(p, ProjectCommand::Save) {
                    // Collab: replicate changed plugin states once (no-op outside a session).
                    self.collab_before_save(now, out);
                }
                self.project_command(p, now, out)
            }
            Command::Transport(t) => self.transport_command(t, now, out),
            Command::Device(DeviceCommand::ListBuiltin) => Ok(ReplyValue::DeviceTypes {
                devices: ether_devices::all_descriptors(),
            }),
            Command::Device(DeviceCommand::GetDescriptor { device }) => {
                self.get_descriptor(*device)
            }
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
            Command::Collab(c) => self.collab_command(c, out),
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
            other => Err(internal(format!(
                "unhandled command {}",
                doc::label_of(other)
            ))),
        }
    }

    // ─── Edits ──────────────────────────────────────────────────────────────────────────

    /// Run `f` as one transaction and commit it (one undo step, merged with the previous
    /// one when `gesture` is still open). Emits the patch and applies engine effects.
    pub(crate) fn edit_with(
        &mut self,
        label: &str,
        gesture: Option<GestureId>,
        now: u64,
        out: &mut dyn MessageSink,
        f: impl FnOnce(&mut DocCtx) -> CmdResult<()>,
    ) -> CmdResult<()> {
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
            label: label.to_string(),
            ops,
        };
        let (applied, inverse) = doc
            .history
            .commit_with_inverse(&mut doc.project, tx, gesture)
            .map_err(model_err)?;
        // Collab: stamp and send (no-op outside a session).
        self.collab_local_commit(label, &applied, &inverse);
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
        let patch = Patch {
            revision: self.revision,
            changes: changes_for(&doc.project, applied),
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
        let armed_before = self.armed.len();
        self.armed.retain(|t| doc.project.tracks.contains_key(t));
        let disarmed = self.armed.len() != armed_before;
        self.engine
            .apply_effects(&mut self.bridge, &doc.project, applied);
        if touches_media {
            self.media.sync(&mut self.bridge, Some(&doc.project));
        }
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
                let applied = if self.collab_active() {
                    // Collab: per-site undo (only this site's steps; peers' later changes
                    // win), stamped and sent like an edit.
                    self.collab_undo_redo(undo)?
                } else {
                    let doc = self.doc.as_mut().ok_or_else(no_project)?;
                    if undo {
                        doc.history.undo(&mut doc.project)
                    } else {
                        doc.history.redo(&mut doc.project)
                    }
                    .map_err(model_err)?
                }
                .ok_or_else(|| {
                    invalid_state(if matches!(c, EditCommand::Undo) {
                        "nothing to undo"
                    } else {
                        "nothing to redo"
                    })
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
            EditCommand::Batch { label, commands } => {
                let current = self.doc.as_ref().map(|d| d.project.id);
                for sub in commands {
                    if !doc::is_document_command(sub, current)
                        || matches!(sub, Command::Transport(_))
                    {
                        return Err(invalid(format!(
                            "{} is not allowed in a batch",
                            doc::label_of(sub)
                        )));
                    }
                }
                self.edit_with(label, gesture, now, out, |ctx| {
                    for sub in commands {
                        doc::apply(ctx, sub)?;
                    }
                    Ok(())
                })?;
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
        // base-53: while listening on a peer, transport commands go to the host.
        if let Some(r) = self.collab_transport_intercept(c, out) {
            return r;
        }
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
        self.edit_with("Tap Tempo", Some(gesture), now, out, |ctx| {
            doc::apply(ctx, &Command::Transport(cmd)).map(drop)
        })
    }

    pub(crate) fn new_gesture(&mut self) -> GestureId {
        let g = GestureId(self.next_gesture);
        self.next_gesture = self.next_gesture.wrapping_add(1).max(0x8000_0000);
        g
    }

    pub(crate) fn transport_state(&self) -> Option<TransportState> {
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
            PluginCommand::Rescan
            | PluginCommand::List
            | PluginCommand::OpenEditor { .. }
            | PluginCommand::CloseEditor { .. } => Err(unsupported(
                "plugin scanning and editors are handled by the host",
            )),
            PluginCommand::SetSandboxed { device, sandboxed } => {
                let plugin = self.plugin_device(*device)?;
                if plugin.sandboxed == *sandboxed {
                    return Ok(ReplyValue::Unit);
                }
                let device = *device;
                let sandboxed = *sandboxed;
                self.edit_with("Set Sandboxed", gesture, now, out, |ctx| {
                    let mut plugin = plugin;
                    plugin.sandboxed = sandboxed;
                    if let Some(state) = ctx.host.plugin_state(device) {
                        plugin.state = Some(state);
                    }
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

    fn plugin_notification(
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
            PluginNotification::ParamEdited { param, value } if exists && value.is_finite() => {
                let gesture = self.plugin_gestures.get(&(device, param)).copied();
                self.engine.echo_from = Some(device);
                let _ = self.edit_with("Set Param", gesture, now, out, |ctx| {
                    ctx.set_device(
                        device,
                        DeviceChange::Param {
                            param,
                            value: Some(value),
                        },
                    )
                });
                self.engine.echo_from = None;
            }
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
            PluginNotification::RestartRequested if exists => self.engine.request_recreate(device),
            PluginNotification::ParamsChanged => {
                let desc = self.bridge.descriptor(device);
                self.engine.set_plugin_descriptor(device, desc);
                self.engine.graph_dirty = true;
            }
            PluginNotification::StateDirty if exists => {
                if let Some(doc) = self.doc.as_mut() {
                    doc.last_edit_ms = now;
                }
                self.set_dirty(true, out);
            }
            PluginNotification::EditorClosed => event(
                out,
                Event::Plugin {
                    event: PluginEvent::EditorClosed { device },
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

    fn locations(&self) -> Vec<BrowseRoot> {
        let mut roots = self.library.roots();
        if self.doc.is_some() {
            roots.push(BrowseRoot {
                location: BrowseLocation::ProjectMedia,
                name: "Project media".into(),
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
                Ok(ReplyValue::Directory { listing })
            }
            MediaCommand::Preview { .. } | MediaCommand::StopPreview => {
                self.preview_command(c, out)
            }
            MediaCommand::BeginUpload { .. }
            | MediaCommand::UploadChunk { .. }
            | MediaCommand::CancelUpload { .. } => self.upload_command(c, out),
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
        let (bytes, name, existing_file) = match source {
            MediaSource::Location { location, path } => {
                check_relative_path(path).map_err(store_err)?;
                if path.is_empty() {
                    return Err(invalid("path must name a file"));
                }
                match location {
                    BrowseLocation::Library { id: root } => {
                        let bytes = self.library.read(root, path).map_err(store_err)?;
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
                let bytes = self.store.read(pid, &m.file).map_err(store_err)?;
                (bytes, m.name, Some(m.file))
            }
            MediaSource::Upload { upload } => {
                let (bytes, name) =
                    crate::upload::take_upload(&mut self.uploads, &mut self.store, upload)?;
                (bytes, name, None)
            }
            // v0.2 (`file-import`): an OS file of the engine machine (desktop).
            MediaSource::Path { path } => {
                let (bytes, name) = crate::file_import::read_path(&mut self.library, path)?;
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
        let file = match existing_file {
            Some(f) => f,
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
            location: Default::default(),
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
        self.edit_with("Import", gesture, now, out, |ctx| ctx.tx.insert(entity))?;
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
        for (device, n) in notes {
            self.plugin_notification(device, n, now, out);
        }
        self.plugins_tick(now, out);
        // Roadmap v2 hooks.
        self.preview_tick(now, out);
        self.midi_learn_tick(now, out);
        self.export_tick(now, out);
        self.recording_tick(now, out);
        self.collab_tick(now, out);
        // v0.2 hooks.
        self.analysis_tick(out);
        self.freeze_tick(now, out);
        self.browser_tick(now, out);
        self.media_refs_tick(now, out);

        // Media jobs.
        if let Some(pid) = self.doc.as_ref().map(|d| d.project.id)
            && self.media.has_jobs()
        {
            let mut events = Vec::new();
            let loaded = self.media.step(
                pid,
                &mut self.store,
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
        if let (Some(after), Some(doc)) = (self.config.autosave_after_ms, self.doc.as_ref())
            && doc.dirty
            && now.saturating_sub(doc.last_edit_ms) >= after
            && let Err(e) = {
                self.collab_before_save(now, out);
                self.save_current(out)
            }
        {
            notify(
                out,
                NotificationLevel::Error,
                format!("autosave failed: {}", e.message),
            );
            // Don't retry every tick.
            if let Some(doc) = self.doc.as_mut() {
                doc.last_edit_ms = now;
            }
        }

        self.publish_if_due(now, true, out);
        self.emit_transport_if_changed(out);
    }

    /// Publish the graph if dirty (from `handle`: only if the last publish is old enough).
    pub(crate) fn publish_if_due(&mut self, now: u64, force: bool, out: &mut dyn MessageSink) {
        if !self.engine.graph_dirty {
            return;
        }
        if !force
            && let Some(last) = self.engine.last_publish_ms
            && now.saturating_sub(last) < self.config.publish_interval_ms
        {
            return;
        }
        let project = self.doc.as_ref().map(|d| &d.project);
        for (level, message) in self
            .engine
            .publish(&mut self.bridge, project, &self.armed, now)
        {
            notify(out, level, message);
        }
    }
}
