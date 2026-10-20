//! Library file filters (protocol `ether_protocol::browser::FileFilter`,
//! CONTRACTS.md §12.8): regex whitelist/blacklist rules on file names, deciding which files
//! `Media::ListDirectory`, the index ([`super::list_folder`]) and folder imports
//! (`Browser::CheckFileFilter { paths }`) show.
//!
//! - **Rules.** Case-insensitive, unanchored, on the file name only. A file shows when it
//!   matches a whitelist rule (an empty whitelist allows everything) and no blacklist rule;
//!   folders are hidden only by the blacklist, and a hidden folder hides its contents.
//!   Each list compiles once into a [`RegexSet`] ([`CompiledFilter`]), so a listing costs
//!   one set match per name.
//! - **Storage.** `Settings/library-filters.json` in the writable user library (next to the
//!   keymap), loaded lazily on first use; a file that no longer parses or compiles is
//!   ignored (defaults). Hosts without a writable library keep the filter for the session.
//! - **Defaults** ([`default_filter`]): whitelist = decodable audio
//!   ([`AUDIO_EXTENSIONS`]), MIDI, Ethereal presets (device, rack and plugin presets are all
//!   `.etherpreset`) and templates; blacklist = dotfiles and system/sidecar files.

use std::collections::HashMap;

use ether_core::protocol::browser::{
    BrowserCommand, BrowserEvent, FileFilter, FileFilterCheck, FileFilterCount, FileFilterList,
    FileFilterRuleError, LibraryItemKind, MAX_FILE_FILTER_RULE_LEN, MAX_FILE_FILTER_RULES,
};
use ether_core::protocol::model::PRESET_EXTENSION;
use ether_core::protocol::model::template::TEMPLATE_EXTENSION;
use ether_core::protocol::{Event, ReplyValue};
use regex::{RegexSet, RegexSetBuilder};

use crate::handlers::{event, store_err};
use crate::store::{AUDIO_EXTENSIONS, Library, MIDI_EXTENSIONS, ProjectStore, StoreError};
use crate::tx::{CmdResult, invalid};
use crate::{EngineBridge, EtherController, HostServices, MessageSink};

use super::{FACTORY_ROOT, PROJECTS_ROOT, changed};

/// The filter file inside the user library root.
pub const FILTER_FILE: &str = "Settings/library-filters.json";

/// Compiled regex size limit per list (bytes of program; generous for hand-written rules).
const SIZE_LIMIT: usize = 4 << 20;

/// The default filter (see the module docs).
pub fn default_filter() -> FileFilter {
    FileFilter {
        whitelist: vec![
            format!(r"\.({})$", AUDIO_EXTENSIONS.join("|")),
            format!(r"\.({})$", MIDI_EXTENSIONS.join("|")),
            format!(r"\.({PRESET_EXTENSION}|{TEMPLATE_EXTENSION})$"),
        ],
        blacklist: [
            r"^\.",
            r"^Thumbs\.db$",
            r"^desktop\.ini$",
            r"\.asd$",
            r"\.reapeaks$",
        ]
        .map(str::to_string)
        .to_vec(),
    }
}

/// A filter, compiled. Rules that don't compile are left out (see [`compile`]).
#[derive(Debug, Clone)]
pub struct CompiledFilter {
    /// `None`: no whitelist rule, every file is allowed.
    whitelist: Option<RegexSet>,
    blacklist: Option<RegexSet>,
}

impl CompiledFilter {
    /// A file named `name` is shown.
    pub fn allows_file(&self, name: &str) -> bool {
        self.whitelist.as_ref().is_none_or(|w| w.is_match(name)) && self.allows_dir(name)
    }

    /// A folder named `name` is shown (the blacklist only).
    pub fn allows_dir(&self, name: &str) -> bool {
        self.blacklist.as_ref().is_none_or(|b| !b.is_match(name))
    }

    /// A file at relative `path` (`a/b/c.wav`) is shown: its folders and itself.
    pub fn allows_path(&self, path: &str) -> bool {
        let mut segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        let Some(name) = segs.pop() else {
            return false;
        };
        segs.iter().all(|d| self.allows_dir(d)) && self.allows_file(name)
    }

    /// Shown files among `names` (file name → how many files have it).
    pub fn count(&self, names: impl Iterator<Item = (impl AsRef<str>, u32)>) -> FileFilterCount {
        let (mut shown, mut total) = (0u32, 0u32);
        for (name, n) in names {
            total = total.saturating_add(n);
            if self.allows_file(name.as_ref()) {
                shown = shown.saturating_add(n);
            }
        }
        FileFilterCount { shown, total }
    }
}

/// Compile `filter`; bad rules (too many, too long, empty, not a regex) are reported and
/// left out.
pub fn compile(filter: &FileFilter) -> (CompiledFilter, Vec<FileFilterRuleError>) {
    let mut errors = Vec::new();
    let whitelist = compile_list(FileFilterList::Whitelist, &filter.whitelist, &mut errors);
    let blacklist = compile_list(FileFilterList::Blacklist, &filter.blacklist, &mut errors);
    (
        CompiledFilter {
            whitelist,
            blacklist,
        },
        errors,
    )
}

fn compile_list(
    list: FileFilterList,
    rules: &[String],
    errors: &mut Vec<FileFilterRuleError>,
) -> Option<RegexSet> {
    let mut good = Vec::new();
    for (i, rule) in rules.iter().enumerate() {
        let mut error = |message: String| {
            errors.push(FileFilterRuleError {
                list,
                index: i as u32,
                message,
            })
        };
        if i >= MAX_FILE_FILTER_RULES {
            error(format!("at most {MAX_FILE_FILTER_RULES} rules per list"));
        } else if rule.trim().is_empty() {
            error("empty rule".to_string());
        } else if rule.len() > MAX_FILE_FILTER_RULE_LEN {
            error(format!(
                "rules are at most {MAX_FILE_FILTER_RULE_LEN} characters"
            ));
        } else {
            match build([rule]) {
                Ok(_) => good.push(rule.as_str()),
                Err(e) => error(regex_error(&e)),
            }
        }
    }
    if good.is_empty() {
        return None;
    }
    // Each rule compiled alone; together they can only exceed the size limit.
    match build(&good) {
        Ok(set) => Some(set),
        Err(e) => {
            errors.push(FileFilterRuleError {
                list,
                index: 0,
                message: regex_error(&e),
            });
            None
        }
    }
}

fn build<I: IntoIterator<Item = S>, S: AsRef<str>>(rules: I) -> Result<RegexSet, regex::Error> {
    RegexSetBuilder::new(rules)
        .case_insensitive(true)
        .size_limit(SIZE_LIMIT)
        .build()
}

/// `regex`'s message on one line (its syntax errors draw a caret diagram).
fn regex_error(e: &regex::Error) -> String {
    match e {
        regex::Error::Syntax(s) => s
            .lines()
            .rev()
            .find_map(|l| l.trim().strip_prefix("error: "))
            .map_or_else(|| "invalid regular expression".to_string(), str::to_string),
        regex::Error::CompiledTooBig(_) => "rule too complex".to_string(),
        _ => "invalid regular expression".to_string(),
    }
}

/// A stored filter, if it parses and every rule compiles.
fn parse(bytes: &[u8]) -> Option<FileFilter> {
    let filter: FileFilter = serde_json::from_slice(bytes).ok()?;
    compile(&filter).1.is_empty().then_some(filter)
}

/// Runtime filter state (in `BrowserState`).
#[derive(Debug, Default)]
pub struct FilterState {
    /// The current filter, once loaded.
    current: Option<(FileFilter, CompiledFilter)>,
}

impl<B, H, S, L, X> EtherController<B, H, S, L, X>
where
    B: EngineBridge,
    H: HostServices,
    S: ProjectStore,
    L: Library,
{
    /// The current filter, compiled (loaded from the user library on first use).
    pub fn browser_filter(&mut self) -> &CompiledFilter {
        self.browser_filter_load();
        &self.browser.filter.current.as_ref().expect("loaded").1
    }

    fn browser_filter_load(&mut self) {
        if self.browser.filter.current.is_some() {
            return;
        }
        let stored = self
            .library
            .user_root()
            .and_then(|root| self.library.read(&root, FILTER_FILE).ok())
            .and_then(|bytes| parse(&bytes))
            .unwrap_or_else(default_filter);
        let compiled = compile(&stored).0;
        self.browser.filter.current = Some((stored, compiled));
    }

    fn browser_filter_reply(&mut self) -> ReplyValue {
        self.browser_filter_load();
        let filter = self
            .browser
            .filter
            .current
            .as_ref()
            .expect("loaded")
            .0
            .clone();
        ReplyValue::BrowserFileFilter {
            filter,
            defaults: default_filter(),
        }
    }

    /// `Browser::{Get,Set,Reset,Check}FileFilter`.
    pub fn browser_filter_command(
        &mut self,
        command: &BrowserCommand,
        now: u64,
        out: &mut dyn MessageSink,
    ) -> CmdResult<ReplyValue> {
        match command {
            BrowserCommand::SetFileFilter { filter } => {
                let (compiled, errors) = compile(filter);
                if let Some(e) = errors.first() {
                    let list = match e.list {
                        FileFilterList::Whitelist => "whitelist",
                        FileFilterList::Blacklist => "blacklist",
                    };
                    return Err(invalid(format!(
                        "{list} rule {}: {}",
                        e.index + 1,
                        e.message
                    )));
                }
                self.browser_filter_store(Some(filter))?;
                self.browser_filter_apply(filter.clone(), compiled, now, out);
            }
            BrowserCommand::ResetFileFilter => {
                self.browser_filter_store(None)?;
                let filter = default_filter();
                let compiled = compile(&filter).0;
                self.browser_filter_apply(filter, compiled, now, out);
            }
            BrowserCommand::CheckFileFilter { filter, paths } => {
                let (compiled, errors) = match filter {
                    Some(f) => compile(f),
                    None => (self.browser_filter().clone(), Vec::new()),
                };
                let st = &self.browser;
                let library = (!st.seen_names.is_empty()).then(|| {
                    compiled.count(
                        st.seen_names
                            .values()
                            .flat_map(|m| m.iter().map(|(k, v)| (k, *v))),
                    )
                });
                return Ok(ReplyValue::BrowserFileFilterCheck {
                    check: FileFilterCheck {
                        errors,
                        visible: paths.iter().map(|p| compiled.allows_path(p)).collect(),
                        library,
                    },
                });
            }
            _ => {}
        }
        Ok(self.browser_filter_reply())
    }

    /// Write (`Some`) or remove (`None`) the stored filter.
    fn browser_filter_store(&mut self, filter: Option<&FileFilter>) -> CmdResult<()> {
        let Some(root) = self.library.user_root() else {
            return Ok(());
        };
        match filter {
            Some(f) => {
                let json = serde_json::to_vec_pretty(f)
                    .map_err(|e| invalid(format!("file filter does not serialize: {e}")))?;
                self.library
                    .write_file(&root, FILTER_FILE, &json)
                    .map_err(store_err)
            }
            None => match self.library.remove_file(&root, FILTER_FILE) {
                Ok(()) | Err(StoreError::NotFound(_)) => Ok(()),
                Err(e) => Err(store_err(e)),
            },
        }
    }

    /// Make `filter` current: drop index items it hides, rescan for the ones it shows, tell
    /// the UIs.
    fn browser_filter_apply(
        &mut self,
        filter: FileFilter,
        compiled: CompiledFilter,
        now: u64,
        out: &mut dyn MessageSink,
    ) {
        let st = &mut self.browser;
        st.index.remove_where(|e| {
            e.item.root != FACTORY_ROOT
                && e.item.root != PROJECTS_ROOT
                && matches!(
                    e.item.kind,
                    LibraryItemKind::Audio | LibraryItemKind::Midi | LibraryItemKind::Preset
                )
                && !compiled.allows_path(&e.item.path)
        });
        st.filter.current = Some((filter.clone(), compiled));
        st.mark_dirty(now);
        if st.started {
            self.browser_scan_all(now);
        }
        event(
            out,
            Event::Browser {
                event: BrowserEvent::FileFilterChanged { filter },
            },
        );
        changed(out);
    }
}

/// File names seen by the last scan of each root (root → name → count).
pub type SeenNames = HashMap<String, HashMap<String, u32>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(white: &[&str], black: &[&str]) -> FileFilter {
        FileFilter {
            whitelist: white.iter().map(|s| s.to_string()).collect(),
            blacklist: black.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn defaults_show_supported_files_only() {
        let (f, errors) = compile(&default_filter());
        assert!(errors.is_empty(), "{errors:?}");
        for name in [
            "kick.wav",
            "Kick.WAV",
            "pad.aiff",
            "x.aif",
            "x.flac",
            "x.mp3",
            "song.m4a",
            "x.aac",
            "x.ogg",
            "beat.mid",
            "beat.MIDI",
            "Warm Pad.etherpreset",
            "Band.ethertemplate",
        ] {
            assert!(f.allows_file(name), "{name} shown");
        }
        for name in [
            "Song.als",
            "song.flp",
            "song.rpp",
            "song.dawproject",
            "kick.wav.asd",
            "kick.wav.reapeaks",
            "Thumbs.db",
            "thumbs.db",
            "desktop.ini",
            "Desktop.ini",
            ".DS_Store",
            "._kick.wav",
            "notes.txt",
            "pack.json",
            "README",
        ] {
            assert!(!f.allows_file(name), "{name} hidden");
        }
        // Folders: only the blacklist.
        assert!(f.allows_dir("Drums"));
        assert!(f.allows_dir("Samples.als"));
        assert!(!f.allows_dir(".git"));
    }

    #[test]
    fn blacklist_wins_and_rules_are_case_insensitive() {
        let (f, _) = compile(&filter(&[r"\.wav$"], &["(?-i:^Draft)", "loop"]));
        assert!(f.allows_file("kick.WAV"));
        assert!(!f.allows_file("drum LOOP.wav"), "blacklist wins");
        assert!(!f.allows_file("Draft kick.wav"));
        assert!(
            f.allows_file("draft kick.wav"),
            "a rule can opt out of case folding"
        );
        assert!(!f.allows_file("kick.aiff"));
    }

    #[test]
    fn empty_whitelist_allows_everything() {
        let (f, _) = compile(&filter(&[], &[r"\.asd$"]));
        assert!(f.allows_file("Song.als"));
        assert!(!f.allows_file("kick.wav.asd"));
        let (f, _) = compile(&filter(&[], &[]));
        assert!(f.allows_file("anything"));
    }

    #[test]
    fn paths_check_their_folders() {
        let (f, _) = compile(&filter(&[r"\.wav$"], &["^backup$"]));
        assert!(f.allows_path("Drums/kick.wav"));
        assert!(!f.allows_path("Drums/Backup/kick.wav"));
        assert!(!f.allows_path("Drums/kick.txt"));
        assert!(!f.allows_path(""));
    }

    #[test]
    fn invalid_rules_are_reported_and_left_out() {
        let long = "a".repeat(MAX_FILE_FILTER_RULE_LEN + 1);
        let (f, errors) = compile(&filter(&[r"\.wav$", "(unclosed", " "], &["[z-a]", &long]));
        let at: Vec<_> = errors.iter().map(|e| (e.list, e.index)).collect();
        assert_eq!(
            at,
            [
                (FileFilterList::Whitelist, 1),
                (FileFilterList::Whitelist, 2),
                (FileFilterList::Blacklist, 0),
                (FileFilterList::Blacklist, 1),
            ]
        );
        assert!(
            errors
                .iter()
                .all(|e| !e.message.contains('\n') && !e.message.is_empty()),
            "{errors:?}"
        );
        assert!(
            errors[0].message.contains("unclosed"),
            "{}",
            errors[0].message
        );
        // The good rule still applies.
        assert!(f.allows_file("a.wav"));
        assert!(!f.allows_file("a.txt"));
    }

    #[test]
    fn too_many_rules() {
        let rules: Vec<String> = (0..=MAX_FILE_FILTER_RULES)
            .map(|i| format!("r{i}"))
            .collect();
        let (_, errors) = compile(&FileFilter {
            whitelist: rules,
            blacklist: vec![],
        });
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].index as usize, MAX_FILE_FILTER_RULES);
    }

    #[test]
    fn counts() {
        let (f, _) = compile(&default_filter());
        let names = [("a.wav", 3u32), ("b.als", 2), ("Thumbs.db", 1)];
        assert_eq!(
            f.count(names.into_iter()),
            FileFilterCount { shown: 3, total: 6 }
        );
    }

    #[test]
    fn stored_files_must_compile() {
        assert_eq!(
            parse(br#"{"whitelist":["\\.wav$"],"blacklist":[]}"#),
            Some(filter(&[r"\.wav$"], &[]))
        );
        assert_eq!(parse(br#"{"whitelist":["("],"blacklist":[]}"#), None);
        assert_eq!(parse(b"not json"), None);
    }
}
