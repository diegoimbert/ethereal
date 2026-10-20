import assert from "node:assert/strict";
import { test } from "node:test";
import { stampMatches, trackedPaths } from "./e2e-shared-target.mjs";

test("tracked files and their directories below each base (not the bases)", () => {
  const got = trackedPaths([
    ["/w", "a/b/c.rs\0top.toml\0"],
    ["/w/ee", "pkg/src/lib.rs\0"],
  ]).sort();
  assert.deepEqual(got, ["/w/a", "/w/a/b", "/w/a/b/c.rs", "/w/ee/pkg", "/w/ee/pkg/src", "/w/ee/pkg/src/lib.rs", "/w/top.toml"]);
});

test("a stamp matches only the same commits", () => {
  const want = { oss: "a", ee: "b" };
  assert.equal(stampMatches({ oss: "a", ee: "b", t0: 5 }, want), true);
  assert.equal(stampMatches({ oss: "a", ee: "c", t0: 5 }, want), false);
  assert.equal(stampMatches({ oss: "a", ee: "b" }, want), false);
  assert.equal(stampMatches(null, want), false);
});
