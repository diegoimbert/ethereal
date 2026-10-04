#!/usr/bin/env node
// Generates THIRD_PARTY_NOTICES.txt: the licenses of every third-party component shipped in
// Ethereal (Rust crates from `cargo metadata`, JS packages from `pnpm licenses`), with the
// license texts found in each package. Identical texts are printed once and referenced.
//
//   node scripts/release/third-party-notices.mjs [--out <file>] [--check]
//
//   --out <file>   output path (default: THIRD_PARTY_NOTICES.txt at the repo root)
//   --check        exit 1 if the output file would change (no write)
//
// Scope: Rust = non-workspace packages reachable from the workspace members through normal
// and build dependencies, for every target platform (dev-dependencies are not shipped).
// JS = `pnpm licenses list --prod` over the whole workspace. Needs cargo + pnpm and an
// installed node_modules (`pnpm install`). No extra tools.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const args = process.argv.slice(2);
const outArg = args.indexOf("--out");
const out = outArg >= 0 ? resolve(args[outArg + 1]) : join(root, "THIRD_PARTY_NOTICES.txt");
const check = args.includes("--check");

/** Top-level files that carry license/notice text. */
const LICENSE_FILE = /^(licen[cs]e|copying|notice|copyright|unlicense)([-_.].*)?$/i;

/**
 * Vendored sources inside a package whose license lives in a subdirectory (C/C++ code
 * compiled by a build script). These are required: the script fails if they are missing.
 */
const VENDORED = {
  "signalsmith-stretch": [
    { name: "Signalsmith Stretch (C++ library, vendored)", file: "signalsmith-stretch/LICENSE.txt" },
    { name: "Signalsmith Linear (C++ library, vendored)", file: "signalsmith-linear/LICENSE.txt" },
  ],
};

/**
 * Interface definitions Ethereal's own code was written from (no code of these projects is
 * compiled in; listed for attribution). `ether-vst2`'s VST2 ABI bindings come from these
 * GPL clean-room headers, never from the Steinberg VST2 SDK.
 */
const INTERFACE_REFERENCES = [
  {
    name: "FST (Free Studio Technologies), fst/fst.h",
    license: "GPL-3.0-or-later",
    authors: "IOhannes m zmölnig, IEM",
    url: "https://git.iem.at/zmoelnig/FST",
    use: "VST 2.x ABI reference for crates/ether-vst2/src/abi.rs (struct layouts, opcodes, flags)",
  },
  {
    name: "VeSTige aeffectx.h (LMMS; also in Ardour)",
    license: "GPL-2.0-or-later",
    authors: "Javier Serrano Polo",
    url: "https://github.com/LMMS/lmms/blob/master/include/aeffectx.h",
    use: "VST 2.x ABI reference for crates/ether-vst2/src/abi.rs (host opcodes, VstMidiEvent, VstTimeInfo)",
  },
];

/** Components called out at the top of the file (the audio stretch engine). */
const HIGHLIGHT = new Set(["signalsmith-stretch"]);

function licenseFiles(dir) {
  if (!dir || !existsSync(dir)) return [];
  return readdirSync(dir)
    .filter((f) => LICENSE_FILE.test(f) && statSync(join(dir, f)).isFile())
    .sort()
    .map((f) => ({ file: f, text: readFileSync(join(dir, f), "utf8") }));
}

function run(cmd, cmdArgs) {
  return execFileSync(cmd, cmdArgs, { cwd: root, encoding: "utf8", maxBuffer: 256 << 20 });
}

// --- Rust -------------------------------------------------------------------------------

function rustPackages() {
  const meta = JSON.parse(run("cargo", ["metadata", "--format-version", "1", "--locked"]));
  const byId = new Map(meta.packages.map((p) => [p.id, p]));
  const nodes = new Map(meta.resolve.nodes.map((n) => [n.id, n]));
  const members = new Set(meta.workspace_members);
  const seen = new Set();
  const stack = [...members];
  while (stack.length) {
    const id = stack.pop();
    if (seen.has(id)) continue;
    seen.add(id);
    for (const dep of nodes.get(id)?.deps ?? []) {
      const shipped = dep.dep_kinds.some((k) => k.kind !== "dev");
      if (shipped) stack.push(dep.pkg);
    }
  }
  const pkgs = [];
  for (const id of seen) {
    if (members.has(id)) continue;
    const p = byId.get(id);
    const dir = dirname(p.manifest_path);
    const files = licenseFiles(dir);
    for (const v of VENDORED[p.name] ?? []) {
      const path = join(dir, v.file);
      if (!existsSync(path)) throw new Error(`${p.name}: vendored license ${v.file} not found in ${dir}`);
      files.push({ file: v.file, label: v.name, text: readFileSync(path, "utf8") });
    }
    pkgs.push({
      ecosystem: "Rust crate",
      name: p.name,
      version: p.version,
      license: p.license ?? (p.license_file ? `see ${p.license_file}` : "UNKNOWN"),
      url: p.repository ?? p.homepage ?? `https://crates.io/crates/${p.name}`,
      authors: (p.authors ?? []).join(", "),
      files,
    });
  }
  return pkgs;
}

// --- JS ---------------------------------------------------------------------------------

function jsPackages() {
  const json = JSON.parse(run("pnpm", ["licenses", "list", "--json", "--prod"]) || "{}");
  const pkgs = [];
  for (const [license, list] of Object.entries(json)) {
    for (const p of list) {
      const versions = p.versions ?? [p.version];
      const paths = p.paths ?? [p.path];
      versions.forEach((version, i) => {
        pkgs.push({
          ecosystem: "npm package",
          name: p.name,
          version,
          license,
          url: p.homepage ?? `https://www.npmjs.com/package/${p.name}`,
          authors: typeof p.author === "string" ? p.author : (p.author?.name ?? ""),
          files: licenseFiles(paths[i] ?? paths[0]),
        });
      });
    }
  }
  return pkgs;
}

// --- Render -----------------------------------------------------------------------------

function normalize(text) {
  return text.replace(/\r\n?/g, "\n").trim();
}

function render(pkgs) {
  pkgs.sort((a, b) => a.ecosystem.localeCompare(b.ecosystem) || a.name.localeCompare(b.name) || a.version.localeCompare(b.version, undefined, { numeric: true }));
  // Dedupe identical license texts: each distinct text is printed once, as [T<n>].
  const texts = new Map();
  const ref = (text) => {
    const n = normalize(text);
    const h = createHash("sha256").update(n).digest("hex");
    if (!texts.has(h)) texts.set(h, { id: `T${texts.size + 1}`, text: n });
    return texts.get(h).id;
  };
  const rule = "=".repeat(80);
  const lines = [
    "THIRD-PARTY SOFTWARE NOTICES",
    "",
    "Ethereal is free software licensed under the GNU General Public License v3.0 or later",
    "(see LICENSE). It includes the third-party components listed below, each under its own",
    "license. License texts are reproduced in the \"License texts\" section and referenced by",
    "id ([T1], [T2], ...). Generated by scripts/release/third-party-notices.mjs; do not edit.",
    "",
  ];
  const entry = (p) => {
    lines.push(`${p.name} ${p.version} (${p.ecosystem})`);
    lines.push(`  License: ${p.license}`);
    if (p.authors) lines.push(`  Authors: ${p.authors}`);
    lines.push(`  Source:  ${p.url}`);
    if (p.files.length === 0) lines.push("  License text: not included in the published package; see the source URL.");
    for (const f of p.files) lines.push(`  ${f.label ?? f.file}: [${ref(f.text)}]`);
    lines.push("");
  };
  const highlighted = pkgs.filter((p) => HIGHLIGHT.has(p.name));
  if (highlighted.length) {
    lines.push(rule, "Time-stretching engine", rule, "");
    highlighted.forEach(entry);
  }
  lines.push(rule, "Interface definitions (clean-room references; no code included)", rule, "");
  for (const r of INTERFACE_REFERENCES) {
    lines.push(r.name, `  License: ${r.license}`, `  Authors: ${r.authors}`, `  Source:  ${r.url}`, `  Used as: ${r.use}`, "");
  }
  let eco = null;
  for (const p of pkgs) {
    if (HIGHLIGHT.has(p.name)) continue;
    if (p.ecosystem !== eco) {
      eco = p.ecosystem;
      lines.push(rule, `${eco}s`, rule, "");
    }
    entry(p);
  }
  lines.push(rule, "License texts", rule, "");
  for (const { id, text } of texts.values()) lines.push(`----- [${id}] ${"-".repeat(70 - id.length)}`, "", text, "");
  return `${lines.join("\n").trimEnd()}\n`;
}

const pkgs = [...rustPackages(), ...jsPackages()];
const missing = [...Object.keys(VENDORED)].filter((n) => !pkgs.some((p) => p.name === n));
if (missing.length) {
  console.error(`third-party-notices: expected components not in the dependency graph: ${missing.join(", ")}`);
  process.exit(1);
}
const text = render(pkgs);
if (check) {
  const cur = existsSync(out) ? readFileSync(out, "utf8") : "";
  if (cur !== text) {
    console.error(`third-party-notices: ${out} is stale; run node scripts/release/third-party-notices.mjs`);
    process.exit(1);
  }
  console.log(`third-party-notices: ${out} is up to date (${pkgs.length} components)`);
} else {
  writeFileSync(out, text);
  const noText = pkgs.filter((p) => p.files.length === 0).length;
  console.log(`third-party-notices: ${pkgs.length} components (${noText} without a license file) → ${out}`);
}
