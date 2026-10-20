// A crate whose integration tests are one binary (crates/<crate>/tests/it/main.rs) declares every
// file of tests/it/ as a module: cargo compiles only what main.rs declares, so a file without its
// `mod` line would never run.
import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

const roots = ["crates", "ee/crates"].filter((d) => existsSync(d));

test("every tests/it/*.rs file is a module of tests/it/main.rs", () => {
  for (const root of roots)
    for (const crate of readdirSync(root)) {
      const dir = join(root, crate, "tests", "it");
      if (!existsSync(join(dir, "main.rs"))) continue;
      const main = readFileSync(join(dir, "main.rs"), "utf8");
      const mods = new Set([...main.matchAll(/^\s*(?:pub\s+)?mod\s+(\w+)\s*;/gm)].map((m) => m[1]));
      const missing = readdirSync(dir)
        .filter((f) => f.endsWith(".rs") && f !== "main.rs")
        .map((f) => f.slice(0, -3))
        .filter((m) => !mods.has(m));
      assert.deepEqual(missing, [], `${dir}/main.rs: add \`mod ${missing[0]};\``);
    }
});
