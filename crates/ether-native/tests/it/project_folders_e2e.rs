//! Native e2e of readable project folders (`project_dirs`): bare-UUID folders of older
//! versions get readable names at startup without losing a file, and a project's folder
//! follows its title once it isn't open.

use crate::common;

use common::{Client, Paths};
use ether_native::project_dirs::{self, folder_name};
use serde_json::json;

#[test]
fn folders_get_readable_names_and_follow_titles() {
    let tmp = ether_native::test_util::TempDir::new("e2e-project-folders");
    let paths = Paths::new(tmp.path());
    let root = paths.projects_root.clone();

    // A project as an older version left it: named by its id.
    let mut c = Client::start(&paths);
    let old: String = c.project_id();
    let old_id = old.parse().unwrap();
    c.ok(
        "Project",
        json!({"type": "Create", "id": old, "name": "Old: Song"}),
    );
    let new_id = c.project_id();
    c.ok(
        "Project",
        json!({"type": "Create", "id": new_id, "name": "Next"}),
    );
    c.quit();
    let settled = root.join(folder_name("Old: Song", old_id));
    assert!(settled.is_dir(), "named after its title once created");
    std::fs::write(settled.join("media/take.wav"), b"RIFF").unwrap();
    let bare = root.join(&old);
    std::fs::rename(&settled, &bare).unwrap();

    // Startup migrates it; it opens and keeps its files.
    let mut c = Client::start(&paths);
    assert!(!bare.exists());
    assert!(settled.is_dir());
    assert_eq!(
        std::fs::read(settled.join("media/take.wav")).unwrap(),
        b"RIFF"
    );
    let opened = c.ok("Project", json!({"type": "Open", "id": old}));
    assert_eq!(opened["project"]["settings"]["name"], "Old: Song");

    // Renamed while open: the folder stays until the project closes.
    c.ok(
        "Project",
        json!({"type": "Rename", "id": old, "name": "Final Mix"}),
    );
    c.ok("Project", json!({"type": "Save"}));
    assert!(settled.is_dir(), "never renamed while open");
    c.ok("Project", json!({"type": "Open", "id": new_id}));
    let renamed = root.join(folder_name("Final Mix", old_id));
    assert!(renamed.is_dir(), "follows its title once closed");
    assert!(!settled.exists());
    assert_eq!(
        std::fs::read(renamed.join("media/take.wav")).unwrap(),
        b"RIFF"
    );
    assert_eq!(project_dirs::dir(&root, old_id), renamed);
    let listed = c.ok("Project", json!({"type": "List"}));
    let names: Vec<&str> = listed["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"Final Mix") && names.contains(&"Next"),
        "{names:?}"
    );
    c.ok("Project", json!({"type": "Open", "id": old}));
    c.quit();
}
