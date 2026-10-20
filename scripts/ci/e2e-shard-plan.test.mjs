import assert from "node:assert/strict";
import { test } from "node:test";
import { parseList, parseResults, plan } from "./e2e-shard-plan.mjs";

const LIST = `Listing tests:
  [webgl] › ../e2e/a.spec.ts:3:1 › a one
  [webgl] › ../e2e/a.spec.ts:9:1 › a two
  [webgl] › ../e2e/b.spec.ts:3:1 › suite › b one
  [canvas2d] › ../e2e/a.spec.ts:3:1 › a one
  [tours] › ../e2e/t.ts:5:9 › tours › t one
Total: 5 tests in 3 files`;

test("the list's tests, grouped by project and file", () => {
  const t = parseList(LIST);
  assert.equal(t.length, 5);
  assert.equal(t[2].line, "[webgl] › ../e2e/b.spec.ts:3:1 › suite › b one");
  assert.deepEqual(
    [...new Set(t.map((x) => x.group))],
    ["[webgl] › ../e2e/a.spec.ts", "[webgl] › ../e2e/b.spec.ts", "[canvas2d] › ../e2e/a.spec.ts", "[tours] › ../e2e/t.ts"],
  );
});

test("a log's times per group (ms, s, m; retries add up)", () => {
  const times = parseResults(`  ✓    1 [webgl] › ../e2e/a.spec.ts:3:1 › a one (3.5s)
  ✘    2 [webgl] › ../e2e/a.spec.ts:9:1 › a two (500ms)
  ✓    3 [webgl] › ../e2e/a.spec.ts:9:1 › a two (retry #1) (1.0s)
  ✓    4 [tours] › ../e2e/t.ts:5:9 › tours › t one (1.5m)
  some other line (2s)`);
  assert.deepEqual(times, { "[webgl] › ../e2e/a.spec.ts": 5, "[tours] › ../e2e/t.ts": 90 });
});

test("every test lands in exactly one shard; the long groups are spread", () => {
  const tests = parseList(LIST);
  const shards = plan(tests, { "[tours] › ../e2e/t.ts": 100, "[webgl] › ../e2e/a.spec.ts": 60 }, 2);
  assert.deepEqual(shards.flatMap((s) => s.lines).sort(), tests.map((t) => t.line).sort());
  // The two measured groups (100 s, 60 s) go to different shards; the unknown ones count their
  // tests times the measured mean (160 s / 3 tests each).
  const shardOf = (prefix) => shards.findIndex((x) => x.lines.some((l) => l.startsWith(prefix)));
  assert.notEqual(shardOf("[tours]"), shardOf("[webgl] › ../e2e/a.spec.ts"));
  assert.equal(Math.round(shards[0].secs + shards[1].secs), Math.round(160 + 2 * (160 / 3)));
  assert.ok(Math.abs(shards[0].secs - shards[1].secs) <= 100);
});
