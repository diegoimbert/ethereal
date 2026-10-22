import { test } from "node:test";
import assert from "node:assert/strict";
import { digest, due } from "./ee-sync-watch.mjs";

const ls = (...refs) => refs.map(([ref, sha]) => `${sha}\t${ref}`).join("\n") + "\n";
const M = 60e3;

test("digest: main apart from the other branches; lock heartbeats are no change", () => {
  const a = digest(ls(["refs/heads/main", "o1"], ["refs/heads/x", "x1"]), ls(["refs/heads/main", "e1"], ["refs/locks/ship", "l1"]));
  const beat = digest(ls(["refs/heads/x", "x1"], ["refs/heads/main", "o1"]), ls(["refs/heads/main", "e1"], ["refs/locks/ship", "l2"]));
  assert.deepEqual(beat, a);
  const pr = digest(ls(["refs/heads/main", "o1"], ["refs/heads/x", "x2"]), ls(["refs/heads/main", "e1"], ["refs/locks/ship", "l1"]));
  assert.equal(pr.main, a.main);
  assert.notEqual(pr.all, a.all);
  const eeMain = digest(ls(["refs/heads/main", "o1"], ["refs/heads/x", "x1"]), ls(["refs/heads/main", "e2"], ["refs/locks/ship", "l1"]));
  assert.notEqual(eeMain.main, a.main);
  const released = digest(ls(["refs/heads/main", "o1"], ["refs/heads/x", "x1"]), ls(["refs/heads/main", "e1"]));
  assert.notEqual(released.all, a.all);
});

test("due: main at once, other branches spaced, periodic, failures backed off", () => {
  const last = { main: "m", all: "a" };
  assert.equal(due({ now: last, last: null, sinceSync: 0, failedAgo: null }), "start");
  assert.equal(due({ now: last, last, sinceSync: 1 * M, failedAgo: null }), null);
  assert.equal(due({ now: { main: "m2", all: "a2" }, last, sinceSync: 0.2 * M, failedAgo: null }), "main moved");
  assert.equal(due({ now: { main: "m", all: "a2" }, last, sinceSync: 1 * M, failedAgo: null }), null);
  assert.equal(due({ now: { main: "m", all: "a2" }, last, sinceSync: 3 * M, failedAgo: null }), "a branch moved");
  assert.equal(due({ now: last, last, sinceSync: 15 * M, failedAgo: null }), "periodic");
  assert.equal(due({ now: { main: "m2", all: "a2" }, last, sinceSync: 1 * M, failedAgo: 1 * M }), null);
  assert.equal(due({ now: { main: "m2", all: "a2" }, last, sinceSync: 2 * M, failedAgo: 2 * M }), "main moved");
});
