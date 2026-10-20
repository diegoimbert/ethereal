import { test } from "node:test";
import assert from "node:assert/strict";
import { isHashedExecutable, stale } from "./sweep-test-binaries.mjs";

test("only <name>-<16 hex> executables count", () => {
  assert.ok(isHashedExecutable("ether_core-0123456789abcdef", 0o755));
  assert.ok(!isHashedExecutable("ether_core-0123456789abcdef", 0o644));
  assert.ok(!isHashedExecutable("libether_core-0123456789abcdef.rlib", 0o755));
  assert.ok(!isHashedExecutable("ether_core-0123456789abcdef.d", 0o755));
});

test("keeps the listed binaries (by inode, so hard links count) and deletes the rest", () => {
  const files = {
    "/t/debug/deps/a-0000000000000001": { ino: 1, mode: 0o755, size: 10 }, // listed test binary
    "/t/debug/deps/a-0000000000000002": { ino: 2, mode: 0o755, size: 20 }, // stale
    "/t/debug/deps/tool-0000000000000003": { ino: 3, mode: 0o755, size: 30 }, // hard-linked bin
    "/t/debug/tool": { ino: 3, mode: 0o755, size: 30 },
    "/t/debug/deps/liba-0000000000000001.rlib": { ino: 4, mode: 0o644, size: 40 },
  };
  const fs = {
    readdirSync: (d) =>
      Object.keys(files)
        .filter((p) => p.startsWith(d + "/") && !p.slice(d.length + 1).includes("/"))
        .map((p) => p.slice(d.length + 1)),
    statSync: (p) => {
      const f = files[p];
      if (!f) throw new Error("ENOENT");
      return { dev: 1, ino: f.ino, mode: f.mode, size: f.size, isFile: () => true };
    },
  };
  const list = {
    "rust-build-meta": { "target-directory": "/t", "non-test-binaries": { pkg: [{ name: "tool", path: "debug/tool" }] } },
    "rust-binaries": { a: { "binary-path": "/t/debug/deps/a-0000000000000001" } },
  };
  assert.deepEqual(stale(list, ["/t/debug/deps", "/t/missing/deps"], fs), [{ path: "/t/debug/deps/a-0000000000000002", size: 20 }]);
});

test("several runs: --also splits them, the merged list keeps the binaries of each", async () => {
  const { splitRuns, mergeLists } = await import("./sweep-test-binaries.mjs");
  assert.deepEqual(splitRuns(["--workspace", "--all-features", "--also", "-p", "x"]), [["--workspace", "--all-features"], ["-p", "x"]]);
  const m = mergeLists([
    { "rust-build-meta": { "target-directory": "/t", "base-output-directories": ["debug"], "non-test-binaries": { a: [{ path: "debug/a" }] } }, "rust-binaries": { a: { "binary-path": "/t/debug/deps/a-1" } } },
    { "rust-build-meta": { "target-directory": "/t", "base-output-directories": ["debug"], "non-test-binaries": {} }, "rust-binaries": { b: { "binary-path": "/t/debug/deps/b-2" } } },
  ]);
  assert.deepEqual(Object.keys(m["rust-binaries"]).sort(), ["a", "b"]);
  assert.deepEqual(m["rust-build-meta"]["base-output-directories"], ["debug"]);
  assert.equal(m["rust-build-meta"]["non-test-binaries"].a.length, 1);
});
