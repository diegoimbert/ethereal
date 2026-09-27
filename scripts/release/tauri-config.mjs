#!/usr/bin/env node
// Writes the release-time Tauri config overlay (merged over apps/desktop/src-tauri/
// tauri.conf.json by `tauri build --config <file>`) and prints its path.
//
//   node scripts/release/tauri-config.mjs <out.json>
//
// The overlay sets:
//   - `version` from the VERSION file (the single source of the release version);
//   - `bundle.resources`: THIRD_PARTY_NOTICES.txt and LICENSE, shipped inside the app;
//   - signing, from the environment:
//       macOS: APPLE_SIGNING_IDENTITY set → Tauri signs with it (and notarizes when
//              APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID are set; Tauri reads those itself);
//              unset → ad-hoc signature ("-") so the unsigned app still runs on Apple Silicon.
//       Windows: WINDOWS_CERTIFICATE_THUMBPRINT set (the workflow imports the .pfx and
//              exports it) → `bundle.windows.certificateThumbprint` + timestamping.
// It also prints `signed=true|false` to $GITHUB_OUTPUT when that variable is set.
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
  const v = readFileSync(join(root, "VERSION"), "utf8").trim();
  // MSI/WiX and CFBundleVersion need plain numeric X.Y.Z.
  if (!/^\d+\.\d+\.\d+$/.test(v)) throw new Error(`VERSION must be X.Y.Z, got '${v}'`);
  return v;
}

const env = process.env;
const platform = env.RUNNER_OS ?? { darwin: "macOS", linux: "Linux", win32: "Windows" }[process.platform];
// Paths in `bundle.resources` are relative to apps/desktop/src-tauri.
const config = {
  version: releaseVersion(),
  bundle: {
    resources: {
      "../../../THIRD_PARTY_NOTICES.txt": "THIRD_PARTY_NOTICES.txt",
      "../../../LICENSE": "LICENSE.txt",
    },
  },
};

let signed = false;
if (platform === "macOS") {
  signed = Boolean(env.APPLE_SIGNING_IDENTITY);
  config.bundle.macOS = { signingIdentity: signed ? env.APPLE_SIGNING_IDENTITY : "-" };
} else if (platform === "Windows" && env.WINDOWS_CERTIFICATE_THUMBPRINT) {
  signed = true;
  config.bundle.windows = {
    certificateThumbprint: env.WINDOWS_CERTIFICATE_THUMBPRINT,
    digestAlgorithm: "sha256",
    timestampUrl: env.WINDOWS_TIMESTAMP_URL || "http://timestamp.digicert.com",
  };
}

writeFileSync(out, `${JSON.stringify(config, null, 2)}\n`);
if (env.GITHUB_OUTPUT) appendFileSync(env.GITHUB_OUTPUT, `signed=${signed}\n`);
console.log(out);
