//! `Media::Reveal`: show a file or folder in the OS file manager (desktop).
//!
//! The UI names what to show (the projects folder, a project, a media source) and the
//! controller resolves it to an engine-machine path through the store and the library, never
//! by joining paths itself (a project folder's name is the store's business). The host then
//! opens the file manager ([`HostServices::reveal`]); hosts whose UI isn't on the engine
//! machine (web, remote) say no first ([`HostServices::reveals_files`]), before anything is
//! resolved.

use ether_core::protocol::ReplyValue;
use ether_core::protocol::media::{BrowseLocation, MediaSource, RevealTarget};
use ether_core::protocol::model::file::MEDIA_DIR;
use ether_core::protocol::model::{MediaLocation, Project};

use crate::handlers::{no_project, store_err};
use crate::store::{Library, ProjectStore, check_relative_path};
use crate::tx::{CmdResult, invalid, not_found, unsupported};
use crate::{EngineBridge, EtherController, HostServices};

/// Where a [`RevealTarget`] is on the engine machine: the path, and whether to open it (a
/// folder shown as itself) rather than select it in its folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub path: String,
    pub open: bool,
}

/// Resolve `target` (see the module docs). `doc`: the open project; `missing`: whether one
/// of its media is unresolved (its external file isn't there: the project copy is shown).
pub fn resolve<S: ProjectStore, L: Library>(
    target: &RevealTarget,
    store: &S,
    library: &L,
    doc: Option<&Project>,
    missing: impl Fn(ether_core::protocol::model::MediaId) -> bool,
) -> CmdResult<Resolved> {
    let none = || unsupported("this host has no folder for that");
    let select = |path: String| Resolved { path, open: false };
    match target {
        RevealTarget::Projects => store
            .projects_os_path()
            .map(|path| Resolved { path, open: true })
            .ok_or_else(none),
        RevealTarget::Project { id } => store.project_os_path(*id, "").map(select).ok_or_else(none),
        RevealTarget::ProjectFile { path } => {
            if path.is_empty() {
                return Err(invalid("no file named"));
            }
            check_relative_path(path).map_err(store_err)?;
            let pid = doc.ok_or_else(no_project)?.id;
            store
                .project_os_path(pid, path)
                .map(select)
                .ok_or_else(none)
        }
        RevealTarget::Media { source } => match source {
            MediaSource::Location { location, path } => {
                check_relative_path(path).map_err(store_err)?;
                match location {
                    BrowseLocation::Library { id } => library.external_path(id, path),
                    BrowseLocation::ProjectMedia => {
                        let pid = doc.ok_or_else(no_project)?.id;
                        let rel = if path.is_empty() {
                            MEDIA_DIR.to_string()
                        } else {
                            format!("{MEDIA_DIR}/{path}")
                        };
                        store.project_os_path(pid, &rel)
                    }
                }
                .map(select)
                .ok_or_else(none)
            }
            MediaSource::Project { media } => {
                let project = doc.ok_or_else(no_project)?;
                let m = project
                    .media
                    .get(media)
                    .ok_or_else(|| not_found(format!("media {media}")))?;
                match &m.location {
                    MediaLocation::External { path } if !missing(*media) => {
                        Ok(select(path.clone()))
                    }
                    _ => {
                        check_relative_path(&m.file).map_err(store_err)?;
                        store
                            .project_os_path(project.id, &m.file)
                            .map(select)
                            .ok_or_else(none)
                    }
                }
            }
            MediaSource::Path { path } => {
                crate::file_import::check_absolute(path)?;
                Ok(select(path.clone()))
            }
            MediaSource::Upload { .. } => Err(invalid("an upload is not a file to show")),
        },
    }
}

impl<B, H, S, L, X> EtherController<B, H, S, L, X>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// `Media::Reveal`.
    pub fn reveal(&mut self, target: &RevealTarget) -> CmdResult<ReplyValue> {
        if !self.host.reveals_files() {
            return Err(unsupported("showing files is not available on this host"));
        }
        let resolved = resolve(
            target,
            &self.store,
            &self.library,
            self.doc.as_ref().map(|d| &d.project),
            |m| self.media.is_missing(m),
        )?;
        self.host
            .reveal(&resolved.path, resolved.open)
            .map_err(store_err)?;
        Ok(ReplyValue::Unit)
    }
}
