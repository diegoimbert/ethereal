// node --test scripts/release/ (part of `just test-all`).
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(new URL("./tauri-config.mjs", import.meta.url));
const baseConf = JSON.parse(readFileSync(fileURLToPath(new URL("../../apps/desktop/src-tauri/tauri.conf.json", import.meta.url)), "utf8"));

function overlay(runnerOs, extraEnv = {}) {
  const out = join(mkdtempSync(join(tmpdir(), "ether-tauri-config-")), "overlay.json");
  const env = { PATH: process.env.PATH, RUNNER_OS: runnerOs, ETHER_RELEASE_VERSION: "1.2.3", ...extraEnv };
  execFileSync(process.execPath, [script, out], { env, stdio: "pipe" });
  return JSON.parse(readFileSync(out, "utf8"));
}

// The release ships ether-mcp next to the app binary on every platform.
test("macOS and Linux bundles install ether-mcp next to the app (tauri.conf.json files)", () => {
  assert.equal(baseConf.bundle.macOS.files["MacOS/ether-mcp"], "binaries/ether-mcp");
  for (const fmt of ["deb", "rpm", "appimage"]) assert.equal(baseConf.bundle.linux[fmt].files["/usr/bin/ether-mcp"], "binaries/ether-mcp", fmt);
  // Not a sidecar in the base config: tauri-build would require the staged binary for every
  // plain `cargo build` of the desktop crate.
  assert.equal(baseConf.bundle.externalBin, undefined);
  assert.match(baseConf.build.beforeBuildCommand, /build-sandbox-helper\.mjs --bundle/);
});

test("Windows ships ether-mcp as a sidecar (signed with the app), only in the release overlay", () => {
  assert.deepEqual(overlay("Windows").bundle.externalBin, ["binaries/ether-mcp"]);
  const signed = overlay("Windows", { WINDOWS_CERTIFICATE_THUMBPRINT: "ABC" });
  assert.deepEqual(signed.bundle.externalBin, ["binaries/ether-mcp"]);
  assert.equal(signed.bundle.windows.certificateThumbprint, "ABC");
  assert.equal(overlay("Linux").bundle.externalBin, undefined);
});

test("macOS signs the staged helpers, ether-mcp included", () => {
  const conf = overlay("macOS");
  assert.match(conf.build.beforeBuildCommand, /sign-macos-helper\.mjs$/);
  const sign = readFileSync(fileURLToPath(new URL("./sign-macos-helper.mjs", import.meta.url)), "utf8");
  assert.match(sign, /"ether-mcp"/);
});

test("the base config's version is VERSION (a local `tauri build` is labeled like the release)", () => {
  const version = readFileSync(fileURLToPath(new URL("../../VERSION", import.meta.url)), "utf8").trim();
  assert.equal(baseConf.version, version, "bump apps/desktop/src-tauri/tauri.conf.json's version with VERSION");
});
