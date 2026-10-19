#!/usr/bin/env node
// Deletes the stale test binaries of a persistent cargo target dir (the devbox runner pool's,
// docs/CI-RUNNERS.md). Every dependency bump gives each test binary a new hash, and cargo never
// deletes the old one: they are most of a Rust job's target dir (54 of 58 GB after one day of
// main), which then hits the pool's size cap and is emptied, so the next run builds from cold.
//
//   node scripts/ci/sweep-test-binaries.mjs -- <the `cargo nextest run` arguments of the job>
//
// Keeps what `cargo nextest list` (same arguments, so nothing rebuilds) names: the test binaries of
// this build and the binaries they run. Deletes the other `<name>-<16 hex>` executables of the
// output dirs' `deps/`. A wrongly deleted one only costs a relink. Never fails the job.
import { spawnSync } from "node:child_process";
import { readdirSync, statSync, unlinkSync } from "node:fs";
import { join } from "node:path";

// A cargo output in deps/ that is an executable: `<crate>-<hash>`, no extension.
export const isHashedExecutable = (name, mode) => /^[A-Za-z0-9_]+-[0-9a-f]{16}$/.test(name) && (mode & 0o100) !== 0;

// The executables of `dirs` (deps/ dirs) that the nextest list does not keep.
export function stale(list, dirs, fs = { readdirSync, statSync }) {
  // By inode: cargo hard-links a binary from deps/ to the output dir (debug/<name>).
  const inode = (p) => {
    try {
      const st = fs.statSync(p);
      return `${st.dev}:${st.ino}`;
    } catch {
      return null;
    }
  };
  const meta = list["rust-build-meta"] ?? {};
  const keep = new Set([
    ...Object.values(list["rust-binaries"] ?? {}).map((b) => inode(b["binary-path"])),
    ...Object.values(meta["non-test-binaries"] ?? {})
      .flat()
      .map((b) => inode(join(meta["target-directory"], b.path))),
  ]);
  const out = [];
  for (const dir of dirs) {
    let names = [];
    try {
      names = fs.readdirSync(dir);
    } catch {
      continue;
    }
    for (const name of names) {
      const p = join(dir, name);
      const st = fs.statSync(p);
      if (st.isFile() && isHashedExecutable(name, st.mode) && !keep.has(`${st.dev}:${st.ino}`)) out.push({ path: p, size: st.size });
    }
  }
  return out;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const args = process.argv.slice(process.argv.indexOf("--") + 1);
  const r = spawnSync("cargo", ["nextest", "list", "--list-type", "binaries-only", "--message-format", "json", ...args], {
    encoding: "utf8",
    maxBuffer: 256 << 20,
    stdio: ["ignore", "pipe", "inherit"],
  });
  let list;
  try {
    if (r.status !== 0) throw new Error(`cargo nextest list exited ${r.status}`);
    list = JSON.parse(r.stdout);
  } catch (e) {
    console.log(`::warning::sweep skipped: ${e.message}`);
    process.exit(0);
  }
  const meta = list["rust-build-meta"];
  const dirs = (meta["base-output-directories"] ?? []).map((d) => join(meta["target-directory"], d, "deps"));
  let freed = 0;
  let n = 0;
  for (const f of stale(list, dirs)) {
    try {
      unlinkSync(f.path);
      freed += f.size;
      n++;
    } catch {}
  }
  console.log(`sweep: deleted ${n} stale test binaries, ${(freed / 1e9).toFixed(1)} GB`);
}
