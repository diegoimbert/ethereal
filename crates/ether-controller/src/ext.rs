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

use ether_core::protocol::model::ProjectId;
use ether_core::protocol::{Command, ReplyValue};

use crate::resolver::ImportHints;

use crate::store::{Library, ProjectStore};
use crate::tx::CmdResult;
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

/// A controller extension. Every hook has a no-op default.
pub trait ControllerExt<B, H, S, L>
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
        ctl: &mut EtherController<B, H, S, L>,
        command: &Command,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> Option<CmdResult<ReplyValue>> {
        let _ = (ctl, command, now, out);
        None
    }

    /// Every controller tick (`now`: the host's milliseconds), before the app's own work.
    fn tick(&mut self, ctl: &mut EtherController<B, H, S, L>, now: u64, out: &mut dyn MessageSink) {
        let _ = (ctl, now, out);
    }
}

/// The extensions of one controller.
pub(crate) struct Extensions<B, H, S, L>(Vec<Box<dyn ControllerExt<B, H, S, L>>>);

impl<B, H, S, L> Default for Extensions<B, H, S, L> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<B, H, S, L> EtherController<B, H, S, L>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// Add an extension (before the host starts; they run in the order added).
    pub fn add_extension(&mut self, ext: impl ControllerExt<B, H, S, L> + 'static) {
        self.extensions.0.push(Box::new(ext));
    }

    /// Run `f` over the extensions, each with the controller (they are out of it meanwhile).
    fn with_extensions<R>(
        &mut self,
        mut f: impl FnMut(&mut dyn ControllerExt<B, H, S, L>, &mut Self) -> Option<R>,
    ) -> Option<R> {
        if self.extensions.0.is_empty() {
            return None;
        }
        let mut exts = std::mem::take(&mut self.extensions.0);
        let mut result = None;
        for ext in exts.iter_mut() {
            result = f(ext.as_mut(), self);
            if result.is_some() {
                break;
            }
        }
        // Extensions added while they ran (none expected) go after the existing ones.
        exts.append(&mut self.extensions.0);
        self.extensions.0 = exts;
        result
    }

    pub(crate) fn ext_command(
        &mut self,
        command: &Command,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> Option<CmdResult<ReplyValue>> {
        self.with_extensions(|ext, ctl| ext.command(ctl, command, now, out))
    }

    pub(crate) fn ext_tick(&mut self, now: u64, out: &mut dyn MessageSink) {
        self.with_extensions::<()>(|ext, ctl| {
            ext.tick(ctl, now, out);
            None
        });
    }
}

/// What an extension reads and changes of the controller (beyond its `pub` fields).
impl<B, H, S, L> EtherController<B, H, S, L>
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
