//! Project folder names on disk: `<projects_root>/<title> - <short id>/`.
//!
//! A project's identity is its [`ProjectId`]; the folder name is only for people browsing
//! the projects folder in a file manager. It is the project's title (made safe for every OS
//! file system, [`folder_name`]) followed by the last 8 hex digits of the id (the random
//! part of the UUIDv7), so two projects with the same title never share a folder.
//!
//! - **Identity.** A folder named with a bare UUID (the layout before readable names, and a
//!   new project before its first [`settle`]) is that project. Any other folder is the
//!   project whose id is in its `project.ether` (`project.id`, read from the head of the
//!   file). A folder whose document has no readable id is not a project. So a folder renamed
//!   by hand in a file manager still opens.
//! - **Renames** ([`settle`]) happen only when nothing has files open in the folder: a
//!   project that was just created or copied, one about to be opened, one that was just
//!   closed, and every project at startup ([`settle_all`], which also migrates bare-UUID
//!   folders). Never the open project (its media streams and recordings hold paths): its
//!   folder follows a new title once it closes. A rename is a single `rename(2)` of the
//!   folder, so a crash leaves either name, both of which resolve. A failed rename (a file
//!   manager holding the folder on Windows, a taken name) keeps the current name and is
//!   tried again next time.
//! - **Lookups** ([`dir`]) go through a per-root index (process-wide, so the store, the
//!   recorder and the disk streamer agree), rebuilt by a scan of the root when an id isn't
//!   in it or its folder is gone. Unknown ids resolve to `<root>/<uuid>` (where a new
//!   project is created).

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use ether_core::protocol::model::{ForkOrigin, ProjectId};

/// File name of the document inside a project folder (`store::PROJECT_FILE`).
const PROJECT_FILE: &str = "project.ether";
/// Longest title part of a folder name, in characters.
const MAX_TITLE_CHARS: usize = 60;
/// Title part when the title has nothing usable.
const FALLBACK_TITLE: &str = "untitled";

/// Index of each root: id → folder name.
type Index = HashMap<ProjectId, OsString>;
static INDEX: LazyLock<Mutex<HashMap<PathBuf, Index>>> = LazyLock::new(Mutex::default);

fn with_index<T>(root: &Path, f: impl FnOnce(&mut Index) -> T) -> T {
    let mut all = INDEX.lock().unwrap_or_else(|p| p.into_inner());
    f(all.entry(root.to_path_buf()).or_default())
}

/// The id part of a folder name: the last 8 hex digits of the UUID.
pub fn short_id(id: ProjectId) -> String {
    let hex = id.0.simple().to_string();
    hex[hex.len() - 8..].to_string()
}

/// The folder name of project `id` titled `title`: `<title> - <short id>`. The title keeps
/// its case and spaces; characters no file system accepts (`<>:"/\|?*`, control
/// characters) become `_`, leading dots (hidden files) and trailing dots/spaces (Windows)
/// go, it is cut at [`MAX_TITLE_CHARS`], and a Windows device name (`CON`, `COM1`, ...)
/// gets a `_` in front.
pub fn folder_name(title: &str, id: ProjectId) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if c.is_whitespace() {
                ' '
            } else if c.is_control()
                || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
            {
                '_'
            } else {
                c
            }
        })
        .collect();
    // Collapse runs of spaces.
    let mut title = String::with_capacity(cleaned.len());
    for word in cleaned.split(' ').filter(|w| !w.is_empty()) {
        if !title.is_empty() {
            title.push(' ');
        }
        title.push_str(word);
    }
    let mut title: String = title
        .trim_start_matches(['.', ' '])
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect();
    let trimmed = title.trim_end_matches(['.', ' ']).len();
    title.truncate(trimmed);
    if title.is_empty() {
        title = FALLBACK_TITLE.to_string();
    }
    let stem = title
        .split('.')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        title.insert(0, '_');
    }
    format!("{title} - {}", short_id(id))
}

/// What the head of a `project.ether` says: `project.id` and `project.settings.name`.
#[derive(Debug, Default, PartialEq)]
pub struct Head {
    pub id: Option<ProjectId>,
    pub name: Option<String>,
    /// `project.settings.forked_from` (read with the rest of the settings).
    pub forked_from: Option<ForkOrigin>,
}

/// Read [`Head`] from a document, streaming: the app writes `project.id` and
/// `project.settings.name` within the first few hundred bytes, so this reads a few KB
/// however big the document is. Other key orders still work (what comes first is skipped).
pub fn read_head(reader: impl Read) -> Head {
    let mut head = Head::default();
    let mut de = serde_json::Deserializer::from_reader(reader);
    // Having found both aborts the parse with an error, on purpose.
    let _ = serde::de::DeserializeSeed::deserialize(HeadSeed(Level::Top, &mut head), &mut de);
    head
}

#[derive(Clone, Copy)]
enum Level {
    Top,
    Project,
    Settings,
}

struct HeadSeed<'a>(Level, &'a mut Head);

impl<'de> serde::de::DeserializeSeed<'de> for HeadSeed<'_> {
    type Value = ();

    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<(), D::Error> {
        de.deserialize_map(self)
    }
}

impl<'de> serde::de::Visitor<'de> for HeadSeed<'_> {
    type Value = ();

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("an object")
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let HeadSeed(level, head) = self;
        while let Some(key) = map.next_key::<String>()? {
            match (level, key.as_str()) {
                (Level::Top, "project") => {
                    return map.next_value_seed(HeadSeed(Level::Project, head));
                }
                (Level::Project, "id") => {
                    let id: String = map.next_value()?;
                    head.id = id.parse().ok();
                    if head.name.is_some() {
                        return Err(serde::de::Error::custom("found"));
                    }
                }
                (Level::Project, "settings") => {
                    map.next_value_seed(HeadSeed(Level::Settings, &mut *head))?;
                    if head.name.is_some() && head.id.is_some() {
                        return Err(serde::de::Error::custom("found"));
                    }
                }
                (Level::Settings, "name") => {
                    head.name = Some(map.next_value()?);
                }
                (Level::Settings, "forked_from") => {
                    // A malformed origin is ignored, not a reason to lose the head.
                    let v: serde_json::Value = map.next_value()?;
                    head.forked_from = serde_json::from_value(v).ok();
                }
                _ => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
        }
        Ok(())
    }
}

/// The head of the document in `dir`.
pub fn head_of(dir: &Path) -> Option<Head> {
    let file = fs::File::open(dir.join(PROJECT_FILE)).ok()?;
    Some(read_head(std::io::BufReader::new(file)))
}

/// A project folder found by [`scan`].
#[derive(Debug)]
pub struct Folder {
    pub id: ProjectId,
    pub dir: PathBuf,
    /// The document's title (`None`: unreadable).
    pub name: Option<String>,
    /// The document's fork origin.
    pub forked_from: Option<ForkOrigin>,
}

/// Every project folder of `root` (hidden folders and folders that are no project are
/// skipped), and the index rebuilt from them. Two folders claiming the same id (one copied
/// by hand): the one with the expected name, else a bare-UUID one, else the first by name;
/// the others are left alone and not listed.
pub fn scan(root: &Path) -> Vec<Folder> {
    let Ok(rd) = fs::read_dir(root) else {
        with_index(root, |i| i.clear());
        return Vec::new();
    };
    let mut entries: Vec<(OsString, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| (e.file_name(), e.path()))
        .filter(|(n, _)| !n.to_string_lossy().starts_with('.'))
        .collect();
    entries.sort();
    let mut found: HashMap<ProjectId, (Folder, u8)> = HashMap::new();
    for (name, dir) in entries {
        let named = name.to_str().and_then(|n| n.parse::<ProjectId>().ok());
        let head = head_of(&dir);
        let (id, rank) = match (named, &head) {
            (Some(id), _) => (id, 1),
            (None, Some(Head { id: Some(id), .. })) => (*id, 2),
            _ => continue,
        };
        let (title, forked_from) = head.map_or((None, None), |h| (h.name, h.forked_from));
        let expected = title.as_deref().map(|t| folder_name(t, id));
        let rank = if expected.as_deref() == name.to_str() {
            0
        } else {
            rank
        };
        let folder = Folder {
            id,
            dir,
            name: title,
            forked_from,
        };
        match found.get(&id) {
            Some((_, r)) if *r <= rank => {
                tracing::warn!(project = %id, dir = %folder.dir.display(), "a second folder of the same project is ignored");
            }
            _ => {
                found.insert(id, (folder, rank));
            }
        }
    }
    let folders: Vec<Folder> = found.into_values().map(|(f, _)| f).collect();
    with_index(root, |index| {
        index.clear();
        for f in &folders {
            if let Some(n) = f.dir.file_name() {
                index.insert(f.id, n.to_os_string());
            }
        }
    });
    folders
}

/// The folder of project `id` if it exists (the index, else a scan).
pub fn existing(root: &Path, id: ProjectId) -> Option<PathBuf> {
    let cached = with_index(root, |i| i.get(&id).map(|n| root.join(n)));
    if let Some(dir) = cached.filter(|d| d.is_dir()) {
        return Some(dir);
    }
    // The old layout's path needs no scan.
    let bare = root.join(id.to_string());
    if bare.is_dir() {
        with_index(root, |i| {
            i.insert(id, bare.file_name().expect("named").to_os_string())
        });
        return Some(bare);
    }
    scan(root).into_iter().find(|f| f.id == id).map(|f| f.dir)
}

/// The folder of project `id`: where it is, else where a new one is created
/// (`<root>/<uuid>`, renamed by its first [`settle`]).
pub fn dir(root: &Path, id: ProjectId) -> PathBuf {
    existing(root, id).unwrap_or_else(|| root.join(id.to_string()))
}

/// Give project `id`'s folder its readable name (its document's title). Only call it when
/// nothing holds files open in the folder (see the module docs). Returns the folder (renamed
/// or not); `None` if there is no such project. A failure keeps the current name.
pub fn settle(root: &Path, id: ProjectId) -> Option<PathBuf> {
    let dir = existing(root, id)?;
    let head = head_of(&dir)?;
    // A folder only becomes readable when its document says which project it is: that is
    // how a readable folder is found again (a bare-UUID folder whose document has another
    // id, a copy that hasn't been rewritten yet, keeps its name).
    if head.id != Some(id) {
        return Some(dir);
    }
    let target = root.join(folder_name(head.name.as_deref().unwrap_or(""), id));
    if target == dir {
        return Some(dir);
    }
    if target.exists() {
        tracing::warn!(project = %id, target = %target.display(), "project folder name taken; kept as is");
        return Some(dir);
    }
    match fs::rename(&dir, &target) {
        Ok(()) => {
            tracing::info!(project = %id, from = %dir.display(), to = %target.display(), "project folder renamed");
            with_index(root, |i| {
                i.insert(id, target.file_name().expect("named").to_os_string())
            });
            Some(target)
        }
        Err(e) => {
            tracing::warn!(project = %id, error = %e, "project folder not renamed (tried again later)");
            Some(dir)
        }
    }
}

/// [`settle`] every project of `root` (at startup, nothing open): migrates bare-UUID
/// folders to readable names and follows titles changed while a project was open.
/// Returns how many folders were renamed.
pub fn settle_all(root: &Path) -> usize {
    let mut renamed = 0;
    for f in scan(root) {
        if settle(root, f.id).is_some_and(|d| d != f.dir) {
            renamed += 1;
        }
    }
    renamed
}

/// Forget a deleted project's folder.
pub fn forget(root: &Path, id: ProjectId) {
    with_index(root, |i| {
        i.remove(&id);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempDir;

    fn pid(n: u8) -> ProjectId {
        ProjectId::v7(1_700_000_000_000 + n as u64, [n; 10])
    }

    fn doc(id: Option<ProjectId>, name: &str) -> String {
        let mut project =
            serde_json::json!({ "settings": { "name": name, "loop_enabled": false } });
        if let Some(id) = id {
            project["id"] = serde_json::json!(id.to_string());
        }
        serde_json::json!({
            "format": "ethereal-project",
            "format_version": 1,
            "app_version": "test",
            "project": project,
        })
        .to_string()
    }

    fn make(root: &Path, folder: &str, json: &str) -> PathBuf {
        let dir = root.join(folder);
        fs::create_dir_all(dir.join("media")).unwrap();
        fs::write(dir.join(PROJECT_FILE), json).unwrap();
        fs::write(dir.join("media/kick.wav"), b"RIFF").unwrap();
        dir
    }

    fn names(root: &Path) -> Vec<String> {
        let mut n: Vec<String> = fs::read_dir(root)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        n.sort();
        n
    }

    #[test]
    fn folder_names_are_readable_safe_and_unique() {
        let a = pid(1);
        let short = short_id(a);
        assert_eq!(short.len(), 8);
        assert_eq!(folder_name("My Song", a), format!("My Song - {short}"));
        assert_eq!(
            folder_name("a/b:c*?\"<>|d", a),
            format!("a_b_c______d - {short}")
        );
        assert_eq!(
            folder_name("  ..hidden  song.. ", a),
            format!("hidden song - {short}")
        );
        assert_eq!(folder_name("", a), format!("untitled - {short}"));
        assert_eq!(folder_name("...", a), format!("untitled - {short}"));
        assert_eq!(folder_name("con", a), format!("_con - {short}"));
        assert_eq!(folder_name("COM1.mix", a), format!("_COM1.mix - {short}"));
        assert_eq!(folder_name("tab\there", a), format!("tab here - {short}"));
        let long = "x".repeat(200);
        assert_eq!(
            folder_name(&long, a).chars().count(),
            MAX_TITLE_CHARS + 3 + 8
        );
        // Same title, other project: another folder.
        assert_ne!(folder_name("Song", a), folder_name("Song", pid(2)));
        // Never mistaken for the old layout or hidden.
        assert!(folder_name("x", a).parse::<ProjectId>().is_err());
    }

    #[test]
    fn the_head_gives_id_and_name_in_any_order() {
        let a = pid(3);
        let head = read_head(doc(Some(a), "Tune").as_bytes());
        assert_eq!(
            head,
            Head {
                id: Some(a),
                name: Some("Tune".into()),
                forked_from: None,
            }
        );
        let late =
            format!(r#"{{"project":{{"settings":{{"name":"Late"}},"tracks":{{}},"id":"{a}"}}}}"#);
        assert_eq!(
            read_head(late.as_bytes()),
            Head {
                id: Some(a),
                name: Some("Late".into()),
                forked_from: None,
            }
        );
        assert_eq!(read_head(&b"not json"[..]), Head::default());
        // A fork's origin is read with the settings; a malformed one is ignored.
        let fork = format!(
            r#"{{"project":{{"id":"{a}","settings":{{"name":"Fork","forked_from":{{"project":"{a}","name":"Tune","created_ms":5.0}}}}}}}}"#
        );
        let head = read_head(fork.as_bytes());
        assert_eq!(head.name.as_deref(), Some("Fork"));
        assert_eq!(
            head.forked_from,
            Some(ForkOrigin {
                project: a,
                name: "Tune".into(),
                created_ms: 5.0
            })
        );
        let bad =
            format!(r#"{{"project":{{"id":"{a}","settings":{{"name":"F","forked_from":3}}}}}}"#);
        let head = read_head(bad.as_bytes());
        assert_eq!((head.name.as_deref(), head.forked_from), (Some("F"), None));
    }

    #[test]
    fn migration_renames_bare_uuid_folders_without_losing_anything() {
        let tmp = TempDir::new("dirs-migrate");
        let root = tmp.path().join("projects");
        let (a, b, c, d) = (pid(10), pid(11), pid(12), pid(13));
        make(&root, &a.to_string(), &doc(Some(a), "First Song"));
        make(&root, &b.to_string(), &doc(Some(b), "First Song"));
        // A document without an id (or with another project's id) keeps its UUID folder.
        make(&root, &c.to_string(), &doc(None, "No id"));
        make(&root, &d.to_string(), &doc(Some(a), "Copy in progress"));
        fs::create_dir_all(root.join(".uploads")).unwrap();
        fs::create_dir_all(root.join("not a project")).unwrap();

        assert_eq!(settle_all(&root), 2);
        let mut expected = vec![
            ".uploads".to_string(),
            c.to_string(),
            d.to_string(),
            folder_name("First Song", a),
            folder_name("First Song", b),
            "not a project".to_string(),
        ];
        expected.sort();
        assert_eq!(names(&root), expected);
        for id in [a, b, c, d] {
            let dir = dir(&root, id);
            assert!(dir.is_dir(), "{id} resolves");
            assert_eq!(fs::read(dir.join("media/kick.wav")).unwrap(), b"RIFF");
        }
        assert_eq!(
            head_of(&dir(&root, a)).unwrap().name.as_deref(),
            Some("First Song")
        );
        // Stable: nothing more to do.
        assert_eq!(settle_all(&root), 0);
        // A fresh process (empty index) finds them by scanning.
        with_index(&root, |i| i.clear());
        assert_eq!(dir(&root, b), root.join(folder_name("First Song", b)));
        assert_eq!(scan(&root).len(), 4);
    }

    #[test]
    fn a_title_change_renames_the_folder_when_settled() {
        let tmp = TempDir::new("dirs-rename");
        let root = tmp.path().join("projects");
        let a = pid(20);
        make(&root, &a.to_string(), &doc(Some(a), "Draft"));
        let first = settle(&root, a).unwrap();
        assert_eq!(first, root.join(folder_name("Draft", a)));
        fs::write(first.join(PROJECT_FILE), doc(Some(a), "Final Mix")).unwrap();
        // Until settled, the old folder is still the project.
        assert_eq!(dir(&root, a), first);
        let second = settle(&root, a).unwrap();
        assert_eq!(second, root.join(folder_name("Final Mix", a)));
        assert!(!first.exists());
        assert_eq!(names(&root), vec![folder_name("Final Mix", a)]);
        assert_eq!(dir(&root, a), second);
    }

    #[test]
    fn a_folder_renamed_by_hand_still_resolves() {
        let tmp = TempDir::new("dirs-by-hand");
        let root = tmp.path().join("projects");
        let a = pid(30);
        make(&root, &a.to_string(), &doc(Some(a), "Song"));
        let settled = settle(&root, a).unwrap();
        let by_hand = root.join("my favourite");
        fs::rename(&settled, &by_hand).unwrap();
        assert_eq!(dir(&root, a), by_hand);
        // ...and gets its name back next time.
        assert_eq!(settle(&root, a).unwrap(), settled);
    }

    #[test]
    fn a_taken_name_or_a_hand_copy_keeps_folders_as_they_are() {
        let tmp = TempDir::new("dirs-taken");
        let root = tmp.path().join("projects");
        let a = pid(40);
        let dir_a = make(&root, &a.to_string(), &doc(Some(a), "Song"));
        // Something else sits at the target name.
        fs::create_dir_all(root.join(folder_name("Song", a))).unwrap();
        assert_eq!(settle(&root, a).unwrap(), dir_a);
        assert!(dir_a.is_dir());
        // A folder copied by hand (same id): the bare-UUID one stays the project.
        fs::remove_dir_all(root.join(folder_name("Song", a))).unwrap();
        make(&root, "Song copy", &doc(Some(a), "Song"));
        with_index(&root, |i| i.clear());
        assert_eq!(dir(&root, a), dir_a);
        assert_eq!(scan(&root).len(), 1);
    }

    #[test]
    fn unknown_ids_resolve_to_a_new_uuid_folder() {
        let tmp = TempDir::new("dirs-unknown");
        let root = tmp.path().join("projects");
        let a = pid(50);
        assert_eq!(existing(&root, a), None);
        assert_eq!(dir(&root, a), root.join(a.to_string()));
        assert_eq!(settle(&root, a), None);
    }
}
