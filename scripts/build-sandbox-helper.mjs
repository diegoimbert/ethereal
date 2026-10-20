#!/usr/bin/env node
// Builds the desktop app's plugin helper binaries: `ether-plugin-scanner` (out-of-process
// plugin scanning, found by ether_plugin_host::ScanRunner::locate next to the app binary)
// and, on macOS/Linux, `ether-sandbox-helper` (the out-of-process plugin host). With
// `--bundle` it also builds `ether-mcp` (the MCP server for AI agents, docs/MCP.md), which the
// release ships next to the app binary.
//
//   node scripts/build-sandbox-helper.mjs            dev: debug build into target/debug,
//                                                    next to target/debug/ether-desktop,
//                                                    where ether_sandbox::helper_path()
//                                                    finds it (`just dev-desktop`)
//   node scripts/build-sandbox-helper.mjs --bundle   release build, copied to
//                                                    apps/desktop/src-tauri/binaries/ for
//                                                    the bundler (tauri.conf.json
//                                                    `bundle.macOS.files` / linux `files`:
//                                                    installed next to the app binary;
//                                                    on Windows `ether-mcp` is a sidecar,
//                                                    staged as ether-mcp-<triple>.exe for
//                                                    the release overlay's `externalBin`,
//                                                    scripts/release/tauri-config.mjs)
//
// A product with binaries of its own (docs/EXTENSIONS.md) builds them from its workspace:
//   --workspace <dir>       the cargo workspace (default: this repository)
//   --dest <dir>            where `--bundle` stages them (default: apps/desktop/src-tauri/binaries)
//   --scanner <package>     the scanner's package (default: ether-plugin-scanner)
//   --helper <package>      the sandbox helper's package (default: ether-sandbox)
//   --mcp <package>         the MCP server's package (default: ether-mcp)
//   --mcp-features <list>   its features for `--bundle` (default: ether-mcp/full)
//
// With `--bundle` it follows the Tauri CLI's `TAURI_ENV_TARGET_TRIPLE` / `TAURI_ENV_DEBUG`
// (set for `beforeBuildCommand`). `ETHER_SANDBOX_HELPER` still overrides the helper path
// and `ETHER_PLUGIN_SCANNER` the scanner path at runtime. The sandbox runs on macOS and Linux
// only; elsewhere only the scanner is built.
import { spawnSync } from "node:child_process";
import { copyFileSync, chmodSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const bundle = process.argv.includes("--bundle");
const arg = (name, fallback) => {
  const i = process.argv.indexOf(name);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
};
const root = resolve(arg("--workspace", repo));
const dest = resolve(arg("--dest", join(repo, "apps/desktop/src-tauri/binaries")));
const scanner = arg("--scanner", "ether-plugin-scanner");
const helper = arg("--helper", "ether-sandbox");
const mcp = arg("--mcp", "ether-mcp");
const mcpFeatures = arg("--mcp-features", "ether-mcp/full");

const sandbox = process.platform === "darwin" || process.platform === "linux";
const exe = process.platform === "win32" ? ".exe" : "";
const bins = [[scanner, "ether-plugin-scanner"]];
if (sandbox) bins.push([helper, "ether-sandbox-helper"]);
// The MCP server is only shipped, never needed by `just dev-desktop` (`just mcp` runs it).
// `full`: its headless `--project` mode embeds the same engine features as the release app.
if (bundle) bins.push([mcp, "ether-mcp"]);

const triple = bundle ? process.env.TAURI_ENV_TARGET_TRIPLE : undefined;
const release = bundle && process.env.TAURI_ENV_DEBUG !== "true";
const args = ["build", ...bins.flatMap(([pkg, bin]) => ["-p", pkg, "--bin", bin])];
if (bundle) args.push("--features", mcpFeatures);
if (release) args.push("--release");
if (triple) args.push("--target", triple);

const cargo = process.env.CARGO ?? "cargo";
/** Tauri's sidecar suffix: the target triple (the host's when the build has no `--target`). */
function sidecarTriple() {
  if (triple) return triple;
  const v = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
  const host = /^host: (\S+)$/m.exec(v.stdout ?? "")?.[1];
  if (!host) {
    console.error("plugin helpers: cannot read the host triple from `rustc -vV`");
    process.exit(1);
  }
  return host;
}
const r = spawnSync(cargo, args, { cwd: root, stdio: "inherit" });
if (r.status !== 0) {
  console.error("plugin helpers: cargo build failed");
  process.exit(r.status ?? 1);
}

const targetDir = process.env.CARGO_TARGET_DIR ? resolve(root, process.env.CARGO_TARGET_DIR) : join(root, "target");
for (const [, bin] of bins) {
  const built = join(targetDir, ...(triple ? [triple] : []), release ? "release" : "debug", bin + exe);
  if (bundle) {
    const staged = join(dest, bin + exe);
    mkdirSync(dirname(staged), { recursive: true });
    copyFileSync(built, staged);
    chmodSync(staged, 0o755);
    console.log(`${bin}: ${staged}`);
    if (bin === "ether-mcp" && process.platform === "win32") {
      // `bundle.externalBin: ["binaries/ether-mcp"]` (release overlay) wants the triple suffix.
      const sidecar = join(dest, `${bin}-${sidecarTriple()}${exe}`);
      copyFileSync(built, sidecar);
      console.log(`${bin}: ${sidecar}`);
    }
  } else {
    console.log(`${bin}: ${built}`);
  }
}
if (!sandbox) console.log("ether-sandbox-helper: sandboxed plugins are not supported on this platform; skipped");
