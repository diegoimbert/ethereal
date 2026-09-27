#!/usr/bin/env node
// Builds `ether-sandbox-helper` (the out-of-process plugin host) for the desktop app.
//
//   node scripts/build-sandbox-helper.mjs            dev: debug build into target/debug,
//                                                    next to target/debug/ether-desktop,
//                                                    where ether_sandbox::helper_path()
//                                                    finds it (`just dev-desktop`)
//   node scripts/build-sandbox-helper.mjs --bundle   release build, copied to
//                                                    apps/desktop/src-tauri/binaries/ for
//                                                    the bundler (tauri.conf.json
//                                                    `bundle.macOS.files` / linux `files`:
//                                                    installed next to the app binary)
//
// With `--bundle` it follows the Tauri CLI's `TAURI_ENV_TARGET_TRIPLE` / `TAURI_ENV_DEBUG`
// (set for `beforeBuildCommand`). `ETHER_SANDBOX_HELPER` still overrides the helper path
// at runtime. The sandbox runs on macOS and Linux only; elsewhere this is a no-op.
import { spawnSync } from "node:child_process";
import { copyFileSync, chmodSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const bundle = process.argv.includes("--bundle");

if (process.platform !== "darwin" && process.platform !== "linux") {
  console.log("ether-sandbox-helper: sandboxed plugins are not supported on this platform; skipping");
  process.exit(0);
}

const triple = bundle ? process.env.TAURI_ENV_TARGET_TRIPLE : undefined;
const release = bundle && process.env.TAURI_ENV_DEBUG !== "true";
const args = ["build", "-p", "ether-sandbox", "--bin", "ether-sandbox-helper"];
if (release) args.push("--release");
if (triple) args.push("--target", triple);

const cargo = process.env.CARGO ?? "cargo";
const r = spawnSync(cargo, args, { cwd: root, stdio: "inherit" });
if (r.status !== 0) {
  console.error("ether-sandbox-helper: cargo build failed");
  process.exit(r.status ?? 1);
}

const targetDir = process.env.CARGO_TARGET_DIR ? resolve(root, process.env.CARGO_TARGET_DIR) : join(root, "target");
const built = join(targetDir, ...(triple ? [triple] : []), release ? "release" : "debug", "ether-sandbox-helper");
if (bundle) {
  const dest = join(root, "apps/desktop/src-tauri/binaries/ether-sandbox-helper");
  mkdirSync(dirname(dest), { recursive: true });
  copyFileSync(built, dest);
  chmodSync(dest, 0o755);
  console.log(`ether-sandbox-helper: ${dest}`);
} else {
  console.log(`ether-sandbox-helper: ${built}`);
}
