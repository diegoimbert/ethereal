#!/usr/bin/env node
// The full product (this checkout + the `ee/` submodule), from a normal OSS checkout:
//
//   just full-desktop [args]   the desktop app, dev build, every feature
//   just full-build [args]     the desktop release bundle (`tauri build --features full`)
//   just full-server [args]    the headless engine server, every feature
//   just full-web [args]       the full web app's dev server
//   node scripts/full.mjs prepare   only bring ee/ and the JS dependencies up to date
//
// It initialises `ee/` (and its submodules) when missing; that needs read access to the
// private repository. `ee/` builds in place: its own Cargo workspace (this checkout's crates
// are its path dependencies) and its JS packages over this checkout's installed dependencies;
// the recipes are ee/justfile's (`just --justfile ee/justfile full-<x>`).
//
// docs/EE-WORKFLOW.md "Building the full app".
import { spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));
const EE = join(ROOT, "ee");
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

/** pnpm is a `.cmd` shim on Windows, which only runs through a shell (keep paths out of `args`). */
function pnpm(args, opts) {
  return run("pnpm", args, { shell: process.platform === "win32", ...opts });
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

/** JS dependencies: this checkout's, then ee/'s, and ee/app's links to them. */
function install() {
  if (pnpm(["install", "--frozen-lockfile", "--prefer-offline"], { cwd: ROOT }) !== 0) die("pnpm install failed");
  if (existsSync(join(EE, "pnpm-lock.yaml")) && pnpm(["install", "--frozen-lockfile", "--prefer-offline"], { cwd: EE }) !== 0) {
    die("pnpm install (ee/) failed");
  }
  if (existsSync(join(EE, "app", "link.mjs"))) run(process.execPath, [join(EE, "app", "link.mjs")], { cwd: ROOT });
}

const [what, ...args] = process.argv.slice(2);
const recipe = what === "prepare" ? null : (RECIPES[what] ?? die(`usage: full.mjs <${Object.keys(RECIPES).join("|")}|prepare> [args]`));
ensureEe();
if (!recipe) {
  install();
  process.exit(0);
}
// First run in this checkout: the dependencies (`prepare` refreshes them later).
if (!existsSync(join(ROOT, "node_modules")) || !existsSync(join(EE, "app", "node_modules"))) install();
process.exit(run("just", ["--justfile", join(EE, "justfile"), "--working-directory", EE, recipe, ...args]));
