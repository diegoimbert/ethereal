#!/usr/bin/env node
// Balanced e2e shards by time, for the full product's e2e on the devbox pool (docs/CI-RUNNERS.md).
// Playwright's `--shard=i/n` cuts the ordered test list into n runs of equal test count; the last
// projects in the list (the Canvas2D arranger specs, the collab/share specs, the tours) are the
// slow ones, so the last shard took 15 min where the others took 5. Here, the e2e build job plans
// the shards once from the previous runs' times and the shards run `--test-list <their file>`.
//
//   node scripts/ci/e2e-shard-plan.mjs plan <shards> <times-dir> <out-dir> <cwd> -- <playwright args>
//       lists the tests (`playwright test --list <args>` in <cwd>), groups them by project and
//       file (a file's tests stay together: its beforeAll runs once), and deals the groups to the
//       shards, longest first, each to the shard with the least time so far. A group's time is its
//       last measured one (<times-dir>/*.json), else its test count times the measured mean. Writes
//       <out-dir>/<shard>.txt (1-based). The log shows counts and seconds, never test names.
//   node scripts/ci/e2e-shard-plan.mjs select <shard> <shards>
//       prints the Playwright arguments of a shard: `--test-list $E2E_PLAN/<shard>.txt` when the
//       shared build's plan is there (scripts/ci/e2e-shared-target.mjs sets E2E_PLAN), else
//       `--shard=<shard>/<shards>`.
//   node scripts/ci/e2e-shard-plan.mjs record <log> <times-file>
//       a shard's Playwright log (list reporter) -> the time of each group it ran.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";

// `[project] › file:line:col › title` (the list reporter's line, also --test-list's format).
const TEST = /^\s*\[([^\]]+)\] › (.+?):\d+:\d+ › /;
// A result line: `✓  12 [project] › file:line:col › title (3.1s)`.
const RESULT = /^\s*[✓✘]\s+\d+\s+(\[[^\]]+\] › .+?):\d+:\d+ › .*\((\d+(?:\.\d+)?)(ms|s|m)\)\s*$/;

/** The tests of a `--list` output: their lines and groups. */
export function parseList(text) {
  const tests = [];
  for (const raw of text.split("\n")) {
    const m = TEST.exec(raw);
    if (m) tests.push({ line: raw.trim(), group: `[${m[1]}] › ${m[2]}` });
  }
  return tests;
}

/** Seconds per group from a shard's log. */
export function parseResults(text) {
  const times = {};
  for (const raw of text.split("\n")) {
    const m = RESULT.exec(raw);
    if (!m) continue;
    const v = Number(m[2]) * (m[3] === "ms" ? 0.001 : m[3] === "m" ? 60 : 1);
    times[m[1]] = (times[m[1]] ?? 0) + v;
  }
  return times;
}

/** The tests dealt to `shards` shards by time (longest group first, to the least loaded shard). */
export function plan(tests, times, shards) {
  const groups = new Map();
  for (const t of tests) groups.set(t.group, [...(groups.get(t.group) ?? []), t.line]);
  let known = 0;
  let knownTests = 0;
  for (const [g, lines] of groups)
    if (times[g] !== undefined) {
      known += times[g];
      knownTests += lines.length;
    }
  const perTest = knownTests ? known / knownTests : 1;
  const weighted = [...groups].map(([g, lines]) => ({ lines, secs: times[g] ?? lines.length * perTest }));
  weighted.sort((a, b) => b.secs - a.secs || b.lines.length - a.lines.length || (a.lines[0] < b.lines[0] ? -1 : 1));
  const out = Array.from({ length: shards }, () => ({ lines: [], secs: 0 }));
  for (const g of weighted) {
    const s = out.reduce((m, x) => (x.secs < m.secs ? x : m), out[0]);
    s.lines.push(...g.lines);
    s.secs += g.secs;
  }
  return out;
}

function readTimes(dir) {
  const times = {};
  let files = [];
  try {
    files = readdirSync(dir).filter((f) => f.endsWith(".json"));
  } catch {
    return times;
  }
  for (const f of files) {
    try {
      Object.assign(times, JSON.parse(readFileSync(join(dir, f), "utf8")));
    } catch {
      // a half-written file: skip it
    }
  }
  return times;
}

function main([verb, ...rest]) {
  if (verb === "select" && rest.length === 2) {
    const file = process.env.E2E_PLAN ? join(process.env.E2E_PLAN, `${rest[0]}.txt`) : "";
    return console.log(file && existsSync(file) ? `--test-list ${file}` : `--shard=${rest[0]}/${rest[1]}`);
  }
  if (verb === "record" && rest.length === 2) {
    const [log, file] = rest;
    let text = "";
    try {
      text = readFileSync(log, "utf8");
    } catch {
      return console.log("e2e times: no log");
    }
    const times = parseResults(text);
    mkdirSync(join(file, ".."), { recursive: true });
    writeFileSync(file, JSON.stringify(times));
    return console.log(`e2e times: ${Object.keys(times).length} groups recorded`);
  }
  const sep = rest.indexOf("--");
  if (verb === "plan" && sep === 4) {
    const [n, timesDir, outDir, cwd] = rest;
    // A plan left by an earlier build would not match these commits' tests.
    rmSync(outDir, { recursive: true, force: true });
    const r = spawnSync("npx", ["playwright", "test", "--list", ...rest.slice(5)], {
      cwd,
      encoding: "utf8",
      maxBuffer: 64 << 20,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const tests = r.status === 0 ? parseList(r.stdout) : [];
    if (!tests.length) {
      console.log(`e2e plan: listing failed (exit ${r.status}); the shards use --shard`);
      return;
    }
    const shards = plan(tests, readTimes(timesDir), Number(n));
    mkdirSync(outDir, { recursive: true });
    shards.forEach((s, i) => writeFileSync(join(outDir, `${i + 1}.txt`), s.lines.join("\n") + "\n"));
    console.log(`e2e plan: ${tests.length} tests; per shard ${shards.map((s) => `${s.lines.length} tests ~${Math.round(s.secs)} s`).join(", ")}`);
    return;
  }
  console.error("usage: e2e-shard-plan.mjs plan <shards> <times-dir> <out-dir> <cwd> -- <args> | record <log> <times-file>");
  process.exit(2);
}

if (import.meta.url === `file://${process.argv[1]}`) main(process.argv.slice(2));
