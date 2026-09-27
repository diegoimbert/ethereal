#!/usr/bin/env node
// Builds the desktop app's plugin helper binaries: `ether-plugin-scanner` (out-of-process
// plugin scanning, found by ether_plugin_host::ScanRunner::locate next to the app binary)
// and, on macOS/Linux, `ether-sandbox-helper` (the out-of-process plugin host).
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
// and `ETHER_PLUGIN_SCANNER` the scanner path at runtime. The sandbox runs on macOS and Linux
// only; elsewhere only the scanner is built.
import { spawnSync } from "node:child_process";
import { copyFileSync, chmodSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const bundle = process.argv.includes("--bundle");

const sandbox = process.platform === "darwin" || process.platform === "linux";
const exe = process.platform === "win32" ? ".exe" : "";
const bins = [["ether-plugin-scanner", "ether-plugin-scanner"]];
if (sandbox) bins.push(["ether-sandbox", "ether-sandbox-helper"]);

const triple = bundle ? process.env.TAURI_ENV_TARGET_TRIPLE : undefined;
const release = bundle && process.env.TAURI_ENV_DEBUG !== "true";
const args = ["build", ...bins.flatMap(([pkg, bin]) => ["-p", pkg, "--bin", bin])];
if (release) args.push("--release");
if (triple) args.push("--target", triple);

const cargo = process.env.CARGO ?? "cargo";
const r = spawnSync(cargo, args, { cwd: root, stdio: "inherit" });
if (r.status !== 0) {
  console.error("plugin helpers: cargo build failed");
  process.exit(r.status ?? 1);
}

const targetDir = process.env.CARGO_TARGET_DIR ? resolve(root, process.env.CARGO_TARGET_DIR) : join(root, "target");
for (const [, bin] of bins) {
  const built = join(targetDir, ...(triple ? [triple] : []), release ? "release" : "debug", bin + exe);
  if (bundle) {
    const dest = join(root, "apps/desktop/src-tauri/binaries", bin + exe);
    mkdirSync(dirname(dest), { recursive: true });
    copyFileSync(built, dest);
    chmodSync(dest, 0o755);
    console.log(`${bin}: ${dest}`);
  } else {
    console.log(`${bin}: ${built}`);
  }
}
if (!sandbox) console.log("ether-sandbox-helper: sandboxed plugins are not supported on this platform; skipped");
