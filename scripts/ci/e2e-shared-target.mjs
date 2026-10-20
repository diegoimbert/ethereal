#!/usr/bin/env node
// One build for every e2e shard of the full product on the devbox pool (docs/CI-RUNNERS.md). Each
// GitHub-hosted shard builds the wasm module (release) and the native binaries its specs start
// (~14 of its ~30 min); on the pool, one job builds them in its cargo target dir and the shards,
// which run on the same machine, use that dir instead of building again. Nothing leaves the
// machine (no artifact), so the private build output stays where the pool's caches are.
//
//   node scripts/ci/e2e-shared-target.mjs publish <t0>
//       (the build job, after the build) stamps its target dir ($CARGO_TARGET_DIR) with the
//       commits it built and <t0>, a time before the build started; writes `dir=<the dir>` to
//       $GITHUB_OUTPUT.
//   node scripts/ci/e2e-shared-target.mjs run <dir> -- <command> [args...]
//       (a shard) runs the command with CARGO_TARGET_DIR=<dir>, holding a shared lock on it (the
//       pool gives a busy dir to no other job), when the dir's stamp names this checkout's
//       commits. Else, or with an empty <dir> (GitHub-hosted), it runs the command as it is: the
//       shard builds in its own target dir, as before.
//
// Cargo decides a path crate is fresh by file times: a source newer than its last build output is
// rebuilt. A shard's checkout has the build's content but its own file times, so for the command's
// duration every tracked file of the checkout (and of ee/) is set to <t0>, then put back (the slot
// keeps its workspace for the next job). The specs' own `cargo build`s are then no-ops.
import { execFileSync, spawnSync } from "node:child_process";
import { appendFileSync, lstatSync, lutimesSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";

const STAMP = "e2e-stamp.json";
const root = process.cwd();

/** The commits of this checkout and of its ee/ (the full product's sources). */
function commits() {
  const head = (dir) => execFileSync("git", ["-C", dir, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  return { oss: head(root), ee: head(join(root, "ee")) };
}

/** Every tracked file of the checkout and of ee/ (with its submodules), and their directories. */
export function trackedPaths(lists) {
  const paths = new Set();
  for (const [base, out] of lists) {
    for (const rel of out.split("\0")) {
      if (!rel) continue;
      let p = join(base, rel);
      paths.add(p);
      while ((p = dirname(p)).length > base.length) paths.add(p);
    }
  }
  return [...paths];
}

function listTracked() {
  const ls = (dir, ...extra) => [dir, execFileSync("git", ["-C", dir, "ls-files", "-z", ...extra], { encoding: "utf8", maxBuffer: 1 << 28 })];
  return trackedPaths([ls(root), ls(join(root, "ee"), "--recurse-submodules")]).filter((p) => p !== join(root, "ee"));
}

/** Sets the paths' times to `t0` (seconds); returns how to put them back. */
function freeze(paths, t0) {
  const saved = [];
  for (const p of paths) {
    try {
      const st = lstatSync(p);
      saved.push([p, st.atime, st.mtime]);
      lutimesSync(p, t0, t0);
    } catch {
      // gone (a deleted file still in the index): nothing to freeze
    }
  }
  return () => {
    for (const [p, a, m] of saved) {
      try {
        lutimesSync(p, a, m);
      } catch {
        // removed by the command
      }
    }
  };
}

export function stampMatches(stamp, want) {
  return !!stamp && stamp.oss === want.oss && stamp.ee === want.ee && Number.isFinite(stamp.t0);
}

function publish(t0) {
  const dir = realpathSync(process.env.CARGO_TARGET_DIR || join(root, "target"));
  // Only a dir of the pool's shared targets: a slot's fallback dir is mounted at the same path in
  // every slot, so a shard would find its own slot's there.
  const shared = process.env.CI_POOL_TARGETS && dir.startsWith(realpathSync(process.env.CI_POOL_TARGETS) + "/");
  if (!shared) {
    console.log("e2e: the build is in no shared target dir; every shard builds its own");
    return;
  }
  writeFileSync(join(dir, STAMP), JSON.stringify({ ...commits(), t0: Number(t0) }));
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `dir=${dir}\n`);
  console.log(`e2e: build shared from ${dir}`);
}

function exec(cmd, env = process.env) {
  const r = spawnSync(cmd[0], cmd.slice(1), { stdio: "inherit", env });
  return r.status ?? 1;
}

// Inside the shared lock (see `run`).
function inner(dir, cmd) {
  let stamp = null;
  try {
    stamp = JSON.parse(readFileSync(join(dir, STAMP), "utf8"));
  } catch {
    // no build there
  }
  if (!stampMatches(stamp, commits())) {
    console.log("e2e: the shared build is of other commits; this shard builds its own");
    return exec(cmd);
  }
  const t = Date.now();
  const paths = listTracked();
  const restore = freeze(paths, stamp.t0);
  console.log(`e2e: using the shared build in ${dir} (${paths.length} paths aligned in ${Date.now() - t} ms)`);
  try {
    return exec(cmd, { ...process.env, CARGO_TARGET_DIR: dir });
  } finally {
    restore();
  }
}

function main(argv) {
  const [verb, ...rest] = argv;
  const sep = rest.indexOf("--");
  if (verb === "publish" && rest.length === 1 && /^\d+$/.test(rest[0])) return publish(rest[0]);
  if ((verb === "run" || verb === "inner") && sep === 1 && rest.length > 2) {
    const [dir, , ...cmd] = rest;
    if (verb === "inner") process.exit(inner(dir, cmd));
    if (!dir) process.exit(exec(cmd));
    // flock holds the lock for the child's life; the pool's job-started helper takes dirs with
    // `flock -n` (exclusive), so no other job builds here meanwhile. The build job's own lock is
    // released when its unit ends, a few seconds after the job: wait for it.
    process.exit(exec(["flock", "-s", "-w", "300", `${dir}.lock`, process.execPath, process.argv[1], "inner", dir, "--", ...cmd]));
  }
  console.error("usage: e2e-shared-target.mjs publish <t0> | run <dir> -- <command> [args...]");
  process.exit(2);
}

if (import.meta.url === `file://${process.argv[1]}`) main(process.argv.slice(2));
