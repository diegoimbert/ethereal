//! On the web: the library file filters through the local wasm controller over
//! OPFS (`WebLibrary<MemFs>`): the folder view of the browser library lists only supported
//! files by default, a whitelist rule shows a new type, and the filter is stored in the
//! OPFS user library (`user-library/Settings/library-filters.json`) and back after a reload.

use ether_controller::{Controller, ControllerConfig, EtherController, HostServices};
use ether_core::protocol::browser::{BrowserCommand, FileFilter};
use ether_core::protocol::media::{BrowseLocation, MediaCommand};
use ether_core::protocol::message::{Command, ReplyResult, ReplyValue};
use ether_core::protocol::{ClientMessage, ServerMessage};
use ether_wasm::bridge::{self, WebBridge};
use ether_wasm::ring::HeapMemory;
use ether_wasm::store::{
    Fs, LIBRARY_ID, LIBRARY_ROOT, MemFs, USER_LIBRARY_ROOT, WebLibrary, WebStore,
};

struct TestHost;

impl HostServices for TestHost {
    fn now_ms(&self) -> u64 {
        1_750_000_000_000
    }
    fn random_seed(&mut self) -> u64 {
        42
    }
}

type Ctl = EtherController<WebBridge<HeapMemory>, TestHost, WebStore<MemFs>, WebLibrary<MemFs>>;

fn controller(fs: &MemFs) -> Ctl {
    let shared = bridge::shared(HeapMemory::new(1 << 16), HeapMemory::new(1 << 14));
    EtherController::with_config(
        WebBridge::new(shared),
        TestHost,
        WebStore::new(fs.clone()),
        WebLibrary::new(fs.clone()),
        ControllerConfig::default(),
    )
}

fn ok(ctl: &mut Ctl, command: Command) -> ReplyValue {
    let mut out = Vec::new();
    ctl.handle(
        ClientMessage {
            id: 1,
            gesture: None,
            command,
        },
        &mut out,
    );
    match out.pop() {
        Some(ServerMessage::Reply(r)) => match r.result {
            ReplyResult::Ok { value } => value,
            ReplyResult::Err { error } => panic!("{error:?}"),
        },
        other => panic!("last message must be the reply, got {other:?}"),
    }
}

fn names(ctl: &mut Ctl) -> Vec<String> {
    match ok(
        ctl,
        Command::Media(MediaCommand::ListDirectory {
            location: BrowseLocation::Library {
                id: LIBRARY_ID.into(),
            },
            path: "Kit".into(),
        }),
    ) {
        ReplyValue::Directory { listing } => listing.entries.into_iter().map(|e| e.name).collect(),
        other => panic!("{other:?}"),
    }
}

fn filter(ctl: &mut Ctl) -> (FileFilter, FileFilter) {
    match ok(ctl, Command::Browser(BrowserCommand::GetFileFilter)) {
        ReplyValue::BrowserFileFilter { filter, defaults } => (filter, defaults),
        other => panic!("{other:?}"),
    }
}

#[test]
fn opfs_listing_is_filtered_and_the_filter_survives_a_reload() {
    let mut fs = MemFs::new();
    let (_, kick) = &ether_media::demo::demo_samples()[0];
    fs.write(&format!("{LIBRARY_ROOT}/Kit/Kick.wav"), kick)
        .unwrap();
    for f in ["Kick.wav.asd", "Kit.als", "Thumbs.db", "notes.txt"] {
        fs.write(&format!("{LIBRARY_ROOT}/Kit/{f}"), b"x").unwrap();
    }
    let mut ctl = controller(&fs);
    assert_eq!(names(&mut ctl), ["Kick.wav"]);

    let (_, defaults) = filter(&mut ctl);
    let mut shown = defaults.clone();
    shown.whitelist.push(r"\.als$".into());
    ok(
        &mut ctl,
        Command::Browser(BrowserCommand::SetFileFilter {
            filter: shown.clone(),
        }),
    );
    assert_eq!(names(&mut ctl), ["Kick.wav", "Kit.als"]);
    assert!(fs.files().contains(&format!(
        "{USER_LIBRARY_ROOT}/Settings/library-filters.json"
    )));

    // Reload: a fresh controller over the same OPFS.
    let mut ctl = controller(&fs);
    assert_eq!(filter(&mut ctl).0, shown);
    assert_eq!(names(&mut ctl), ["Kick.wav", "Kit.als"]);
}
