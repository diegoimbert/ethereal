#!/usr/bin/env node
// Signs the staged helpers (apps/desktop/src-tauri/binaries/ether-sandbox-helper,
// ether-plugin-scanner and ether-mcp)
// with hardened runtime and the release entitlements: with APPLE_SIGNING_IDENTITY (Developer
// ID, plus a secure timestamp, as notarization requires) or ad-hoc ("-") without it. Tauri
// signs sidecars (`externalBin`) but not plain `bundle.macOS.files`, which is how the helper
// is bundled, and codesign refuses to sign the .app around a nested binary with no
// signature at all (x86_64 binaries are not linker-signed, unlike arm64).
//
// Each helper gets a stable code identifier under the app's bundle id
// (`dev.ethereal.app.ether-sandbox-helper`, …). Without one, codesign derives
// `<name>-<hash>` (the binaries have no Info.plist), which changes every build; macOS keys
// keychain and privacy records on it. The privacy permissions themselves (Screen Recording,
// Microphone) are asked for and used by the app process, so they show up as "Ethereal"
// (docs/RELEASING.md "macOS permissions").
//
// Run by the release overlay's beforeBuildCommand (scripts/release/tauri-config.mjs) after
// the helper is staged, on macOS. A real identity must already be in a keychain on the
// search list (the release workflow imports it).
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const bundleId = JSON.parse(readFileSync(join(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8")).identifier;
const identity = process.env.APPLE_SIGNING_IDENTITY || "-";
const helpers = ["ether-sandbox-helper", "ether-plugin-scanner", "ether-mcp"].map((b) => join(root, "apps/desktop/src-tauri/binaries", b));

if (process.platform !== "darwin") {
  console.log("sign-macos-helper: not macOS; nothing to sign");
  process.exit(0);
}
const entitlements = join(root, "scripts/release/macos/entitlements.plist");
for (const helper of helpers) {
  if (!existsSync(helper)) {
    console.error(`sign-macos-helper: ${helper} not found (did build-sandbox-helper.mjs --bundle run?)`);
    process.exit(1);
  }
  const id = `${bundleId}.${basename(helper)}`;
  const args = ["--force", "--options", "runtime", "--identifier", id, "--entitlements", entitlements, "--sign", identity, helper];
  if (identity !== "-") args.splice(1, 0, "--timestamp");
  execFileSync("codesign", args, { stdio: "inherit" });
  execFileSync("codesign", ["--verify", "--strict", "--verbose=2", helper], { stdio: "inherit" });
  console.log(`sign-macos-helper: signed ${helper} as ${id} (${identity === "-" ? "ad-hoc" : identity})`);
}
