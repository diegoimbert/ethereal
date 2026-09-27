#!/usr/bin/env node
// Signs the staged sandbox helper (apps/desktop/src-tauri/binaries/ether-sandbox-helper)
// with hardened runtime and the release entitlements: with APPLE_SIGNING_IDENTITY (Developer
// ID, plus a secure timestamp, as notarization requires) or ad-hoc ("-") without it. Tauri
// signs sidecars (`externalBin`) but not plain `bundle.macOS.files`, which is how the helper
// is bundled, and codesign refuses to sign the .app around a nested binary with no
// signature at all (x86_64 binaries are not linker-signed, unlike arm64).
//
// Run by the release overlay's beforeBuildCommand (scripts/release/tauri-config.mjs) after
// the helper is staged, on macOS. A real identity must already be in a keychain on the
// search list (the release workflow imports it).
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const identity = process.env.APPLE_SIGNING_IDENTITY || "-";
const helper = join(root, "apps/desktop/src-tauri/binaries/ether-sandbox-helper");

if (process.platform !== "darwin") {
  console.log("sign-macos-helper: not macOS; nothing to sign");
  process.exit(0);
}
if (!existsSync(helper)) {
  console.error(`sign-macos-helper: ${helper} not found (did build-sandbox-helper.mjs --bundle run?)`);
  process.exit(1);
}
const entitlements = join(root, "scripts/release/macos/entitlements.plist");
const args = ["--force", "--options", "runtime", "--entitlements", entitlements, "--sign", identity, helper];
if (identity !== "-") args.splice(1, 0, "--timestamp");
execFileSync("codesign", args, { stdio: "inherit" });
execFileSync("codesign", ["--verify", "--strict", "--verbose=2", helper], { stdio: "inherit" });
console.log(`sign-macos-helper: signed ${helper} (${identity === "-" ? "ad-hoc" : identity})`);
