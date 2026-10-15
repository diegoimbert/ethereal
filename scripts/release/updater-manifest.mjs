#!/usr/bin/env node
// Writes the updater manifest (`latest.json`) that the desktop app's updater reads from
// https://github.com/<repo>/releases/latest/download/latest.json (tauri-plugin-updater's
// static JSON format; docs/RELEASING.md "Auto-update"). <repo> is the public releases
// Builds `latest.json` for the private overlay's self-updater (the OSS build has no updater;
// see `ethereal-private`). Download URLs point at this repository's releases.
//
//   node scripts/release/updater-manifest.mjs --assets <dir> --version <X.Y.Z> --tag <tag>
//        --repo <owner/name> [--notes-file <file>] [--pub-date <RFC 3339>] --out <latest.json>
//
// <dir> holds the release assets. Each platform's updater bundle must sit next to its
// minisign signature (`<file>.sig`, written by `tauri build` when TAURI_SIGNING_PRIVATE_KEY
// is set and `bundle.createUpdaterArtifacts` is on):
//
//   darwin-aarch64       Ethereal_<v>_aarch64.app.tar.gz   (collect.mjs renames Tauri's Ethereal.app.tar.gz)
//   darwin-x86_64        Ethereal_<v>_x86_64.app.tar.gz
//   linux-x86_64         Ethereal_<v>_amd64.AppImage        (Tauri 2 signs the AppImage itself)
//   windows-x86_64       Ethereal_<v>_x64-setup.exe          (NSIS: the preferred update path)
//   windows-x86_64-msi   Ethereal_<v>_x64_en-US.msi          (only for copies installed from the MSI)
//
// The updater looks up `<os>-<arch>-<installer>` first, then `<os>-<arch>`, so `.deb`
// installs fall back to the AppImage entry; the app itself never installs that over a
// system package (it links to the release page instead).
//
// A platform without its bundle or signature is left out with a warning; with none at all
// nothing is written (exit 0): a release never fails over the updater.
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

/** Updater platform keys, in manifest order, and how to recognize their bundles. */
export const PLATFORMS = [
  { key: "darwin-aarch64", label: "macOS (Apple Silicon)", match: /_aarch64\.app\.tar\.gz$/ },
  { key: "darwin-x86_64", label: "macOS (Intel)", match: /_(x86_64|x64)\.app\.tar\.gz$/ },
  { key: "linux-x86_64", label: "Linux AppImage", match: /_amd64\.AppImage$/ },
  { key: "windows-x86_64", label: "Windows (NSIS)", match: /_x64-setup\.exe$/ },
  { key: "windows-x86_64-msi", label: "Windows (MSI)", match: /_x64_[A-Za-z-]+\.msi$/, optional: true },
];

/** Release notes for the app: the release body without the download and signing sections. */
export function notesForApp(body) {
  if (!body) return "";
  const out = [];
  let skipping = false;
  for (const line of body.replace(/\r\n/g, "\n").split("\n")) {
    const h = /^(#{1,6})\s+(.*)$/.exec(line);
    if (h) {
      const level = h[1].length;
      const title = h[2].trim().toLowerCase();
      // "## Ethereal v0.3.1" heading: the app shows the version itself.
      if (level <= 2 && /^ethereal v?\d/.test(title)) {
        skipping = false;
        continue;
      }
      skipping = level >= 3 && (title === "downloads" || title === "code signing");
      if (skipping) continue;
    }
    if (!skipping) out.push(line);
  }
  return out.join("\n").trim();
}

/**
 * Build the manifest from a list of asset file names (`files`) and their signatures
 * (`readSig(name)` → the `.sig` file's text, or `null` if it is missing).
 * Returns `{ manifest | null, warnings: string[] }`.
 */
export function buildManifest({ files, readSig, version, tag, repo, notes = "", pubDate }) {
  if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) throw new Error(`version must be X.Y.Z, got '${version}'`);
  if (!/^[\w.-]+\/[\w.-]+$/.test(repo)) throw new Error(`repo must be owner/name, got '${repo}'`);
  const warnings = [];
  const platforms = {};
  const names = new Set(files);
  for (const p of PLATFORMS) {
    const candidates = files.filter((f) => p.match.test(f)).sort();
    if (candidates.length === 0) {
      if (!p.optional) warnings.push(`${p.key} (${p.label}): no updater bundle; that platform won't self-update`);
      continue;
    }
    if (candidates.length > 1) warnings.push(`${p.key}: several bundles (${candidates.join(", ")}); using ${candidates[0]}`);
    const file = candidates[0];
    const sig = names.has(`${file}.sig`) ? readSig(`${file}.sig`) : null;
    const signature = sig?.trim();
    if (!signature) {
      warnings.push(`${p.key} (${p.label}): ${file} has no signature (${file}.sig); left out`);
      continue;
    }
    platforms[p.key] = {
      signature,
      url: `https://github.com/${repo}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(file)}`,
    };
  }
  // The NSIS entry also answers `windows-x86_64-nsis` lookups (the default key does already;
  // explicit is clearer for copies installed from the setup.exe).
  if (platforms["windows-x86_64"]) platforms["windows-x86_64-nsis"] = platforms["windows-x86_64"];
  if (platforms["linux-x86_64"]) platforms["linux-x86_64-appimage"] = platforms["linux-x86_64"];
  if (Object.keys(platforms).length === 0) return { manifest: null, warnings };
  const manifest = { version, notes: notesForApp(notes), pub_date: pubDate ?? new Date().toISOString(), platforms };
  return { manifest, warnings };
}

function parseArgs(argv) {
  const args = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) throw new Error(`unexpected argument ${a}`);
    args[a.slice(2)] = argv[++i];
  }
  return args;
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  for (const k of ["assets", "version", "tag", "repo", "out"]) {
    if (!args[k]) {
      console.error(
        "usage: updater-manifest.mjs --assets <dir> --version <X.Y.Z> --tag <tag> --repo <owner/name> [--notes-file f] [--pub-date d] --out <file>",
      );
      process.exit(2);
    }
  }
  const dir = args.assets;
  if (!existsSync(dir)) throw new Error(`${dir} does not exist`);
  const files = readdirSync(dir);
  const notes = args["notes-file"] && existsSync(args["notes-file"]) ? readFileSync(args["notes-file"], "utf8") : "";
  const { manifest, warnings } = buildManifest({
    files,
    readSig: (name) => readFileSync(join(dir, name), "utf8"),
    version: args.version,
    tag: args.tag,
    repo: args.repo,
    notes,
    pubDate: args["pub-date"],
  });
  for (const w of warnings) console.log(`::warning::updater: ${w}`);
  if (!manifest) {
    console.log("::notice::updater: no signed updater bundles; latest.json not written (the apps won't see this release as an update)");
    return;
  }
  writeFileSync(args.out, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`updater: wrote ${args.out} (${Object.keys(manifest.platforms).join(", ")})`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    main();
  } catch (e) {
    // Never fail the release over the updater manifest.
    console.log(`::warning::updater: latest.json not written: ${e instanceof Error ? e.message : e}`);
  }
}
