#!/usr/bin/env node
// Writes the release-time Tauri config overlay (merged over apps/desktop/src-tauri/
// tauri.conf.json by `tauri build --config <file>`) and prints its path.
//
//   node scripts/release/tauri-config.mjs <out.json>
//
// The overlay sets:
//   - `version` from the VERSION file (the single source of the release version);
//   - signing, from the environment:
//       macOS: hardened-runtime entitlements (scripts/release/macos/entitlements.plist).
//              APPLE_SIGNING_IDENTITY set → Tauri signs with it (and notarizes when
//              APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID are set; Tauri reads those itself),
//              beforeBuildCommand also signs the staged sandbox helper (sign-macos-helper.mjs,
//              same identity or ad-hoc), which Tauri does not sign;
//              unset → ad-hoc signature ("-") so the unsigned app still runs on Apple Silicon.
//       Windows: `bundle.externalBin: ["binaries/ether-mcp"]`, the MCP server installed next
//              to Ethereal.exe (macOS/Linux ship it through tauri.conf.json `files`; Windows
//              has no `files`, and Tauri signs sidecars with the app's certificate);
//              WINDOWS_CERTIFICATE_THUMBPRINT set (the workflow imports the .pfx and
//              exports it) → `bundle.windows.certificateThumbprint` + timestamping.
//   - auto-update: TAURI_SIGNING_PRIVATE_KEY set → `bundle.createUpdaterArtifacts: true`
//     (Tauri signs the updater bundles with it: .app.tar.gz, AppImage, setup.exe, msi);
//     unset → no updater artifacts (Tauri would fail the build without the key) and a notice.
// It also prints `signed=true|false` and `updater=true|false` to $GITHUB_OUTPUT when that
// variable is set.
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const out = process.argv[2];
if (!out) {
  console.error("usage: tauri-config.mjs <out.json>");
  process.exit(2);
}

function releaseVersion() {
  // The workflow passes the computed release version (never a duplicate); VERSION otherwise.
  const v = (process.env.ETHER_RELEASE_VERSION || readFileSync(join(root, "VERSION"), "utf8")).trim();
  // MSI/WiX and CFBundleVersion need plain numeric X.Y.Z.
  if (!/^\d+\.\d+\.\d+$/.test(v)) throw new Error(`VERSION must be X.Y.Z, got '${v}'`);
  return v;
}

const env = process.env;
const platform = env.RUNNER_OS ?? { darwin: "macOS", linux: "Linux", win32: "Windows" }[process.platform];
// Paths in `bundle.resources` are relative to apps/desktop/src-tauri.
const config = {
  version: releaseVersion(),
  bundle: {},
};

let signed = false;
if (platform === "macOS") {
  signed = Boolean(env.APPLE_SIGNING_IDENTITY);
  config.bundle.macOS = {
    signingIdentity: signed ? env.APPLE_SIGNING_IDENTITY : "-",
    entitlements: "../../../scripts/release/macos/entitlements.plist",
  };
  const base = JSON.parse(readFileSync(join(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8"));
  const before = base.build?.beforeBuildCommand;
  const sign = "pnpm --workspace-root exec node scripts/release/sign-macos-helper.mjs";
  config.build = { beforeBuildCommand: before ? `${before} && ${sign}` : sign };
} else if (platform === "Windows") {
  // Only here, not in tauri.conf.json: tauri-build requires every externalBin to exist, which
  // would break plain `cargo build` of the desktop crate (build-sandbox-helper.mjs --bundle
  // stages binaries/ether-mcp-<triple>.exe in beforeBuildCommand).
  config.bundle.externalBin = ["binaries/ether-mcp"];
}
if (platform === "Windows" && env.WINDOWS_CERTIFICATE_THUMBPRINT) {
  signed = true;
  config.bundle.windows = {
    certificateThumbprint: env.WINDOWS_CERTIFICATE_THUMBPRINT,
    digestAlgorithm: "sha256",
    timestampUrl: env.WINDOWS_TIMESTAMP_URL || "http://timestamp.digicert.com",
  };
}

const updater = Boolean(env.TAURI_SIGNING_PRIVATE_KEY);
if (updater) config.bundle.createUpdaterArtifacts = true;
else console.log("::notice::TAURI_SIGNING_PRIVATE_KEY is not set: building without updater artifacts (no auto-update for this release)");

writeFileSync(out, `${JSON.stringify(config, null, 2)}\n`);
if (env.GITHUB_OUTPUT) appendFileSync(env.GITHUB_OUTPUT, `signed=${signed}\nupdater=${updater}\n`);
console.log(out);
