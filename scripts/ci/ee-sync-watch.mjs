#!/usr/bin/env node
// The sync bot's loop (.github/workflows/ee-sync.yml): runs `feat sync --ci` whenever a branch
// of either repository moves, for WATCH_MINUTES, then exits (the workflow starts its next run).
// GitHub's schedule alone is not enough: the `*/10` cron fired about every 4 h (16 runs in 3 days
// to 2026-10-22), and merges that don't go through `feat ship` (an EE PR merged on GitHub) left
// OSS main on an old EE commit for up to an hour.
//
// Watching costs no REST API quota: `git ls-remote` of both repositories every WATCH_POLL_S
// seconds (branch heads, and which ship-lock refs exist: their heartbeats are not a change).
// A move of either main syncs right away; other moves (PR branches, locks) at most every
// WATCH_GAP_S seconds, to keep the bot token's API quota (a sync makes a few calls per open
// PR). A sync also runs every WATCH_SYNC_MIN minutes regardless (a ship lock going stale), and
// a failed one is retried after WATCH_RETRY_S seconds. Each sync's output goes to the
// encrypted log (private-step.mjs): the public log only says when one ran and its result.
//
//   FEAT_OSS_REPO, FEAT_EE_REPO, FEAT_EE_TOKEN (read access to EE), as for `feat sync --ci`.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const num = (k, d) => (Number.isFinite(Number(process.env[k])) && process.env[k] !== "" && process.env[k] !== undefined ? Number(process.env[k]) : d);
const MINUTES = num("WATCH_MINUTES", 55);
const POLL_MS = num("WATCH_POLL_S", 20) * 1000;
const SYNC_MS = num("WATCH_SYNC_MIN", 15) * 60e3;
const GAP_MS = num("WATCH_GAP_S", 180) * 1000;
const RETRY_MS = num("WATCH_RETRY_S", 120) * 1000;
const HERE = dirname(fileURLToPath(import.meta.url));

// What a sync depends on, as digests (never printed: EE branch names are private): `main` for
// both repositories' main, `all` for every branch and which lock refs exist.
export function digest(ossOut, eeOut) {
  const keep = (out) =>
    out
      .split("\n")
      .map((l) => l.trim().split(/\s+/))
      .filter(([sha, ref]) => sha && ref)
      .map(([sha, ref]) => (ref.startsWith("refs/locks/") ? ref : `${ref} ${sha}`))
      .sort();
  const h = (lines) => createHash("sha256").update(lines.join("\n")).digest("hex");
  const [o, e] = [keep(ossOut), keep(eeOut)];
  const main = (l) => l.startsWith("refs/heads/main ");
  return { main: h([...o.filter(main), "", ...e.filter(main)]), all: h([...o, "", ...e]) };
}

// Sync now? `last`: the digests the last good sync saw (null: none yet).
export function due({ now, last, sinceSync, failedAgo }) {
  if (failedAgo !== null && failedAgo < RETRY_MS) return null;
  if (last === null) return "start";
  if (now && now.main !== last.main) return "main moved";
  if (now && now.all !== last.all && sinceSync >= GAP_MS) return "a branch moved";
  if (sinceSync >= SYNC_MS) return "periodic";
  return null;
}

export function lsRemote(repo, token) {
  const args = [];
  if (token) args.push("-c", `http.extraheader=AUTHORIZATION: basic ${Buffer.from(`x-access-token:${token}`).toString("base64")}`);
  args.push("ls-remote", `https://github.com/${repo}`, "refs/heads/*", "refs/locks/*");
  const r = spawnSync("git", args, { encoding: "utf8", maxBuffer: 16 << 20, env: { ...process.env, GIT_TERMINAL_PROMPT: "0" } });
  return r.status === 0 ? r.stdout : null;
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const stamp = () => new Date().toISOString().slice(11, 19);

async function main() {
  const oss = process.env.FEAT_OSS_REPO;
  const ee = process.env.FEAT_EE_REPO;
  const token = process.env.FEAT_EE_TOKEN;
  if (!oss || !ee || !token) {
    console.error("ee-sync-watch: FEAT_OSS_REPO, FEAT_EE_REPO and FEAT_EE_TOKEN are needed");
    process.exit(2);
  }
  const end = Date.now() + MINUTES * 60e3;
  let last = null;
  let lastSync = 0;
  let failedAt = null;
  let n = 0;
  let failed = 0;
  for (;;) {
    const a = lsRemote(oss, token);
    const b = a === null ? null : lsRemote(ee, token);
    const now = a === null || b === null ? null : digest(a, b);
    if (now === null) console.log(`${stamp()} could not list the branches; retrying`);
    const why = due({ now, last, sinceSync: Date.now() - lastSync, failedAgo: failedAt === null ? null : Date.now() - failedAt });
    if (why) {
      n++;
      console.log(`${stamp()} sync ${n} (${why})`);
      // The digests from before the sync: what the sync itself pushes runs one more (no-op) one.
      const r = spawnSync("node", [join(HERE, "private-step.mjs"), `sync-${n}`, "--", "node", join(HERE, "..", "feat.mjs"), "sync", "--ci"], { stdio: "inherit" });
      lastSync = Date.now();
      if (r.status === 0) [last, failedAt] = [now ?? last, null];
      else [failed, failedAt] = [failed + 1, Date.now()];
    }
    if (Date.now() + POLL_MS >= end) break;
    await sleep(POLL_MS);
  }
  console.log(`${n} syncs, ${failed} failed`);
  process.exit(failed && failed === n ? 1 : 0);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) await main();
