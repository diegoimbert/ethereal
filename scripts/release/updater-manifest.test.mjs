// node --test scripts/release/ (part of `just test-all`).
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { buildManifest, notesForApp } from "./updater-manifest.mjs";

const V = "0.3.1";
const ALL = [
  `Ethereal_${V}_aarch64.dmg`,
  `Ethereal_${V}_aarch64.app.zip`,
  `Ethereal_${V}_aarch64.app.tar.gz`,
  `Ethereal_${V}_aarch64.app.tar.gz.sig`,
  `Ethereal_${V}_x64.dmg`,
  `Ethereal_${V}_x86_64.app.tar.gz`,
  `Ethereal_${V}_x86_64.app.tar.gz.sig`,
  `Ethereal_${V}_amd64.AppImage`,
  `Ethereal_${V}_amd64.AppImage.sig`,
  `Ethereal_${V}_amd64.deb`,
  `Ethereal_${V}_x64-setup.exe`,
  `Ethereal_${V}_x64-setup.exe.sig`,
  `Ethereal_${V}_x64_en-US.msi`,
  `Ethereal_${V}_x64_en-US.msi.sig`,
  `Ethereal_${V}_web.zip`,
  "SHA256SUMS.txt",
];
const sig = (name) => `sig-of:${name}\n`;
const base = { version: V, tag: `v${V}`, repo: "diegoimbert/ethereal", pubDate: "2026-10-04T00:00:00Z" };

test("every platform, with urls and signatures", () => {
  const { manifest, warnings } = buildManifest({ ...base, files: ALL, readSig: sig, notes: "hello" });
  assert.deepEqual(warnings, []);
  assert.equal(manifest.version, V);
  assert.equal(manifest.pub_date, "2026-10-04T00:00:00Z");
  assert.equal(manifest.notes, "hello");
  assert.deepEqual(Object.keys(manifest.platforms).sort(), [
    "darwin-aarch64",
    "darwin-x86_64",
    "linux-x86_64",
    "linux-x86_64-appimage",
    "windows-x86_64",
    "windows-x86_64-msi",
    "windows-x86_64-nsis",
  ]);
  const mac = manifest.platforms["darwin-aarch64"];
  assert.equal(mac.url, `https://github.com/diegoimbert/ethereal/releases/download/v${V}/Ethereal_${V}_aarch64.app.tar.gz`);
  // The signature is the .sig file's content, trimmed.
  assert.equal(mac.signature, `sig-of:Ethereal_${V}_aarch64.app.tar.gz.sig`);
  assert.match(manifest.platforms["linux-x86_64"].url, /_amd64\.AppImage$/);
  // Windows prefers NSIS; the MSI only answers MSI installs.
  assert.match(manifest.platforms["windows-x86_64"].url, /-setup\.exe$/);
  assert.match(manifest.platforms["windows-x86_64-msi"].url, /\.msi$/);
});

test("a missing platform is left out with a warning", () => {
  const files = ALL.filter((f) => !f.includes("x86_64.app.tar.gz"));
  const { manifest, warnings } = buildManifest({ ...base, files, readSig: sig });
  assert.equal(manifest.platforms["darwin-x86_64"], undefined);
  assert.ok(manifest.platforms["darwin-aarch64"]);
  assert.equal(warnings.length, 1);
  assert.match(warnings[0], /darwin-x86_64/);
});

test("a bundle without its signature is left out with a warning", () => {
  const files = ALL.filter((f) => f !== `Ethereal_${V}_amd64.AppImage.sig`);
  const { manifest, warnings } = buildManifest({ ...base, files, readSig: sig });
  assert.equal(manifest.platforms["linux-x86_64"], undefined);
  assert.equal(manifest.platforms["linux-x86_64-appimage"], undefined);
  assert.match(warnings.join("\n"), /linux-x86_64.*no signature/);
  // An empty .sig counts as missing.
  const empty = buildManifest({ ...base, files: ALL, readSig: (n) => (n.endsWith(".exe.sig") ? "  \n" : sig(n)) });
  assert.equal(empty.manifest.platforms["windows-x86_64"], undefined);
  assert.equal(empty.manifest.platforms["windows-x86_64-nsis"], undefined);
  assert.ok(empty.manifest.platforms["windows-x86_64-msi"]);
});

test("no signed bundles at all: no manifest (an unsigned release)", () => {
  const files = ALL.filter((f) => !f.endsWith(".sig"));
  const { manifest, warnings } = buildManifest({ ...base, files, readSig: sig });
  assert.equal(manifest, null);
  // The four required platforms, plus the MSI that is there but unsigned.
  assert.equal(warnings.length, 5);
});

test("a missing MSI is not worth a warning", () => {
  const files = ALL.filter((f) => !f.includes(".msi"));
  const { manifest, warnings } = buildManifest({ ...base, files, readSig: sig });
  assert.deepEqual(warnings, []);
  assert.equal(manifest.platforms["windows-x86_64-msi"], undefined);
});

test("tags and file names are url-encoded", () => {
  const { manifest } = buildManifest({ ...base, tag: "v0.3.1+abc123", files: ALL, readSig: sig });
  assert.match(manifest.platforms["linux-x86_64"].url, /\/download\/v0\.3\.1%2Babc123\//);
});

test("bad version or repo throws", () => {
  assert.throws(() => buildManifest({ ...base, version: "latest", files: ALL, readSig: sig }));
  assert.throws(() => buildManifest({ ...base, repo: "no slash", files: ALL, readSig: sig }));
});

test("notes drop the download and signing sections", () => {
  const body = [
    "## Ethereal v0.3.1",
    "",
    "### Downloads",
    "- **macOS**: dmg",
    "",
    "### Code signing",
    "- macOS: unsigned",
    "",
    "Unsigned builds trigger OS warnings.",
    "",
    "## What's Changed",
    "* Nested racks by @diego in #272",
    "",
    "**Full Changelog**: v0.3.0...v0.3.1",
  ].join("\n");
  assert.equal(notesForApp(body), "## What's Changed\n* Nested racks by @diego in #272\n\n**Full Changelog**: v0.3.0...v0.3.1");
  assert.equal(notesForApp(""), "");
  assert.equal(notesForApp(undefined), "");
  assert.equal(notesForApp("Just text"), "Just text");
});

test("CLI writes latest.json, and writes nothing (exit 0) without signatures", () => {
  const script = fileURLToPath(new URL("./updater-manifest.mjs", import.meta.url));
  const dir = mkdtempSync(join(tmpdir(), "ether-manifest-"));
  for (const f of ALL) writeFileSync(join(dir, f), f.endsWith(".sig") ? sig(f) : "x");
  writeFileSync(join(dir, "notes.md"), "## What's Changed\n* thing");
  const out = join(dir, "latest.json");
  const args = ["--assets", dir, "--version", V, "--tag", `v${V}`, "--repo", "diegoimbert/ethereal", "--notes-file", join(dir, "notes.md"), "--out", out];
  execFileSync(process.execPath, [script, ...args]);
  const json = JSON.parse(readFileSync(out, "utf8"));
  assert.equal(json.version, V);
  assert.equal(json.notes, "## What's Changed\n* thing");
  assert.ok(json.platforms["darwin-aarch64"].signature);

  const bare = mkdtempSync(join(tmpdir(), "ether-manifest-"));
  for (const f of ALL.filter((f) => !f.endsWith(".sig"))) writeFileSync(join(bare, f), "x");
  const out2 = join(bare, "latest.json");
  const log = execFileSync(process.execPath, [script, ...args.slice(0, 8).map((a) => (a === dir ? bare : a)), "--out", out2], { encoding: "utf8" });
  assert.equal(existsSync(out2), false);
  assert.match(log, /latest\.json not written/);
});
