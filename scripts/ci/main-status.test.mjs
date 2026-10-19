import { test } from "node:test";
import assert from "node:assert/strict";
import { latestFinished, plan } from "./main-status.mjs";

const run = (id, conclusion) => ({ id, status: "completed", conclusion, head_sha: `${id}`.padEnd(40, "0"), html_url: `https://x/runs/${id}` });

test("a skipped (cancelled) run says nothing about main", () => {
  assert.equal(latestFinished([run(3, "cancelled"), run(2, "failure"), run(1, "success")]).id, 2);
  assert.equal(latestFinished([run(1, "cancelled"), run(0, "skipped")]), null);
});

test("red with no issue opens one", () => {
  const p = plan([{ name: "CI", run: run(5, "failure"), failed: ["Rust (Linux)"] }, { name: "Full product", run: run(4, "success"), failed: [] }], null);
  assert.equal(p.open, true);
  assert.equal(p.comments.length, 1);
  assert.match(p.comments[0], /CI failed on `main`.*runs\/5/);
  assert.match(p.comments[0], /`Rust \(Linux\)`/);
  assert.match(p.comments[0], /Full product: green/);
});

test("a red run the issue already mentions is not repeated", () => {
  const latest = [{ name: "CI", run: run(5, "failure"), failed: [] }, { name: "Full product", run: run(6, "failure"), failed: [], logs: (id) => `logs ${id}` }];
  const p = plan(latest, { number: 98, text: "CI failed: https://x/runs/5" });
  assert.deepEqual([p.open, p.close, p.comments.length], [false, false, 1]);
  assert.match(p.comments[0], /^Full product failed.*runs\/6 \(logs 6\)/);
});

test("all green closes the issue, and does nothing without one", () => {
  const latest = [{ name: "CI", run: run(7, "success"), failed: [] }, { name: "Full product", run: null, failed: [] }];
  const p = plan(latest, { number: 98, text: "" });
  assert.deepEqual([p.open, p.close, p.comments.length], [false, true, 1]);
  assert.deepEqual(plan(latest, null), { comments: [], open: false, close: false });
});
