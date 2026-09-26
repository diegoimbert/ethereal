use std::path::Path;

fn main() {
    // `tauri.conf.json` points `frontendDist` at the UI build output (`ui/dist`), and
    // `tauri::generate_context!` fails to compile if that directory does not exist.
    // So that `cargo check`/`clippy` work on a fresh checkout (before `pnpm build`), we
    // create a placeholder `index.html` there when the directory is missing. `ui/dist` is
    // gitignored and gets replaced by the real build (Vite empties it first), and
    // `tauri build` runs `beforeBuildCommand` which builds the UI before this script runs.
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../ui/dist");
    if !dist.join("index.html").exists() {
        std::fs::create_dir_all(&dist).expect("create ui/dist placeholder dir");
        std::fs::write(
            dist.join("index.html"),
            "<!doctype html><meta charset=\"utf-8\"><title>Ethereal</title>\
             <p>UI not built. Run <code>pnpm --filter @ethereal/ui build</code> \
             or use the dev server.</p>\n",
        )
        .expect("write ui/dist placeholder index.html");
    }

    tauri_build::build();
}
