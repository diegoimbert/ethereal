use std::path::Path;

fn main() {
    // `tauri.conf.json` points `frontendDist` at the UI build output (`ui/dist`), and
    // `tauri::generate_context!` fails to compile if that directory does not exist.
    // So that `cargo check`/`clippy` work on a fresh checkout (before `pnpm build`), we
    // create a placeholder `index.html` there when the directory is missing. `ui/dist` is
    // gitignored and gets replaced by the real build (Vite empties it first), and
    // `tauri build` runs `beforeBuildCommand` which builds the UI before this script runs.
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../ui/dist");
    // Re-run when the page goes away (`git clean`, a fresh checkout of a kept workspace): a cached
    // run of this script would not recreate the placeholder.
    println!("cargo:rerun-if-changed={}", dist.join("index.html").display());
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(&dist).expect("create ui/dist placeholder dir");
        std::fs::write(
            dist.join("index.html"),
            "<!doctype html><meta charset=\"utf-8\"><title>ethereal</title>\
             <p>UI not built. Run <code>pnpm --filter @ethereal/ui build</code> \
             or use the dev server.</p>\n",
        )
        .expect("write ui/dist placeholder index.html");
    }

    app_version();
    tauri_build::build();
}

/// `ETHER_APP_VERSION` (`crate::APP_VERSION`): the app's version, from the same place as the
/// release bundle's (`scripts/release/tauri-config.mjs`): the release workflow's
/// `ETHER_RELEASE_VERSION`, else the repo `VERSION` file. Never `tauri.conf.json`'s
/// placeholder `version`.
fn app_version() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../VERSION");
    println!("cargo:rerun-if-changed={}", file.display());
    println!("cargo:rerun-if-env-changed=ETHER_RELEASE_VERSION");
    let version = std::env::var("ETHER_RELEASE_VERSION")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| std::fs::read_to_string(&file).ok())
        .expect("read the repo VERSION file")
        .trim()
        .to_string();
    println!("cargo:rustc-env=ETHER_APP_VERSION={version}");
}
