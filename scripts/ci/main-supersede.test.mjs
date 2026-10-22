import { test } from "node:test";
import assert from "node:assert/strict";
import { decide, starvingMs } from "./main-supersede.mjs";

const H = 3600e3;
const NOW = Date.parse("2026-10-22T12:00:00Z");
const at = (ms) => new Date(NOW - ms).toISOString();
// Newest first, like the API.
const run = (n, status, conclusion, createdAgo, updatedAgo = createdAgo, attempt = 1) => ({
  id: 1000 + n,
  run_number: n,
  status,
  conclusion,
  run_attempt: attempt,
  created_at: at(createdAgo),
  updated_at: at(updatedAgo),
});

test("the running run is cancelled when a newer one waits", () => {
  const runs = [run(5, "pending", null, 0.01 * H), run(4, "in_progress", null, 0.2 * H), run(3, "completed", "success", 1 * H, 0.3 * H)];
  const d = decide(runs, NOW);
  assert.deepEqual(d.cancel.map((r) => r.id), [1004]);
  assert.match(d.reason, /superseded by run 1005/);
});

test("the newest run alone is never cancelled", () => {
  assert.deepEqual(decide([run(4, "in_progress", null, 0.2 * H), run(3, "completed", "failure", 1 * H, 0.5 * H)], NOW).cancel, []);
  assert.deepEqual(decide([], NOW).cancel, []);
});

test("cancelled runs are no signal: the guard counts from the last green or red run", () => {
  const runs = [
    run(9, "pending", null, 0.01 * H),
    run(8, "in_progress", null, 0.1 * H),
    run(7, "completed", "cancelled", 1 * H, 0.1 * H),
    run(6, "completed", "cancelled", 2.2 * H, 1 * H),
    run(5, "completed", "success", 3 * H, 2.5 * H),
  ];
  // The streak started 2.2 h ago (run 6), the last finished run 2.5 h ago: starving.
  const d = decide(runs, NOW);
  assert.deepEqual(d.cancel, []);
  assert.match(d.reason, /no finished run for 2\.2 h/);
  // A shorter streak: cancelled.
  assert.deepEqual(decide(runs, NOW, 3 * H).cancel.map((r) => r.id), [1008]);
});

test("a quiet main is not starving: the clock starts at the first push after the last finished run", () => {
  const runs = [run(5, "pending", null, 0.01 * H), run(4, "in_progress", null, 0.05 * H), run(3, "completed", "success", 10 * H, 9.5 * H)];
  assert.deepEqual(decide(runs, NOW).cancel.map((r) => r.id), [1004]);
});

test("re-runs are left alone, and nothing finished in sight means let it finish", () => {
  const rerun = [run(5, "pending", null, 0.01 * H), run(4, "in_progress", null, 0.5 * H, 0.1 * H, 2), run(3, "completed", "success", 1 * H, 0.6 * H)];
  assert.deepEqual(decide(rerun, NOW).cancel, []);
  const none = [run(2, "pending", null, 0.01 * H), run(1, "in_progress", null, 0.2 * H)];
  assert.deepEqual(decide(none, NOW).cancel, []);
});

test("a run past 70% of a typical run is let finish; one earlier on is cancelled", () => {
  const M = 60e3;
  // Finished runs of 11-13 min (their median: 12).
  const finished = [
    run(3, "completed", "failure", 0.9 * H, 0.9 * H - 12 * M),
    run(2, "completed", "success", 1.2 * H, 1.2 * H - 11 * M),
    run(1, "completed", "success", 1.5 * H, 1.5 * H - 13 * M),
  ];
  const going = (startedAgo) => ({ ...run(4, "in_progress", null, 0.5 * H), first_job_at: at(startedAgo) });
  const nearly = decide([run(5, "pending", null, 0.01 * H), going(9 * M), ...finished], NOW);
  assert.deepEqual(nearly.cancel, []);
  assert.match(nearly.reason, /nearly done \(1004 ~75%/);
  const early = decide([run(5, "pending", null, 0.01 * H), going(5 * M), ...finished], NOW);
  assert.deepEqual(early.cancel.map((r) => r.id), [1004]);
  // No start time (its jobs haven't started): judged as just started.
  const waiting = decide([run(5, "pending", null, 0.01 * H), run(4, "in_progress", null, 0.5 * H), ...finished], NOW);
  assert.deepEqual(waiting.cancel.map((r) => r.id), [1004]);
});

test("a run already cancelling is neither cancelled again nor let finish; its successor is judged alone", () => {
  const M = 60e3;
  const finished = [run(2, "completed", "success", 1.2 * H, 1.2 * H - 12 * M), run(1, "completed", "success", 1.5 * H, 1.5 * H - 12 * M)];
  // Run 4 was cancelled 20 min in and is still winding down; run 5 runs (5 min in); run 6 waits.
  const cancelling = { ...run(4, "in_progress", null, 0.5 * H), first_job_at: at(20 * M), cancelling: true };
  const running = { ...run(5, "queued", null, 0.2 * H), first_job_at: at(5 * M) };
  const d = decide([run(6, "pending", null, 0.01 * H), running, cancelling, ...finished], NOW);
  assert.deepEqual(d.cancel.map((r) => r.id), [1005]);
  // Run 5 nearly done: nothing is cancelled.
  const late = { ...running, first_job_at: at(10 * M) };
  assert.deepEqual(decide([run(6, "pending", null, 0.01 * H), late, cancelling, ...finished], NOW).cancel, []);
  // An older run nearly done: a run newer than it (its successor) isn't cancelled either.
  const older = { ...run(3, "in_progress", null, 0.6 * H), first_job_at: at(10 * M) };
  assert.deepEqual(decide([run(6, "pending", null, 0.01 * H), running, older, ...finished], NOW).cancel, []);
});

test("the guard is two typical runs, within 30 min and 2 h", () => {
  const M = 60e3;
  const fin = (mins) => mins.map((m, i) => run(10 - i, "completed", "success", (i + 1) * H, (i + 1) * H - m * M));
  assert.equal(starvingMs(fin([15, 14, 16])), 30 * M);
  assert.equal(starvingMs(fin([12, 11, 13])), 30 * M);
  assert.equal(starvingMs(fin([28, 27, 30])), 56 * M);
  assert.equal(starvingMs(fin([80, 90, 70])), 2 * H);
  assert.equal(starvingMs([]), 2 * H);
  // A streak of merges 3 min apart for 40 min after a green run of ~15 min: the running run
  // (5 min in, far from 70%) is let finish, where a flat 2 h guard would cancel it.
  const finished = [run(2, "completed", "success", 55 * M, 41 * M), run(1, "completed", "success", 2 * H, 2 * H - 15 * M)];
  const streak = [run(9, "pending", null, 0.01 * H), { ...run(8, "in_progress", null, 6 * M), first_job_at: at(5 * M) }];
  for (let n = 7; n >= 3; n--) streak.push(run(n, "completed", "cancelled", (40 - (n - 3) * 3) * M, (37 - (n - 3) * 3) * M));
  const d = decide([...streak, ...finished], NOW);
  assert.deepEqual(d.cancel, []);
  assert.match(d.reason, /no finished run for 0\.7 h \(guard 30 min\)/);
  assert.deepEqual(decide([...streak, ...finished], NOW, 2 * H).cancel.map((r) => r.id), [1008]);
});
