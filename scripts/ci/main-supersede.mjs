// Supersede (.github/workflows/main-supersede.yml): after a push to `main`, cancel the push runs of
// CI, CI (Windows), Full product and Full product (macOS, Windows) that a newer push run of the same
// workflow replaces, so a streak of merges builds only its last commit. Each workflow's main
// concurrency group (cancel-in-progress: false) already skips the runs that wait; this cancels the
// one that is running, and the newest then starts.
//
// Starvation guard: if a streak has kept a workflow without a finished run (green or red) on `main`
// for more than two hours, the running run is let finish: `main` gets a signal at least every
// ~2 h plus one run, however fast the merges come. The clock starts at the later of the last
// finished run and the first push after it (a quiet `main` isn't starving).
//
// Never touched: manual runs and re-runs (someone asked for them), other workflows (release, EE
// sync, ...), the newest run of each workflow.
//
//   node scripts/ci/main-supersede.mjs   (GH_TOKEN, GH_REPO, SHA: the pushed commit; `--dry-run`)
import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";
import { WORKFLOWS, latestFinished } from "./main-status.mjs";

export const STARVING_MS = 2 * 3600e3;

/**
 * What to cancel among a workflow's push runs on `main` (newest first, any status). Returns
 * `{ cancel: [run], reason }`.
 */
export function decide(runs, now, starvingMs = STARVING_MS) {
  const active = runs.filter((r) => r.status !== "completed");
  if (active.length < 2) return { cancel: [], reason: "nothing superseded" };
  const [newest, ...older] = [...active].sort((a, b) => b.run_number - a.run_number);
  const cancel = older.filter((r) => (r.run_attempt ?? 1) === 1);
  if (!cancel.length) return { cancel, reason: "only re-runs are superseded" };
  const last = latestFinished(runs);
  if (!last) return { cancel: [], reason: "no finished run in sight: let it finish" };
  const since = runs.filter((r) => r.run_number > last.run_number).map((r) => Date.parse(r.created_at));
  const quietSince = Math.max(Date.parse(last.updated_at), since.length ? Math.min(...since) : 0);
  const hours = ((now - quietSince) / 3600e3).toFixed(1);
  if (now - quietSince > starvingMs) return { cancel: [], reason: `no finished run for ${hours} h: let it finish` };
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
