//! Settings "sandboxed by default" (#231) through the desktop host (null audio backend):
//! add / update / remove by format + plugin id, persisted across a restart.

use crate::common;

use common::{Client, Paths};
use ether_native::test_util::TempDir;
use serde_json::{Value, json};

fn set(c: &mut Client, format: &str, id: &str, name: Option<&str>, on: bool) -> Value {
    c.ok(
        "Plugin",
        json!({"type": "SetSandboxDefault", "format": format, "plugin_id": id, "name": name, "sandboxed": on}),
    )["plugins"]
        .clone()
}

#[test]
fn sandbox_defaults_are_edited_and_persisted_by_the_host() {
    let tmp = TempDir::new("sandbox-defaults-e2e");
    let paths = Paths::new(&tmp.path().join("data"));

    let mut c = Client::start(&paths);
    assert_eq!(
        c.ok("Plugin", json!({"type": "ListSandboxDefaults"}))["plugins"],
        json!([])
    );
    set(&mut c, "Vst3", "ABCDEF0123", Some("Verb"), true);
    let l = set(&mut c, "Clap", "com.acme.synth", None, true);
    assert_eq!(
        l,
        json!([
            {"format": "Vst3", "plugin_id": "ABCDEF0123", "name": "Verb"},
            {"format": "Clap", "plugin_id": "com.acme.synth", "name": "com.acme.synth"},
        ])
    );
    c.quit();

    let mut c = Client::start(&paths);
    assert_eq!(
        c.ok("Plugin", json!({"type": "ListSandboxDefaults"}))["plugins"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    // Ids compare case-insensitively.
    let l = set(&mut c, "Vst3", "abcdef0123", None, false);
    assert_eq!(
        l,
        json!([{"format": "Clap", "plugin_id": "com.acme.synth", "name": "com.acme.synth"}])
    );
    c.quit();
}
