//! Controller extensions (docs/EXTENSIONS.md "The controller"): features built outside this
//! repository plug into the controller here.
//!
//! An extension is a [`ControllerExt`] added with [`EtherController::add_extension`] before
//! the host starts. It keeps its own state and gets the controller with every call:
//!
//! - [`ControllerExt::command`] sees each command before the app does, and may answer it (an
//!   extension's own domain, `Command::Ext`, or an app command the app leaves to extensions,
//!   such as `Project::Import`, which replies `Unsupported` without one).
//! - [`ControllerExt::tick`] runs with every controller tick (background jobs).
//!
//! What an extension may use of the controller is the public API: the `pub` fields and
//! methods of [`EtherController`] and the helpers in [`api`].

use std::collections::BTreeSet;

use ether_core::plugin::PluginNotification;
use ether_core::protocol::freeze::FreezeCommand;
use ether_core::protocol::model::Op;
use ether_core::protocol::model::{Base64Bytes, DeviceId, ProjectId, SiteId};
use ether_core::protocol::transport::TransportState;
use ether_core::protocol::{Command, CommandError, ReplyValue};

pub use crate::doc::{DocCtx, ExtDocCommands, UndoLabel, set_ext_doc_commands};
pub use crate::freeze::{Finish, RenderUpdate};
pub use crate::saving::SaveKind;

use crate::resolver::ImportHints;

use crate::store::{Library, ProjectStore, RenderSidecar};
use crate::tx::CmdResult;
use crate::{BridgeError, EngineBridge, EtherController, HostServices, MessageSink};

/// A controller extension. Every hook has a no-op default.
pub trait ControllerExt<B, H, S, L, X = ()>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// A command, before the app handles it. `Some`: the extension handled it (its result
    /// is the reply; the app never sees the command). Called for every command, in the
    /// order the extensions were added; the first `Some` wins.
    fn command(
        &mut self,
        ctl: &mut EtherController<B, H, S, L, X>,
        command: &Command,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> Option<CmdResult<ReplyValue>> {
        let _ = (ctl, command, now, out);
        None
    }

    /// Every controller tick (`now`: the host's milliseconds), before the app's own work.
    fn tick(
        &mut self,
        ctl: &mut EtherController<B, H, S, L, X>,
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        let _ = (ctl, now, out);
    }

    /// A command the app is about to handle (after [`Self::command`] left it to the app).
    fn before_command(&mut self, ctl: &mut EtherController<B, H, S, L, X>, command: &Command) {
        let _ = (ctl, command);
    }

    /// A document edit is about to be committed: `Some` refuses it (the error is the reply).
    fn refuse_edit(
        &self,
        ctl: &EtherController<B, H, S, L, X>,
        ops: &[Op],
    ) -> Option<CommandError> {
        let _ = (ctl, ops);
        None
    }

    /// Why saves are held back (`Some`: autosaves skip, saves fail with it, switching away
    /// drops the unsaved work instead of saving it).
    fn save_refused(&self, ctl: &EtherController<B, H, S, L, X>) -> Option<CommandError> {
        let _ = ctl;
        None
    }

    /// The plugins' notifications of this tick, before the app handles them (an extension
    /// may take some out).
    fn plugin_notes(
        &mut self,
        ctl: &mut EtherController<B, H, S, L, X>,
        notes: &mut Vec<(DeviceId, PluginNotification)>,
        now: u64,
    ) {
        let _ = (ctl, notes, now);
    }

    /// A local edit was committed: `ops` applied (and their `inverse`), one undo step.
    fn committed(
        &mut self,
        ctl: &mut EtherController<B, H, S, L, X>,
        label: &UndoLabel,
        ops: &[Op],
        inverse: &[Op],
    ) {
        let _ = (ctl, label, ops, inverse);
    }

    /// Ops were applied to the open document (any edit, undo or redo).
    fn applied(&mut self, ctl: &mut EtherController<B, H, S, L, X>, ops: &[Op]) {
        let _ = (ctl, ops);
    }

    /// Undo or redo, when the extension keeps the history (`Some`: the applied ops, `None`
    /// inside it: nothing to do; `plugin_states` gets the states the step set, as they were
    /// before it). Called only while [`Self::session_active`].
    fn undo_redo(
        &mut self,
        ctl: &mut EtherController<B, H, S, L, X>,
        undo: bool,
        plugin_states: &mut Vec<(DeviceId, Option<Base64Bytes>)>,
    ) -> Option<CmdResult<Option<Vec<Op>>>> {
        let _ = (ctl, undo, plugin_states);
        None
    }

    /// `true` while the extension shares the document with others (undo/redo, relinks and
    /// version restores take their shared paths).
    fn session_active(&self, ctl: &EtherController<B, H, S, L, X>) -> bool {
        let _ = ctl;
        false
    }

    /// `true` while the document may not be edited here (a read-only view).
    fn view_only(&self, ctl: &EtherController<B, H, S, L, X>) -> bool {
        let _ = ctl;
        false
    }

    /// The transport to report instead of the local one (`Some`: another engine plays).
    fn transport_state(&self, ctl: &EtherController<B, H, S, L, X>) -> Option<TransportState> {
        let _ = ctl;
        None
    }

    /// The local transport moved (play, stop, locate, loop).
    fn transport_changed(&mut self, ctl: &mut EtherController<B, H, S, L, X>) {
        let _ = ctl;
    }

    /// A save is about to happen.
    fn before_save(
        &mut self,
        ctl: &mut EtherController<B, H, S, L, X>,
        now: u64,
        kind: SaveKind,
        out: &mut dyn MessageSink,
    ) {
        let _ = (ctl, now, kind, out);
    }

    /// A file to write right after the document by this save (e.g. a journal naming the
    /// document it goes with), from the document's JSON.
    fn save_sidecar(
        &mut self,
        ctl: &mut EtherController<B, H, S, L, X>,
        project: ProjectId,
    ) -> Option<RenderSidecar> {
        let _ = (ctl, project);
        None
    }

    /// The open project was saved (`with_sidecar`: with a [`Self::save_sidecar`] file).
    fn saved(
        &mut self,
        ctl: &mut EtherController<B, H, S, L, X>,
        project: ProjectId,
        with_sidecar: bool,
    ) {
        let _ = (ctl, project, with_sidecar);
    }

    /// A stored project was copied (`SaveAs`, `Duplicate`) as `copy`: a private project.
    fn project_copied(&mut self, ctl: &mut EtherController<B, H, S, L, X>, copy: ProjectId) {
        let _ = (ctl, copy);
    }

    /// A plugin's instance was re-created from the document.
    fn plugin_reloaded(&mut self, ctl: &mut EtherController<B, H, S, L, X>, device: DeviceId) {
        let _ = (ctl, device);
    }

    /// A project was loaded (`project_loaded`), and the engine changed with it
    /// (`engine_changed`).
    fn project_loaded(
        &mut self,
        ctl: &mut EtherController<B, H, S, L, X>,
        project: ProjectId,
        out: &mut dyn MessageSink,
    ) {
        let _ = (ctl, project, out);
    }

    /// The engine's nodes were rebuilt for a newly loaded project.
    fn engine_changed(
        &mut self,
        ctl: &mut EtherController<B, H, S, L, X>,
        out: &mut dyn MessageSink,
    ) {
        let _ = (ctl, out);
    }

    /// A render is about to start: `Some` runs it elsewhere (the extension reports its
    /// progress as `Event::Freeze` and calls `finish_render` when it is done; its reply is
    /// the command's). `finish`: what the render's end does here.
    fn offload_render(
        &mut self,
        ctl: &mut EtherController<B, H, S, L, X>,
        command: &FreezeCommand,
        job: &str,
        finish: &Finish,
        out: &mut dyn MessageSink,
    ) -> Option<ReplyValue> {
        let _ = (ctl, command, job, finish, out);
        None
    }

    /// `FreezeCommand::Cancel` of a render this extension runs elsewhere (`true`: it was).
    fn cancel_render(&mut self, ctl: &mut EtherController<B, H, S, L, X>, job: &str) -> bool {
        let _ = (ctl, job);
        false
    }

    /// `true` while a render this extension runs elsewhere is under way (no second one).
    fn render_busy(&self, ctl: &EtherController<B, H, S, L, X>) -> bool {
        let _ = ctl;
        false
    }

    /// Whether the site a render runs for (`freeze_for`) is still there (`false` drops it).
    fn render_requester_present(&self, ctl: &EtherController<B, H, S, L, X>, site: SiteId) -> bool {
        let _ = (ctl, site);
        true
    }

    /// How a render for another site (`freeze_for`) goes.
    fn render_update(
        &mut self,
        ctl: &mut EtherController<B, H, S, L, X>,
        site: SiteId,
        job: &str,
        update: RenderUpdate,
    ) {
        let _ = (ctl, site, job, update);
    }

    /// Why play and record are refused while the local engine is off (`None`: the app's
    /// message).
    fn engine_off_error(&self, ctl: &EtherController<B, H, S, L, X>) -> Option<CommandError> {
        let _ = ctl;
        None
    }

    /// Report the engine's status (`force`: even unchanged).
    fn engine_status(
        &mut self,
        ctl: &mut EtherController<B, H, S, L, X>,
        force: bool,
        out: &mut dyn MessageSink,
    ) {
        let _ = (ctl, force, out);
    }
}

/// An extension, boxed.
type BoxedExt<B, H, S, L, X> = Box<dyn ControllerExt<B, H, S, L, X>>;

/// One added extension.
enum Slot<B, H, S, L, X> {
    /// With its own state: out of the controller while one of its hooks runs (a hook it
    /// triggers itself doesn't reach it).
    Stateful(Option<BoxedExt<B, H, S, L, X>>),
    /// Stateless (its state, if any, is the controller's `ext`): a fresh one for every
    /// call, so the hooks it triggers reach it too.
    Stateless(fn() -> BoxedExt<B, H, S, L, X>),
}

/// The extensions of one controller.
pub struct Extensions<B, H, S, L, X>(Vec<Slot<B, H, S, L, X>>);

impl<B, H, S, L, X> Default for Extensions<B, H, S, L, X> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<B, H, S, L, X> EtherController<B, H, S, L, X>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// Add an extension (before the host starts; they run in the order added).
    pub fn add_extension(&mut self, ext: impl ControllerExt<B, H, S, L, X> + 'static) {
        self.extensions.0.push(Slot::Stateful(Some(Box::new(ext))));
    }

    /// Add a stateless extension (its state lives in the controller's `ext`): its hooks
    /// may trigger hooks of its own (an edit it makes reaches its `committed`).
    pub fn add_stateless_extension<E>(&mut self)
    where
        E: ControllerExt<B, H, S, L, X> + Default + 'static,
    {
        self.extensions
            .0
            .push(Slot::Stateless(|| Box::new(E::default())));
    }

    /// Run `f` over the extensions, each with the controller, until one answers `Some`.
    pub fn with_extensions<R>(
        &mut self,
        mut f: impl FnMut(&mut dyn ControllerExt<B, H, S, L, X>, &mut Self) -> Option<R>,
    ) -> Option<R> {
        let mut i = 0;
        while i < self.extensions.0.len() {
            let result = match &mut self.extensions.0[i] {
                Slot::Stateful(slot) => match slot.take() {
                    Some(mut ext) => {
                        let result = f(ext.as_mut(), self);
                        if let Slot::Stateful(slot) = &mut self.extensions.0[i] {
                            *slot = Some(ext);
                        }
                        result
                    }
                    // Running further up the stack.
                    None => None,
                },
                Slot::Stateless(make) => {
                    let mut ext = make();
                    f(ext.as_mut(), self)
                }
            };
            if result.is_some() {
                return result;
            }
            i += 1;
        }
        None
    }

    pub fn ext_command(
        &mut self,
        command: &Command,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> Option<CmdResult<ReplyValue>> {
        self.with_extensions(|ext, ctl| ext.command(ctl, command, now, out))
    }

    /// Every extension in turn (none returns early).
    pub fn ext_each(
        &mut self,
        mut f: impl FnMut(&mut dyn ControllerExt<B, H, S, L, X>, &mut Self),
    ) {
        self.with_extensions::<()>(|ext, ctl| {
            f(ext, ctl);
            None
        });
    }

    /// The first extension's `Some` answer to a question that doesn't change anything.
    pub fn ext_ask<R>(
        &self,
        f: impl Fn(&dyn ControllerExt<B, H, S, L, X>, &Self) -> Option<R>,
    ) -> Option<R> {
        self.extensions.0.iter().find_map(|slot| match slot {
            Slot::Stateful(Some(e)) => f(e.as_ref(), self),
            Slot::Stateful(None) => None,
            Slot::Stateless(make) => f(make().as_ref(), self),
        })
    }

    pub fn ext_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        self.with_extensions::<()>(|ext, ctl| {
            ext.tick(ctl, now, out);
            None
        });
    }
}

/// What an extension reads and changes of the controller (beyond its `pub` fields).
impl<B, H, S, L, X> EtherController<B, H, S, L, X>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// The app version written into saved files.
    pub fn app_version(&self) -> &str {
        &self.config.app_version
    }

    /// The bytes and file name of a completed upload (`BundleSource::Upload`); consumed.
    pub fn take_upload(&mut self, upload: &str) -> CmdResult<(Vec<u8>, String)> {
        crate::upload::take_upload(&mut self.uploads, &mut self.store, upload)
    }

    /// Keep `bytes` as a download for `Export::ReadChunk` (dropped with `project`'s others
    /// when it closes).
    pub fn add_download(&mut self, token: String, project: ProjectId, bytes: Vec<u8>) {
        self.export.add_download(token, project, bytes);
    }

    /// What an import knows about the project it just opened (the resolver uses them).
    pub fn set_import_hints(&mut self, hints: ImportHints) {
        self.resolver.set_import_hints(hints);
    }

    /// Load a new state into plugin `device`'s running instance: in place when the host can
    /// (`Ok(true)`: its editor stays open, its params are mirrored into the document again),
    /// else the instance is re-created from the document (`Ok(false)`; `Err`: the live load
    /// failed, re-created too).
    pub fn load_plugin_state(
        &mut self,
        device: DeviceId,
        state: &Base64Bytes,
    ) -> Result<bool, BridgeError> {
        let loaded = self
            .engine
            .apply_plugin_state(&mut self.bridge, device, state);
        if matches!(loaded, Ok(true)) {
            self.plugins.remirror(device);
        }
        loaded
    }

    /// The open project's files still on their way (an extension writes them later): their
    /// media are neither read nor reported missing until [`Self::reload_media_file`].
    pub fn set_incoming_media(&mut self, files: BTreeSet<String>) {
        if let Some(doc) = self.doc.as_ref() {
            self.media.set_incoming(&doc.project, files);
        }
    }

    /// No file of the open project is on its way any more.
    pub fn clear_incoming_media(&mut self) {
        self.media.clear_incoming();
    }

    /// The project file `file` was written or replaced: reload the media stored there.
    pub fn reload_media_file(&mut self, file: &str) {
        if let Some(doc) = self.doc.as_ref() {
            self.media.reload_file(&mut self.bridge, &doc.project, file);
        }
    }
}

/// The controller's helpers an extension may use.
pub mod api {
    pub use crate::bundle::pack;
    pub use crate::handlers::{event, notify, store_err};
    pub use crate::media::{IncrementalDecoder, extension_of};
    pub use crate::project::{bundle_file_name, import_err};
    pub use crate::resolver::ImportHints;
    pub use crate::text::msg;
    pub use crate::tx::{
        CmdResult, cmd_err, internal, invalid, invalid_state, keyed_err, not_found, unsupported,
    };
}
