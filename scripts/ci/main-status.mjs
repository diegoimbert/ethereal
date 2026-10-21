// Main status (.github/workflows/main-status.yml): after a run of CI, CI (Windows), Full product or
// Full product (macOS, Windows) on `main` finishes, report the latest *finished* push run of each on
// the one open `ci-main` issue. A cancelled run (skipped by the concurrency group, or superseded by
// main-supersede.yml) is no signal: it never opens, comments on or closes the issue.
//   - a red run that the issue doesn't mention yet: a comment (the issue is opened if none is);
//   - a workflow the issue last reported red that is green again, while others are still red: a
//     comment, so the issue says which ones are left;
//   - every workflow green: a last comment, and the issue is closed.
// Only run links, commits and job names: the full product's logs are private.
//
//   node scripts/ci/main-status.mjs            (GH_TOKEN, GH_REPO; `--dry-run` prints only)
import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";

export const WORKFLOWS = [
  { file: "ci.yml", name: "CI" },
  { file: "ci-windows.yml", name: "CI (Windows)" },
  { file: "full.yml", name: "Full product", logs: (id) => `logs: \`just full-logs ${id}\`` },
  { file: "full-platforms.yml", name: "Full product (macOS, Windows)", logs: (id) => `logs: \`just full-logs ${id}\`` },
];
const VERDICTS = new Set(["success", "failure", "timed_out", "startup_failure"]);

/** The newest run that ended green or red (runs come newest first), or null. */
export function latestFinished(runs) {
  return runs.find((r) => r.status === "completed" && VERDICTS.has(r.conclusion)) ?? null;
}

/**
 * What to do with the issue. `latest`: one entry per workflow, `{ name, run, failed, logs }`
 * (`run` null when no run finished yet). `issue`: `{ number, text }` (its body and comments) or
 * null. Returns `{ comments: [string], open: bool, close: bool }`.
 */
export function plan(latest, issue) {
  const red = latest.filter((l) => l.run && l.run.conclusion !== "success");
  const line = (l) =>
    `${l.name}: ${l.run ? `${l.run.conclusion === "success" ? "green" : "red"} at ${l.run.head_sha.slice(0, 10)} (${l.run.html_url})` : "no finished run yet"}`;
  const summary = latest.map((l) => `- ${line(l)}`).join("\n");
  if (!red.length) {
    // Green needs a verdict from every workflow: one with only cancelled runs in sight says nothing.
    if (!issue || latest.some((l) => !l.run)) return { comments: [], open: false, close: false };
    return { comments: [`\`main\` is green again: the latest finished run of every workflow passed.\n\n${summary}`], open: false, close: true };
  }
  const news = red.filter((l) => !issue?.text.includes(l.run.html_url));
  const comments = news.map((l) => {
    const jobs = l.failed.length ? `\n\nFailed: ${l.failed.map((j) => `\`${j}\``).join(", ")}` : "";
    const logs = l.logs ? ` (${l.logs(l.run.id)})` : "";
    return `${l.name} failed on \`main\` at ${l.run.head_sha}: ${l.run.html_url}${logs}${jobs}\n\nLatest finished runs on \`main\`:\n${summary}`;
  });
  // Green again: the issue's last word on the workflow is a failure (its comments are in order).
  const text = issue?.text ?? "";
  const recovered = latest.filter(
    (l) => l.run?.conclusion === "success" && text.lastIndexOf(`${l.name} failed on`) > text.lastIndexOf(`${l.name} is green again`),
  );
  for (const l of recovered) {
    comments.push(`${l.name} is green again on \`main\` at ${l.run.head_sha}: ${l.run.html_url}\n\nStill red: ${red.map((r) => r.name).join(", ")}\n\nLatest finished runs on \`main\`:\n${summary}`);
  }
  return { comments, open: !issue && comments.length > 0, close: false };
}

const gh = (args) => execFileSync("gh", args, { encoding: "utf8", maxBuffer: 64 << 20 });
const api = (path) => JSON.parse(gh(["api", path]));

function main() {
  const dry = process.argv.includes("--dry-run");
  const repo = process.env.GH_REPO ?? die("GH_REPO is not set");
  const latest = WORKFLOWS.map((w) => {
    let run = null;
    try {
      // Past a merge streak's cancelled runs (3 pages at most).
      for (let page = 1; page <= 3 && !run; page++) {
        const runs = api(`repos/${repo}/actions/workflows/${w.file}/runs?branch=main&event=push&status=completed&per_page=100&page=${page}`).workflow_runs;
        run = latestFinished(runs);
        if (runs.length < 100) break;
      }
    } catch {} // a workflow that isn't on `main` yet: no finished run
    const failed = run
      ? api(`repos/${repo}/actions/runs/${run.id}/jobs?filter=latest&per_page=100`)
          .jobs.filter((j) => ["failure", "timed_out"].includes(j.conclusion))
          .map((j) => j.name)
      : [];
    return { ...w, run, failed };
  });
  const found = JSON.parse(gh(["issue", "list", "-R", repo, "--label", "ci-main", "--state", "open", "--json", "number", "--limit", "1"]))[0];
  const issue = found
    ? (() => {
        const v = JSON.parse(gh(["issue", "view", String(found.number), "-R", repo, "--json", "body,comments"]));
        return { number: found.number, text: [v.body, ...v.comments.map((c) => c.body)].join("\n") };
      })()
    : null;
  const p = plan(latest, issue);
  for (const l of latest) console.log(`${l.name}: ${l.run ? `${l.run.conclusion} ${l.run.head_sha} ${l.run.html_url}` : "none"}${l.failed.length ? ` [${l.failed.join(", ")}]` : ""}`);
  console.log(`issue: ${issue ? `#${issue.number}` : "none"}; ${p.comments.length} comment(s)${p.open ? ", open" : ""}${p.close ? ", close" : ""}`);
  if (dry) return console.log(p.comments.join("\n---\n"));
  let number = issue?.number;
  const [first, ...rest] = p.comments;
  if (p.open) {
    try { gh(["label", "create", "ci-main", "-R", repo, "--color", "B60205", "--description", "CI is red on main"]); } catch {}
    const url = gh(["issue", "create", "-R", repo, "--title", "CI is red on main", "--label", "ci-main", "--body",
      `${first}\n\nPull requests run no builds or tests; \`main\` is where a break shows up first. Fix forward: this issue closes itself once the latest finished run of every workflow on \`main\` is green.`]).trim();
    number = Number(url.split("/").pop());
  } else if (first) gh(["issue", "comment", String(number), "-R", repo, "--body", first]);
  for (const c of rest) gh(["issue", "comment", String(number), "-R", repo, "--body", c]);
  if (p.close) gh(["issue", "close", String(number), "-R", repo, "--reason", "completed"]);
}

function die(msg) {
  console.error(msg);
  process.exit(1);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) main();
