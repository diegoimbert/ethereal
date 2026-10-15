//! `base-136` on the web: a folder imported from the user's computer is copied into OPFS
//! (`imported/<folder>/`, preserving sub-folders) through the local wasm controller
//! (`Browser::ImportFolder`, uploads + `ImportFile`, `Rescan`), indexed like any library
//! root, imported into a project from there, and back after a reload (a fresh controller
//! over the same OPFS). Removing it deletes the copy.

use ether_controller::{Controller, ControllerConfig, EtherController, HostServices};
use ether_core::protocol::browser::{
    BrowserCommand, BrowserPage, BrowserQuery, BrowserRoot, BrowserRootKind, BrowserSort,
};
use ether_core::protocol::media::{MediaCommand, MediaSource};
use ether_core::protocol::message::{Command, ReplyResult, ReplyValue};
use ether_core::protocol::model::{Base64Bytes, MediaId, ProjectId, Ulid};
use ether_core::protocol::project::ProjectCommand;
use ether_core::protocol::{ClientMessage, ServerMessage};
use ether_wasm::bridge::{self, WebBridge};
use ether_wasm::ring::HeapMemory;
use ether_wasm::store::{IMPORTED_ROOT, MemFs, WebLibrary, WebStore};

struct TestHost {
    now: std::rc::Rc<std::cell::Cell<u64>>,
}

impl HostServices for TestHost {
    fn now_ms(&self) -> u64 {
        self.now.get()
    }
    fn random_seed(&mut self) -> u64 {
        42
    }
}

type Ctl = EtherController<WebBridge<HeapMemory>, TestHost, WebStore<MemFs>, WebLibrary<MemFs>>;

struct H {
    ctl: Ctl,
    now: std::rc::Rc<std::cell::Cell<u64>>,
    next: u32,
}

impl H {
    fn new(fs: &MemFs) -> Self {
        let shared = bridge::shared(HeapMemory::new(1 << 16), HeapMemory::new(1 << 14));
        let now = std::rc::Rc::new(std::cell::Cell::new(1_750_000_000_000));
        let ctl = EtherController::with_config(
            WebBridge::new(shared),
            TestHost { now: now.clone() },
            WebStore::new(fs.clone()),
            WebLibrary::new(fs.clone()),
            ControllerConfig {
                engine_sample_rate: 48_000,
                ..ControllerConfig::default()
            },
        );
        Self { ctl, now, next: 1 }
    }

    fn send(&mut self, command: Command) -> ReplyResult {
        let mut out = Vec::new();
        self.next += 1;
        self.ctl.handle(
            ClientMessage {
                id: self.next,
                gesture: None,
                command,
            },
            &mut out,
        );
        match out.pop() {
            Some(ServerMessage::Reply(r)) => r.result,
            other => panic!("last message must be the reply, got {other:?}"),
        }
    }

    fn ok(&mut self, command: Command) -> ReplyValue {
        match self.send(command) {
            ReplyResult::Ok { value } => value,
            ReplyResult::Err { error } => panic!("{error:?}"),
        }
    }

    fn browser(&mut self, c: BrowserCommand) -> ReplyValue {
        self.ok(Command::Browser(c))
    }

    fn roots(&mut self) -> Vec<BrowserRoot> {
        match self.browser(BrowserCommand::ListRoots) {
            ReplyValue::BrowserRoots { roots } => roots,
            other => panic!("{other:?}"),
        }
    }

    /// Tick until the index settles.
    fn index(&mut self) {
        for _ in 0..2000 {
            self.now.set(self.now.get() + 50);
            let mut out = Vec::new();
            self.ctl.tick(self.now.get(), &mut out);
        }
    }

    fn query(&mut self, root: &str) -> BrowserPage {
        match self.browser(BrowserCommand::Query {
            query: BrowserQuery {
                text: String::new(),
                kinds: vec![],
                tags: vec![],
                favourites_only: false,
                roots: vec![root.to_string()],
                folder: None,
                device: None,
                sort: BrowserSort::Name,
                offset: 0,
                limit: 200,
            },
        }) {
            ReplyValue::BrowserPage { page } => page,
            other => panic!("{other:?}"),
        }
    }

    /// Upload `bytes` in 4 KiB chunks and write them to `root`/`path`.
    fn import_file(&mut self, root: &str, path: &str, bytes: &[u8]) {
        let upload = format!("u{}", self.next);
        self.ok(Command::Media(MediaCommand::BeginUpload {
            upload: upload.clone(),
            name: path.rsplit('/').next().unwrap().into(),
            size: bytes.len() as f64,
        }));
        for (i, chunk) in bytes.chunks(4096).enumerate() {
            self.ok(Command::Media(MediaCommand::UploadChunk {
                upload: upload.clone(),
                offset: (i * 4096) as f64,
                data: Base64Bytes(chunk.to_vec()),
            }));
        }
        self.browser(BrowserCommand::ImportFile {
            root: root.into(),
            path: path.into(),
            upload,
        });
    }
}

#[test]
fn imported_folder_lives_in_opfs_and_survives_a_reload() {
    let fs = MemFs::new();
    let mut h = H::new(&fs);
    h.roots();
    let ReplyValue::BrowserRoots { roots } = h.browser(BrowserCommand::ImportFolder {
        name: "Drum Kit".into(),
    }) else {
        panic!()
    };
    let folder = roots
        .iter()
        .find(|r| r.kind == BrowserRootKind::Folder)
        .expect("a user folder")
        .clone();
    assert_eq!(folder.name, "Drum Kit");
    assert_eq!(
        folder.path.as_deref(),
        Some(format!("{IMPORTED_ROOT}/Drum Kit").as_str())
    );
    let samples = ether_media::demo::demo_samples();
    let (_, kick) = &samples[0];
    let (_, snare) = &samples[1];
    h.import_file(&folder.id, "Kick.wav", kick);
    h.import_file(&folder.id, "Snares/Tight/Snare.wav", snare);
    let mut files: Vec<String> = fs
        .files()
        .into_iter()
        .filter(|f| f.starts_with("imported/"))
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            "imported/Drum Kit/Kick.wav",
            "imported/Drum Kit/Snares/Tight/Snare.wav"
        ]
    );
    assert!(
        !fs.files().iter().any(|f| f.starts_with("uploads/")),
        "staging dropped"
    );

    h.browser(BrowserCommand::Rescan {
        root: Some(folder.id.clone()),
    });
    h.index();
    let page = h.query(&folder.id);
    assert_eq!(page.total, 2);
    let snare_item = page
        .items
        .iter()
        .find(|i| i.name == "Snare.wav")
        .unwrap()
        .clone();
    assert_eq!(snare_item.path, "Snares/Tight/Snare.wav");
    assert!(snare_item.meta.duration_seconds.is_some(), "probed");

    // A drop on a lane imports the library item like any other (copied into the project).
    let pid = ProjectId::v7(1_750_000_000_000, [7; 10]);
    h.ok(Command::Project(ProjectCommand::Create {
        id: pid,
        name: "Web".into(),
    }));
    let source = snare_item.source.clone().unwrap();
    assert!(matches!(source, MediaSource::Location { .. }));
    let ReplyValue::Media { media } = h.ok(Command::Media(MediaCommand::Import {
        id: MediaId(Ulid(9)),
        source,
    })) else {
        panic!()
    };
    assert!(
        fs.files()
            .contains(&format!("projects/{pid}/{}", media.file))
    );

    // Reload: a fresh controller over the same OPFS finds the folder and its items.
    h.now.set(h.now.get() + 60_000);
    h.index();
    let mut h = H::new(&fs);
    let roots = h.roots();
    let back = roots
        .iter()
        .find(|r| r.id == folder.id)
        .expect("restored after a reload");
    assert_eq!(back.kind, BrowserRootKind::Folder);
    assert_eq!(back.name, "Drum Kit");
    assert_eq!(h.query(&folder.id).total, 2);
    h.index();
    assert_eq!(h.query(&folder.id).total, 2);
    let ReplyValue::Locations { locations } = h.ok(Command::Media(MediaCommand::ListLocations))
    else {
        panic!()
    };
    assert!(locations.iter().any(|l| l.name == "Drum Kit"), "browsable");

    // Removing it deletes the copy.
    h.browser(BrowserCommand::RemoveFolder {
        root: folder.id.clone(),
    });
    assert!(!fs.files().iter().any(|f| f.starts_with("imported/")));
    assert!(!h.roots().iter().any(|r| r.id == folder.id));
}
