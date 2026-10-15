#!/usr/bin/env node
// Generates THIRD-PARTY-NOTICES.md: every third-party component distributed with Ethereal
// (the desktop installers and the web build) and the license texts they require.
//
//   node scripts/gen-notices.mjs
//
// Rust crates come from `cargo bundle-licenses` (the lockfile's resolved graph). Crates whose
// published tarball omits a license file ("NOT FOUND") are filled from `third-party/`
// (third-party/README.md). npm packages come from `pnpm licenses list` (production deps);
// their texts are read from node_modules, so `pnpm install` must have run. The vendored
// Signalsmith C++ (crates/ether-stretch/csrc) is added by hand.
//
// The output is committed; `just check-notices-fresh` (CI) fails if it is stale. Run
// `just notices` after changing Cargo.lock or the JS dependencies.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdtempSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const thirdParty = join(root, "third-party");
const outPath = join(root, "THIRD-PARTY-NOTICES.md");
const LICENSE_FILE = /^(licen[cs]e|copying|notice|unlicense)/i;

function fail(msg) {
  console.error(`gen-notices: ${msg}`);
  process.exit(1);
}

function normRepo(url) {
  return (url || "").replace(/\.git$/, "").replace(/\/$/, "");
}

// --- Rust crates -----------------------------------------------------------------------------
function rustLibraries() {
  const out = join(mkdtempSync(join(tmpdir(), "ether-lic-")), "notices.json");
  try {
    execFileSync("cargo", ["bundle-licenses", "--format", "json", "-o", out], { cwd: root, stdio: ["ignore", "ignore", "pipe"] });
  } catch (e) {
    fail(`cargo bundle-licenses failed (install it with \`cargo install cargo-bundle-licenses --version 4.2.0\`): ${e.message}\n${e.stderr || ""}`);
  }
  return JSON.parse(readFileSync(out, "utf8")).third_party_libraries;
}

const overrides = JSON.parse(readFileSync(join(thirdParty, "overrides.json"), "utf8"));
function overrideText(name, repo, id) {
  const pick = (map) => (map && map[id] ? readFileSync(join(thirdParty, map[id]), "utf8") : null);
  const key = Object.keys(overrides.repositories).find((k) => normRepo(k) === normRepo(repo));
  return pick(key && overrides.repositories[key]) || pick(overrides.canonical) || pick(overrides.crates[name]);
}

function components() {
  const comps = [];
  for (const lib of rustLibraries()) {
    const texts = lib.licenses.map((l) => ({
      id: l.license,
      text: l.text && l.text !== "NOT FOUND" ? l.text : overrideText(lib.package_name, lib.repository, l.license),
    }));
    if (texts.some((t) => !t.text))
      fail(`no license text for ${lib.package_name}@${lib.package_version} (${lib.license}); add an override in third-party/overrides.json`);
    comps.push({ ecosystem: "Rust", name: lib.package_name, version: lib.package_version, license: lib.license, source: lib.repository || "", texts });
  }
  return comps;
}

// --- npm / pnpm ------------------------------------------------------------------------------
function npmComponents() {
  let listed;
  try {
    listed = JSON.parse(execFileSync("pnpm", ["licenses", "list", "--json", "--prod"], { cwd: root, encoding: "utf8" }));
  } catch (e) {
    fail(`pnpm licenses list failed (run \`pnpm install\` first): ${e.message}`);
  }
  const comps = [];
  for (const [license, pkgs] of Object.entries(listed)) {
    for (const p of pkgs) {
      const dir = p.paths[0];
      const id = p.license || license;
      const files = existsSync(dir) ? readdirSync(dir).filter((f) => LICENSE_FILE.test(f)) : [];
      // A few packages ship no license file; fall back to the canonical SPDX text.
      const text = files.length > 0 ? files.map((f) => readFileSync(join(dir, f), "utf8").trim()).join("\n\n") : overrideText(p.name, "", id);
      if (!text) fail(`no license file found for ${p.name}@${p.versions[0]} in ${dir} (${id})`);
      comps.push({ ecosystem: "npm", name: p.name, version: p.versions[0], license: id, source: p.homepage || "", texts: [{ id, text }] });
    }
  }
  return comps;
}

// --- vendored source ---------------------------------------------------------------------------
function vendoredComponents() {
  const csrc = join(root, "crates/ether-stretch/csrc");
  return [
    {
      name: "Signalsmith Stretch (vendored C++)",
      version: "1.3.1",
      license: "MIT",
      source: "https://github.com/Signalsmith-Audio/signalsmith-stretch",
      texts: [{ id: "MIT", text: readFileSync(join(csrc, "LICENSE-signalsmith-stretch.txt"), "utf8") }],
    },
    {
      name: "Signalsmith Linear (vendored C++)",
      version: "-",
      license: "MIT",
      source: "https://github.com/Signalsmith-Audio/linear",
      texts: [{ id: "MIT", text: readFileSync(join(csrc, "signalsmith-linear/LICENSE.txt"), "utf8") }],
    },
  ].map((c) => ({ ecosystem: "vendored", ...c }));
}

// --- render -------------------------------------------------------------------------------------
function main() {
  const libs = [...components(), ...npmComponents(), ...vendoredComponents()];
  libs.sort(
    (a, b) => a.name.localeCompare(b.name, "en", { sensitivity: "base" }) || a.version.localeCompare(b.version) || a.ecosystem.localeCompare(b.ecosystem),
  );

  // Deduplicate license texts by content; remember which components (and SPDX ids) each serves.
  const texts = new Map();
  for (const c of libs) {
    for (const t of c.texts) {
      const key = t.text.replace(/\r\n/g, "\n").trim();
      if (!texts.has(key)) texts.set(key, { ids: new Set(), users: [] });
      const e = texts.get(key);
      e.ids.add(t.id);
      if (!e.users.includes(c.name)) e.users.push(c.name);
    }
  }

  const lines = [];
  lines.push("# Third-party notices", "");
  lines.push("Ethereal is built with open-source software. This file lists every third-party");
  lines.push("component distributed with Ethereal (the desktop installers and the web build) and");
  lines.push("reproduces the license texts they require.", "");
  lines.push(`Components: ${libs.length} (Rust crates, npm packages and vendored source).`, "");
  lines.push("<!-- GENERATED by scripts/gen-notices.mjs; do not edit. Run `just notices`. -->", "");

  lines.push("## Components", "");
  lines.push("| Component | Version | License | Source |");
  lines.push("| --- | --- | --- | --- |");
  for (const c of libs) {
    lines.push(`| \`${c.name}\` | ${c.version} | ${c.license.replace(/\|/g, "\\|")} | ${c.source || "—"} |`);
  }
  lines.push("");

  lines.push("## License texts", "");
  const sorted = [...texts.entries()].sort((a, b) => {
    const la = [...a[1].ids].sort().join(" ");
    const lb = [...b[1].ids].sort().join(" ");
    return la.localeCompare(lb) || a[0].localeCompare(b[0]);
  });
  for (const [text, e] of sorted) {
    const ids = [...e.ids].sort().join(" / ");
    const first = text.split("\n").find((l) => l.trim()) || "";
    const title = /^copyright/i.test(first.trim()) ? `${ids} — ${first.trim()}` : ids;
    lines.push(`### ${title}`, "");
    lines.push(
      `<sub>Applies to ${e.users.length} component(s): ${e.users.slice(0, 12).join(", ")}${e.users.length > 12 ? `, +${e.users.length - 12} more` : ""}.</sub>`,
      "",
    );
    lines.push(text, "");
  }

  const content = lines.join("\n").replace(/\n{3,}/g, "\n\n");
  const digest = createHash("sha256").update(content).digest("hex").slice(0, 12);
  writeFileSync(outPath, content.trimEnd() + "\n");
  console.log(`gen-notices: wrote ${outPath} (${libs.length} components, ${texts.size} license texts, sha256:${digest})`);
}

main();
