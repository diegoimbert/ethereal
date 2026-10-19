#!/usr/bin/env node
// Keeps only third-party artifacts in a target dir `cargo chef cook` built, before it is saved
// to the (public) Actions cache: cook also compiles stub versions of the workspace's own crates
// (empty libs named after them), and with the private overlay those names are private.
//
//   node scripts/ci/chef-strip.mjs <target-dir>          remove the workspace crates' artifacts
//   node scripts/ci/chef-strip.mjs <target-dir> --check  fail if any is left (prints counts only)
//
// The workspace members come from `cargo metadata --no-deps` (package and target names, `-` and
// `_` forms). Removed per profile dir (target/<profile>, target/<triple>/<profile>): deps/,
// .fingerprint/, build/, examples/ entries named `[lib]<name>[-<hash>]…`, the top-level `<name>`/`lib<name>`
// files, and incremental/. Never prints a name.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, rmSync, statSync } from "node:fs";
import { basename, join } from "node:path";

const [dir, flag] = process.argv.slice(2);
if (!dir || (flag && flag !== "--check")) {
  console.error("usage: chef-strip.mjs <target-dir> [--check]");
  process.exit(2);
}
const check = flag === "--check";

const meta = JSON.parse(execFileSync("cargo", ["metadata", "--no-deps", "--format-version", "1"], { encoding: "utf8", maxBuffer: 1 << 28, stdio: ["ignore", "pipe", "ignore"] }));
const names = new Set();
for (const p of meta.packages) {
  // Build scripts are all named `build-script-build`: not a crate name.
  for (const n of [p.name, ...p.targets.filter((t) => !t.kind.includes("custom-build")).map((t) => t.name)]) {
    names.add(n);
    names.add(n.replaceAll("-", "_"));
  }
}
const escape = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const alt = [...names].map(escape).join("|");
// `[lib]name-<16 hex>[.ext...]` and the plain `[lib]name[.ext]` outputs, in each profile dir and
// its deps/, .fingerprint/, build/ and examples/ (third-party build-script outputs, deeper down,
// are not looked into: their file names are the dependencies' own).
const hashed = new RegExp(`^(?:lib)?(?:${alt})-[0-9a-f]{16}(?:\\..*)?$`);
const plain = new RegExp(`^(?:lib)?(?:${alt})(?:\\.(?:d|rlib|rmeta|so|dylib|dll|exe|pdb|lib|a))?$`);

/** Profile dirs: target/<profile> and target/<triple>/<profile> (those with a deps/ dir). */
function profileDirs(root) {
  const out = [];
  if (!existsSync(root)) return out;
  for (const a of readdirSync(root)) {
    const p = join(root, a);
    if (!statSync(p).isDirectory()) continue;
    if (existsSync(join(p, "deps"))) out.push(p);
    else for (const b of readdirSync(p)) if (existsSync(join(p, b, "deps"))) out.push(join(p, b));
  }
  return out;
}

let hits = 0;
for (const prof of profileDirs(dir)) {
  for (const sub of ["deps", ".fingerprint", "build", "examples"]) {
    const d = join(prof, sub);
    if (!existsSync(d)) continue;
    for (const f of readdirSync(d)) {
      if (!hashed.test(f) && !plain.test(f)) continue;
      hits++;
      if (!check) rmSync(join(d, f), { recursive: true, force: true });
    }
  }
  for (const f of readdirSync(prof)) {
    const p = join(prof, f);
    if (statSync(p).isDirectory()) {
      if (f === "incremental") {
        hits += check ? readdirSync(p).length : 0;
        if (!check) rmSync(p, { recursive: true, force: true });
      }
      continue;
    }
    if (!plain.test(f)) continue;
    hits++;
    if (!check) rmSync(p, { force: true });
  }
}

if (check) {
  if (hits > 0) {
    console.error(`chef-strip: ${hits} workspace-crate artifact(s) left in ${basename(dir)}: not saving it`);
    process.exit(1);
  }
  console.log(`chef-strip: ${basename(dir)} holds third-party artifacts only`);
} else {
  console.log(`chef-strip: removed ${hits} workspace-crate artifact(s)`);
}
