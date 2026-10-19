#!/usr/bin/env node
// The full product (this checkout + the `ee/` submodule), from a normal OSS checkout:
//
//   just full-desktop [args]   the desktop app, dev build, every feature
//   just full-build [args]     the desktop release bundle (`tauri build --features full`)
//   just full-server [args]    the headless engine server, every feature
//   just full-web [args]       the full web app's dev server
//   node scripts/full.mjs prepare   only bring ee/ and the build tree up to date
//
// It initialises `ee/` (and its submodules) when missing; that needs read access to the
// private repository. While the private Rust code still overlays OSS files (`ee/overlay.mjs`),
// the build runs in a cached copy, `target/full-tree`: this checkout (your uncommitted
// changes included) with the overlay applied, refreshed on every run by rewriting (rsync) only
// the files whose content changed, so its cargo target stays warm. Once `ee/` builds on its own
// (no `overlay.mjs`), the recipes run there in place: `just --justfile ee/justfile full-<x>`.
//
// docs/EE-WORKFLOW.md "Building the full app".
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, statSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
const EE = join(ROOT, "ee");
const TREE = join(ROOT, "target", "full-tree");
/** The files the last refresh wrote (what a later one may delete), relative to TREE. */
const MANIFEST = join(TREE, ".full-tree-files");
const RECIPES = { desktop: "full-desktop", build: "full-build", server: "full-server", web: "full-web" };

function die(msg) {
  console.error(`full: ${msg}`);
  process.exit(1);
}

function run(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { stdio: "inherit", ...opts });
  if (r.error) die(`${cmd}: ${r.error.message}`);
  return r.status ?? 1;
}

/** `ee/` and its own submodules, initialised when missing. */
function ensureEe() {
  if (!existsSync(join(EE, "justfile"))) {
    console.log("full: initialising ee/ (the private repository)");
    // `update = none` in .gitmodules (public clones never fetch it): `--checkout` overrides it.
    if (run("git", ["-C", ROOT, "submodule", "update", "--init", "--checkout", "ee"]) !== 0 || !existsSync(join(EE, "justfile"))) {
      die("could not fetch ee/: the full app needs read access to the private repository");
    }
  }
  // Nested submodules (e.g. the VP9 encoder's sources): optional, the build works without.
  if (run("git", ["-C", EE, "submodule", "update", "--init", "--recursive"]) !== 0) {
    console.warn("full: some of ee/'s submodules could not be fetched (optional parts build without them)");
  }
}

function hash(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function same(a, b) {
  const [sa, sb] = [statSync(a), statSync(b)];
  return sa.size === sb.size && hash(a) === hash(b);
}

function filesUnder(dir, base = dir, out = []) {
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) filesUnder(p, base, out);
    else if (e.isFile()) out.push(relative(base, p));
  }
  return out;
}

/** Copy `stage` over TREE, rewriting only the files whose content differs; returns the count. */
function syncInto(stage, files) {
  // rsync compares checksums (a fresh copy has new mtimes) and leaves equal files untouched,
  // so their mtimes stay and cargo/vite see no change.
  const r = spawnSync("rsync", ["-rlt", "--checksum", "--out-format=%n", `${stage}/`, `${TREE}/`], { encoding: "utf8", maxBuffer: 1 << 28 });
  if (!r.error && r.status === 0) return r.stdout.split("\n").filter((l) => l && !l.endsWith("/")).length;
  if (!r.error) die(`rsync failed: ${r.stderr}`);
  // No rsync (e.g. Windows): the same, in node.
  let written = 0;
  for (const f of files) {
    const [from, to] = [join(stage, f), join(TREE, f)];
    if (existsSync(to) && same(from, to)) continue;
    mkdirSync(dirname(to), { recursive: true });
    copyFileSync(from, to);
    written++;
  }
  return written;
}

/** Refresh TREE from a fresh overlay of this checkout, touching only changed files. */
function refreshTree() {
  // The overlay refuses to make a tree inside the checkout: stage it in the temp dir.
  const scratch = mkdtempSync(join(tmpdir(), "ether-full-"));
  const stage = join(scratch, "tree");
  try {
    const made = spawnSync(process.execPath, [join(EE, "overlay.mjs"), "--make", stage, "--from", ROOT], { stdio: ["ignore", "ignore", "inherit"] });
    if (made.status !== 0) die("the overlay failed (above)");
    const files = filesUnder(stage);
    mkdirSync(TREE, { recursive: true });
    const written = syncInto(stage, files);
    // Files the last refresh wrote that are gone now (build outputs are never in the list).
    const now = new Set(files);
    const before = existsSync(MANIFEST) ? readFileSync(MANIFEST, "utf8").split("\n").filter(Boolean) : [];
    let removed = 0;
    for (const f of before) {
      if (now.has(f) || !existsSync(join(TREE, f))) continue;
      unlinkSync(join(TREE, f));
      removed++;
    }
    writeFileSync(MANIFEST, files.join("\n") + "\n");
    console.log(`full: ${relative(ROOT, TREE)} refreshed (${written} written, ${removed} removed, ${files.length} files)`);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

/** JS dependencies of the tree: installed again only when a lockfile changed. */
function installTree() {
  const stamp = join(TREE, "target", ".full-installed");
  const locks = ["pnpm-lock.yaml", join("ee", "pnpm-lock.yaml")].filter((f) => existsSync(join(TREE, f)));
  const key = locks.map((f) => hash(join(TREE, f))).join(" ");
  const fresh = existsSync(stamp) && readFileSync(stamp, "utf8") === key && existsSync(join(TREE, "node_modules"));
  if (!fresh) {
    if (run("pnpm", ["install", "--frozen-lockfile", "--prefer-offline"], { cwd: TREE }) !== 0) die("pnpm install failed");
    if (existsSync(join(TREE, "ee", "pnpm-lock.yaml")) && run("pnpm", ["--dir", "ee", "install", "--frozen-lockfile", "--prefer-offline"], { cwd: TREE }) !== 0) {
      die("pnpm install (ee/) failed");
    }
    mkdirSync(dirname(stamp), { recursive: true });
    writeFileSync(stamp, key);
  }
  if (existsSync(join(TREE, "ee", "app", "link.mjs"))) run(process.execPath, [join("ee", "app", "link.mjs")], { cwd: TREE });
}

const [what, ...args] = process.argv.slice(2);
const recipe = what === "prepare" ? null : (RECIPES[what] ?? die(`usage: full.mjs <${Object.keys(RECIPES).join("|")}|prepare> [args]`));
ensureEe();
if (existsSync(join(EE, "overlay.mjs"))) {
  refreshTree();
  installTree();
  process.exit(recipe ? run("just", [recipe, ...args], { cwd: TREE }) : 0);
}
if (!recipe) process.exit(0);
// `ee/` builds on its own: its recipes run there.
process.exit(run("just", ["--justfile", join(EE, "justfile"), "--working-directory", EE, recipe, ...args]));
