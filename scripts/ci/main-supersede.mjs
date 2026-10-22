// Supersede (.github/workflows/main-supersede.yml): after a push to `main`, cancel the push runs of
// CI, CI (Windows), Full product and Full product (macOS, Windows) that a newer push run of the same
// workflow replaces, so a streak of merges builds only its last commit. Each workflow's main
// concurrency group (cancel-in-progress: false) already skips the runs that wait; this cancels the
// one that is running, and the newest then starts.
//
// Starvation guard: if a streak has kept a workflow without a finished run (green or red) on `main`
// for more than two of its typical runs (at least 30 min, at most 2 h), the running run is let
// finish: `main` gets a signal at least every ~3 runs, however fast the merges come. The clock
// starts at the later of the last finished run and the first push after it (a quiet `main` isn't
// starving). With merges closer than 70% of a run (below), only this guard gives a verdict: on
// 2026-10-22 (~17 merges an hour for 3 h) a flat 2 h left every workflow 2-3 h without one.
//
// Nearly done: a running run that has been going (since its first job started) for at least 70%
// of the workflow's typical run (the median of its last finished runs) is let finish too:
// cancelling it would throw away most of its work for a verdict ~a third of a run away, and a
// streak of merges a few minutes apart would otherwise keep `main` without one until the guard.
//
// Never touched: manual runs and re-runs (someone asked for them), other workflows (release, EE
// sync, ...), the newest run of each workflow.
//
//   node scripts/ci/main-supersede.mjs   (GH_TOKEN, GH_REPO, SHA: the pushed commit; `--dry-run`)
import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";
import { VERDICTS, WORKFLOWS, latestFinished } from "./main-status.mjs";

export const STARVING_MS = 2 * 3600e3; // the longest guard (and the one without a typical run)
export const STARVING_MIN_MS = 30 * 60e3;
export const STARVING_RUNS = 2;
export const NEARLY_DONE = 0.7;

/** The median wall time of the last (up to 5) finished runs, or null. */
export function typicalMs(runs) {
  const d = runs
    .filter((r) => r.status === "completed" && VERDICTS.has(r.conclusion))
    .slice(0, 5)
    .map((r) => Date.parse(r.updated_at) - Date.parse(r.run_started_at ?? r.created_at))
    .filter((ms) => ms > 0)
    .sort((a, b) => a - b);
  return d.length ? d[d.length >> 1] : null;
}

/** The starvation guard of a workflow: two typical runs, within [30 min, 2 h]. */
export function starvingMs(runs) {
  const t = typicalMs(runs);
  return t ? Math.min(STARVING_MS, Math.max(STARVING_MIN_MS, STARVING_RUNS * t)) : STARVING_MS;
}

/**
 * What to cancel among a workflow's push runs on `main` (newest first, any status; a running run
 * may carry `first_job_at`, when its first job started, and `cancelling`, when a job of it was
 * cancelled already: a cancel takes minutes to land). Returns `{ cancel: [run], reason }`.
 */
export function decide(runs, now, guardMs = starvingMs(runs)) {
  const active = runs.filter((r) => r.status !== "completed");
  if (active.length < 2) return { cancel: [], reason: "nothing superseded" };
  const [newest, ...older] = [...active].sort((a, b) => b.run_number - a.run_number);
  const cancel = older.filter((r) => (r.run_attempt ?? 1) === 1 && !r.cancelling);
  if (!cancel.length) return { cancel, reason: "only re-runs or runs already cancelling are superseded" };
  const last = latestFinished(runs);
  if (!last) return { cancel: [], reason: "no finished run in sight: let it finish" };
  const since = runs.filter((r) => r.run_number > last.run_number).map((r) => Date.parse(r.created_at));
  const quietSince = Math.max(Date.parse(last.updated_at), since.length ? Math.min(...since) : 0);
  const hours = ((now - quietSince) / 3600e3).toFixed(1);
  if (now - quietSince > guardMs) return { cancel: [], reason: `no finished run for ${hours} h (guard ${Math.round(guardMs / 60e3)} min): let it finish` };
  const typical = typicalMs(runs);
  const done = (r) => (typical && r.first_job_at ? (now - Date.parse(r.first_job_at)) / typical : 0);
  const nearly = cancel.filter((r) => done(r) >= NEARLY_DONE);
  if (nearly.length) {
    // Only the runs older than the one let finish go: a newer one would be its successor.
    const keep = Math.max(...nearly.map((r) => r.run_number));
    const rest = cancel.filter((r) => r.run_number < keep && !nearly.includes(r));
    const pct = nearly.map((r) => `${r.id} ~${Math.round(done(r) * 100)}%`).join(", ");
    return { cancel: rest, reason: `nearly done (${pct} of a typical run): let it finish` };
  }
  return { cancel, reason: `superseded by run ${newest.id} (last finished run ${hours} h ago)` };
}

const gh = (args) => execFileSync("gh", args, { encoding: "utf8", maxBuffer: 64 << 20 });
const api = (path) => JSON.parse(gh(["api", path]));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** The workflow's push runs on `main`, newest first, up to the last finished one (3 pages at most). */
function pushRuns(repo, file) {
  const runs = [];
  for (let page = 1; page <= 3; page++) {
    const got = api(`repos/${repo}/actions/workflows/${file}/runs?branch=main&event=push&per_page=100&page=${page}`).workflow_runs;
    runs.push(...got);
    if (got.length < 100 || latestFinished(runs)) break;
  }
  return runs;
}

async function main() {
  const dry = process.argv.includes("--dry-run");
  const repo = process.env.GH_REPO ?? die("GH_REPO is not set");
  const sha = process.env.SHA ?? die("SHA is not set");
  await Promise.all(
    WORKFLOWS.map(async (w) => {
      // The push's own run appears within seconds; a workflow its paths filter skips (CI on a
      // pointer bump) gets none, then whatever runs stays.
      let runs = [];
      for (let i = 0; i < 12; i++) {
        runs = pushRuns(repo, w.file);
        if (runs.some((r) => r.head_sha === sha)) break;
        await sleep(5000);
      }
      // When the unfinished runs' first jobs started (a run waiting in the group has none yet;
      // a running one can read "queued" while its next jobs wait for runners).
      for (const r of runs.filter((r) => r.status !== "completed")) {
        try {
          const jobs = api(`repos/${repo}/actions/runs/${r.id}/jobs?per_page=100`).jobs;
          const starts = jobs.map((j) => Date.parse(j.started_at ?? "")).filter((t) => t > 0);
          if (starts.length) r.first_job_at = new Date(Math.min(...starts)).toISOString();
          r.cancelling = jobs.some((j) => j.conclusion === "cancelled");
        } catch (e) {
          console.log(`${w.name}: run ${r.id}: no job times (${String(e.stderr ?? e).trim()}); judged as just started`);
        }
        console.log(`${w.name}: run ${r.id} ${r.status}${r.cancelling ? " (cancelling)" : ""}, first job ${r.first_job_at ?? "not started"}`);
      }
      const d = decide(runs, Date.now());
      console.log(`${w.name}: ${d.cancel.length ? `cancel ${d.cancel.map((r) => r.id).join(", ")}, ` : ""}${d.reason}`);
      if (dry) return;
      for (const r of d.cancel) {
        try {
          gh(["api", "-X", "POST", `repos/${repo}/actions/runs/${r.id}/cancel`]);
        } catch (e) {
          console.log(`${w.name}: run ${r.id} not cancelled (${String(e.stderr ?? e).trim()}); it may have just finished`);
        }
      }
    }),
  );
}

function die(msg) {
  console.error(msg);
  process.exit(1);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) await main();
