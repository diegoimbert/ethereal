//! Format-agnostic plugin loading (native only).
//!
//! Every plugin format (CLAP in `ether-clap`, VST3 in `ether-vst3`, AU in `ether-au`)
//! implements [`PluginFormatHost`]. Everything that loads plugins (the out-of-process
//! scanner, the sandbox helper, the native host) goes through a [`Formats`] registry built
//! from those implementations, so adding a format never touches the loaders. The contract a
//! format must honor is in `docs/PLUGIN-FORMATS.md`.
//!
//! The format crates depend on this crate (not the other way round), so a registry is built
//! by whoever links the formats:
//!
//! ```ignore
//! let formats = Formats::new(vec![
//!     Arc::new(ether_clap::ClapFormat),
//!     Arc::new(ether_vst3::Vst3Format),
//!     Arc::new(ether_au::AuFormat),
//! ]);
//! ```
//!
//! Also here: the file-system bundle walker shared by bundle-based formats ([`bundles`])
//! and the host side of the out-of-process scanner ([`ScanRunner`]).
#![cfg(not(target_arch = "wasm32"))]

pub mod bundles;
mod cache;
mod scan;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ether_core::plugin::{PluginController, PluginError};
use ether_core::protocol::model::PluginFormat;
use ether_core::protocol::plugins::{PluginDescriptor, ScanRequest, ScanResponse};

pub use cache::{CachedScan, Fingerprint, SCAN_CACHE_FILE, SCANNER_VERSION, ScanCache, fingerprint};
pub use scan::{SCAN_JOBS_ENV, SCANNER_BIN, ScanReport, ScanRunner};

/// One plugin format's loader. Implementations are stateless or internally synchronized
/// (`Send + Sync`): one registry is shared by every thread of a process.
///
/// # Threads and processes
/// - [`PluginFormatHost::default_search_paths`], [`PluginFormatHost::discover`] and
///   [`PluginFormatHost::discover_registry`] never run plugin code: safe in the host process.
/// - [`PluginFormatHost::scan`] loads plugin code. It runs ONLY inside
///   `ether-plugin-scanner` (one target per process; the host drives it with
///   [`ScanRunner`]), so a plugin that crashes or hangs while loading only loses itself.
/// - [`PluginFormatHost::instantiate`] runs on the plugin **main thread**: the thread that
///   will own the returned controller and call its `poll` (the native host's `MainThread`
///   executor, which is the process main thread on macOS; or the sandbox helper's main
///   thread). The controller is `!Send` and never leaves that thread.
pub trait PluginFormatHost: Send + Sync {
    fn format(&self) -> PluginFormat;

    /// Platform default search roots, in priority order (may not exist). Registry-based
    /// formats (AU) return their conventional install folders for information only.
    fn default_search_paths(&self) -> Vec<PathBuf>;

    /// Scan targets of this format under `paths` (file-system walk, no loading). Sorted,
    /// deduplicated. A target path passed directly is returned as-is if it is one.
    fn discover(&self, paths: &[PathBuf]) -> Vec<PathBuf>;

    /// Scan targets that don't come from a search path (AU: the system component registry,
    /// one target per component id). Listed without running plugin code. Default: none.
    fn discover_registry(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    /// Whether `target` is a scan target of this format (by its shape, e.g. the `.clap`
    /// extension; no file-system access needed). Used to infer the format of a
    /// [`ScanRequest`] without one.
    fn claims(&self, target: &Path) -> bool;

    /// Load `target` and describe its plugins (`PluginDescriptor.format == self.format()`,
    /// ids per the `PluginFormat` convention). Runs plugin code: scanner process only.
    fn scan(&self, target: &Path) -> Result<Vec<PluginDescriptor>, PluginError>;

    /// Instantiate `plugin_id` (from `PluginDescriptor.id`) loaded from `path`
    /// (`PluginDescriptor.path`), on the plugin main thread. The returned controller is
    /// inactive; the caller restores state and activates it.
    fn instantiate(
        &self,
        path: &Path,
        plugin_id: &str,
    ) -> Result<Box<dyn PluginController>, PluginError>;
}

/// A unit of out-of-process scanning: one bundle (CLAP/VST3) or one component (AU).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScanTarget {
    pub format: PluginFormat,
    pub path: PathBuf,
}

/// `PluginError::Unsupported` for a format feature that isn't available (yet).
pub fn unsupported(format: PluginFormat, what: &str) -> PluginError {
    PluginError::Unsupported(format!("{what} is not supported for {format} plugins yet"))
}

/// The plugin formats a process can load. Cheap to clone.
#[derive(Clone, Default)]
pub struct Formats {
    hosts: Vec<Arc<dyn PluginFormatHost>>,
}

impl std::fmt::Debug for Formats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.hosts.iter().map(|h| h.format()))
            .finish()
    }
}

impl Formats {
    /// A registry of `hosts`. If two hosts claim the same format the first one wins.
    pub fn new(hosts: Vec<Arc<dyn PluginFormatHost>>) -> Self {
        Self { hosts }
    }

    /// Registered formats, in registration order.
    pub fn formats(&self) -> Vec<PluginFormat> {
        self.hosts.iter().map(|h| h.format()).collect()
    }

    pub fn get(&self, format: PluginFormat) -> Option<&dyn PluginFormatHost> {
        self.hosts
            .iter()
            .find(|h| h.format() == format)
            .map(|h| &**h)
    }

    /// Like [`Formats::get`], but a missing format is an `Unsupported` error.
    pub fn host(&self, format: PluginFormat) -> Result<&dyn PluginFormatHost, PluginError> {
        self.get(format).ok_or_else(|| {
            PluginError::Unsupported(format!("{format} plugins are not supported by this build"))
        })
    }

    /// The format of a scan target, from its shape (first registered format claiming it).
    pub fn infer(&self, target: &Path) -> Option<PluginFormat> {
        self.hosts
            .iter()
            .find(|h| h.claims(target))
            .map(|h| h.format())
    }

    /// Every format's default search paths, tagged with the format.
    pub fn default_search_paths(&self) -> Vec<(PluginFormat, PathBuf)> {
        self.hosts
            .iter()
            .flat_map(|h| {
                let f = h.format();
                h.default_search_paths().into_iter().map(move |p| (f, p))
            })
            .collect()
    }

    /// Scan targets of every format, without loading anything.
    /// - `None`: each format's default search paths plus its registry (AU components).
    /// - `Some(paths)`: only what lies under `paths` (no registry).
    pub fn discover(&self, paths: Option<&[PathBuf]>) -> Vec<ScanTarget> {
        let mut out = Vec::new();
        for h in &self.hosts {
            let format = h.format();
            let found = match paths {
                Some(paths) => h.discover(paths),
                None => {
                    let mut v = h.discover(&h.default_search_paths());
                    v.extend(h.discover_registry());
                    v
                }
            };
            out.extend(found.into_iter().map(|path| ScanTarget { format, path }));
        }
        out.sort();
        out.dedup();
        out
    }

    /// Scan targets under the OS default folders (plus the registry, which lists the
    /// components installed in those folders) if `include_defaults`, and under each user
    /// folder for the formats its filter allows (`None` = every format). Folders and
    /// targets reached twice (overlapping folders, symlinks) are kept once, by canonical
    /// path; the first spelling wins.
    pub fn discover_folders(
        &self,
        include_defaults: bool,
        folders: &[(PathBuf, Option<PluginFormat>)],
    ) -> Vec<ScanTarget> {
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let mut out = Vec::new();
        for h in &self.hosts {
            let format = h.format();
            let mut roots: Vec<PathBuf> = Vec::new();
            let mut seen_roots = std::collections::HashSet::new();
            let defaults = if include_defaults {
                h.default_search_paths()
            } else {
                Vec::new()
            };
            let user = folders
                .iter()
                .filter(|(_, f)| f.is_none_or(|f| f == format))
                .map(|(p, _)| p.clone());
            for root in defaults.into_iter().chain(user) {
                if seen_roots.insert(canon(&root)) {
                    roots.push(root);
                }
            }
            let mut found = h.discover(&roots);
            if include_defaults {
                found.extend(h.discover_registry());
            }
            out.extend(found.into_iter().map(|path| ScanTarget { format, path }));
        }
        out.sort();
        out.dedup();
        let mut seen = std::collections::HashSet::new();
        out.retain(|t| seen.insert((t.format, canon(&t.path))));
        out
    }

    /// Scan one target in THIS process (scanner process only). `format: None` = inferred.
    pub fn scan(
        &self,
        format: Option<PluginFormat>,
        target: &Path,
    ) -> Result<Vec<PluginDescriptor>, PluginError> {
        let format = match format.or_else(|| self.infer(target)) {
            Some(f) => f,
            None => {
                return Err(PluginError::Unsupported(format!(
                    "not a plugin bundle of a supported format: {}",
                    target.display()
                )));
            }
        };
        self.host(format)?.scan(target)
    }

    /// Answer one scanner-process request (`ether-plugin-scanner` protocol mode).
    pub fn handle_scan_request(&self, request: &ScanRequest) -> ScanResponse {
        match self.scan(request.format, Path::new(&request.bundle_path)) {
            Ok(plugins) => ScanResponse::Ok { plugins },
            Err(e) => ScanResponse::Err {
                message: e.to_string(),
            },
        }
    }

    /// Instantiate a plugin of `format` on the plugin main thread.
    pub fn instantiate(
        &self,
        format: PluginFormat,
        path: &Path,
        plugin_id: &str,
    ) -> Result<Box<dyn PluginController>, PluginError> {
        self.host(format)?.instantiate(path, plugin_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ether_core::protocol::devices::DeviceCategory;

    /// A format whose targets end in `.<ext>`; `scan` describes one plugin per target.
    struct Fake {
        format: PluginFormat,
        ext: &'static str,
        registry: Vec<PathBuf>,
    }

    impl PluginFormatHost for Fake {
        fn format(&self) -> PluginFormat {
            self.format
        }
        fn default_search_paths(&self) -> Vec<PathBuf> {
            vec![PathBuf::from(format!("/default/{}", self.ext))]
        }
        fn discover(&self, paths: &[PathBuf]) -> Vec<PathBuf> {
            paths
                .iter()
                .map(|p| p.join(format!("x.{}", self.ext)))
                .collect()
        }
        fn discover_registry(&self) -> Vec<PathBuf> {
            self.registry.clone()
        }
        fn claims(&self, target: &Path) -> bool {
            bundles::has_extension(target, self.ext)
        }
        fn scan(&self, target: &Path) -> Result<Vec<PluginDescriptor>, PluginError> {
            Ok(vec![PluginDescriptor {
                sidechain_inputs: Default::default(),
                format: self.format,
                id: target.display().to_string(),
                name: String::new(),
                vendor: String::new(),
                version: String::new(),
                description: String::new(),
                features: Vec::new(),
                category: DeviceCategory::AudioEffect,
                path: target.display().to_string(),
            }])
        }
        fn instantiate(&self, _: &Path, _: &str) -> Result<Box<dyn PluginController>, PluginError> {
            Err(unsupported(self.format, "instantiation"))
        }
    }

    fn formats() -> Formats {
        Formats::new(vec![
            Arc::new(Fake {
                format: PluginFormat::Clap,
                ext: "clap",
                registry: vec![],
            }),
            Arc::new(Fake {
                format: PluginFormat::Au,
                ext: "component",
                registry: vec![PathBuf::from("aufx:dely:appl")],
            }),
        ])
    }

    #[test]
    fn lookup_and_inference() {
        let f = formats();
        assert_eq!(f.formats(), [PluginFormat::Clap, PluginFormat::Au]);
        assert!(f.get(PluginFormat::Vst3).is_none());
        assert!(matches!(
            f.host(PluginFormat::Vst3),
            Err(PluginError::Unsupported(_))
        ));
        assert_eq!(f.infer(Path::new("/a/B.CLAP")), Some(PluginFormat::Clap));
        assert_eq!(f.infer(Path::new("/a/B.vst3")), None);
        assert_eq!(f.default_search_paths().len(), 2);
    }

    #[test]
    fn discover_defaults_include_registry_explicit_paths_dont() {
        let f = formats();
        let all = f.discover(None);
        assert_eq!(
            all,
            vec![
                ScanTarget {
                    format: PluginFormat::Clap,
                    path: "/default/clap/x.clap".into()
                },
                ScanTarget {
                    format: PluginFormat::Au,
                    path: "/default/component/x.component".into()
                },
                ScanTarget {
                    format: PluginFormat::Au,
                    path: "aufx:dely:appl".into()
                },
            ]
        );
        let some = f.discover(Some(&[PathBuf::from("/p")]));
        assert_eq!(some.len(), 2);
        assert!(some.iter().all(|t| t.path.starts_with("/p")));
    }

    #[test]
    fn discover_folders_filters_by_format_toggles_defaults_and_dedupes() {
        let f = formats();
        let user = vec![
            (PathBuf::from("/u/any"), None),
            (PathBuf::from("/u/clap-only"), Some(PluginFormat::Clap)),
            // The same folder twice (and a default listed again) is walked once.
            (PathBuf::from("/u/any"), Some(PluginFormat::Au)),
            (PathBuf::from("/default/clap"), None),
        ];
        let paths = |v: Vec<ScanTarget>| {
            v.into_iter()
                .map(|t| format!("{:?} {}", t.format, t.path.display()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            paths(f.discover_folders(true, &user)),
            [
                "Clap /default/clap/x.clap",
                "Clap /u/any/x.clap",
                "Clap /u/clap-only/x.clap",
                // An `Any` user folder is walked for every format.
                "Au /default/clap/x.component",
                "Au /default/component/x.component",
                "Au /u/any/x.component",
                "Au aufx:dely:appl",
            ]
        );
        // Defaults off: no default folders and no registry.
        assert_eq!(
            paths(f.discover_folders(false, &user[..2])),
            [
                "Clap /u/any/x.clap",
                "Clap /u/clap-only/x.clap",
                "Au /u/any/x.component",
            ]
        );
        assert_eq!(f.discover_folders(true, &[]), f.discover(None));
    }

    #[test]
    fn scan_requests_dispatch_by_format() {
        let f = formats();
        let req = |path: &str, format| ScanRequest {
            bundle_path: path.into(),
            format,
        };
        match f.handle_scan_request(&req("/x/A.clap", None)) {
            ScanResponse::Ok { plugins } => assert_eq!(plugins[0].format, PluginFormat::Clap),
            other => panic!("{other:?}"),
        }
        match f.handle_scan_request(&req("aufx:dely:appl", Some(PluginFormat::Au))) {
            ScanResponse::Ok { plugins } => assert_eq!(plugins[0].format, PluginFormat::Au),
            other => panic!("{other:?}"),
        }
        match f.handle_scan_request(&req("/x/A.vst3", None)) {
            ScanResponse::Err { message } => assert!(message.contains("unsupported"), "{message}"),
            other => panic!("{other:?}"),
        }
        match f.handle_scan_request(&req("/x/A.vst3", Some(PluginFormat::Vst3))) {
            ScanResponse::Err { message } => {
                assert!(
                    message.contains("vst3 plugins are not supported"),
                    "{message}"
                )
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            f.instantiate(PluginFormat::Clap, Path::new("/x"), "id"),
            Err(PluginError::Unsupported(_))
        ));
    }
}
