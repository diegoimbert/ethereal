import { test } from "node:test";
import assert from "node:assert/strict";
import { describe, lockDrift, manifestDeps, parseLock, staleTypes } from "./ee-pair-check.mjs";

const m = (o) => new Map(Object.entries(o));

test("a changed shared type must be the same file in EE", () => {
  const ossBase = m({ "A.ts": "a1", "index.ts": "i1" });
  const eeBase = m({ "A.ts": "a1", "index.ts": "i1+ee", "EeX.ts": "x" });
  const ossHead = m({ "A.ts": "a2", "index.ts": "i1" });
  assert.deepEqual(staleTypes({ ossBase, ossHead, eeBase, eeHead: eeBase }), ["A.ts"]);
  assert.deepEqual(staleTypes({ ossBase, ossHead, eeBase, eeHead: m({ "A.ts": "a2", "index.ts": "i1+ee", "EeX.ts": "x" }) }), []);
  // regenerated from another OSS version: still stale
  assert.deepEqual(staleTypes({ ossBase, ossHead, eeBase, eeHead: m({ "A.ts": "a3", "index.ts": "i1+ee" }) }), ["A.ts"]);
});

test("a new type: EE has it too; an extended file (index.ts) must have been regenerated", () => {
  const ossBase = m({ "index.ts": "i1" });
  const eeBase = m({ "index.ts": "i1+ee" });
  const ossHead = m({ "index.ts": "i2", "New.ts": "n" });
  assert.deepEqual(staleTypes({ ossBase, ossHead, eeBase, eeHead: eeBase }), ["New.ts", "index.ts"]);
  assert.deepEqual(staleTypes({ ossBase, ossHead, eeBase, eeHead: m({ "index.ts": "i2+ee", "New.ts": "n" }) }), []);
  // a new type the EE side extends from the start
  assert.deepEqual(staleTypes({ ossBase, ossHead, eeBase, eeHead: m({ "index.ts": "i2+ee", "New.ts": "n+ee" }) }), []);
});

test("a removed type must be gone from EE (or EE's own now)", () => {
  const ossBase = m({ "Old.ts": "o" });
  const eeBase = m({ "Old.ts": "o" });
  assert.deepEqual(staleTypes({ ossBase, ossHead: m({}), eeBase, eeHead: eeBase }), ["Old.ts"]);
  assert.deepEqual(staleTypes({ ossBase, ossHead: m({}), eeBase, eeHead: m({}) }), []);
});

test("files the PR leaves alone are not checked (main may already be stale)", () => {
  const oss = m({ "A.ts": "a2" });
  assert.deepEqual(staleTypes({ ossBase: oss, ossHead: oss, eeBase: m({ "A.ts": "a1" }), eeHead: m({ "A.ts": "a1" }) }), []);
});

const pkg = (name, version, deps = [], source = null) =>
  `[[package]]\nname = "${name}"\nversion = "${version}"\n${source ? `source = "${source}"\n` : ""}${deps.length ? `dependencies = [\n${deps.map((d) => ` "${d}",`).join("\n")}\n]\n` : ""}`;
const REG = "registry+https://github.com/rust-lang/crates.io-index";
const lock = (...p) => `version = 4\n\n${p.join("\n")}`;

test("parseLock reads names, versions, sources and dependencies", () => {
  const l = parseLock(lock(pkg("ether-stretch", "0.0.1", ["cc", "rustfft"]), pkg("cc", "1.9.0", [], REG), pkg("rustfft", "6.4.0", [], REG)));
  assert.deepEqual(l.map((p) => [p.name, p.version, !!p.source, p.deps]), [
    ["ether-stretch", "0.0.1", false, ["cc", "rustfft"]],
    ["cc", "1.9.0", true, []],
    ["rustfft", "6.4.0", true, []],
  ]);
});

test("a dependency an OSS crate gains must be in ee/Cargo.lock", () => {
  const base = lock(pkg("ether-stretch", "0.0.1", ["rustfft"]), pkg("rustfft", "6.4.0", [], REG));
  const head = lock(pkg("ether-stretch", "0.0.1", ["realfft", "rustfft"]), pkg("realfft", "3.5.0", [], REG), pkg("rustfft", "6.4.0", [], REG));
  const normal = () => true;
  assert.deepEqual(lockDrift({ ossBase: base, ossHead: head, eeBase: base, eeHead: base }, normal), ["ether-stretch: +realfft 3.5.0"]);
  assert.deepEqual(lockDrift({ ossBase: base, ossHead: head, eeBase: base, eeHead: head }, normal), []);
  // a dev-dependency (not in the manifest's normal ones) is not resolved in ee/
  assert.deepEqual(lockDrift({ ossBase: base, ossHead: head, eeBase: base, eeHead: base }, () => false), []);
});

test("version bumps and drift already on main are not reported; a dropped dependency is", () => {
  const ee = lock(pkg("ether-core", "0.0.1", ["serde_json", "old"]), pkg("serde_json", "1.0.151", [], REG), pkg("old", "1.0.0", [], REG));
  const ossBase = lock(pkg("ether-core", "0.0.1", ["serde_json", "old"]), pkg("serde_json", "1.0.154", [], REG), pkg("old", "1.0.0", [], REG));
  const ossHead = lock(pkg("ether-core", "0.0.1", ["serde_json"]), pkg("serde_json", "1.0.155", [], REG));
  assert.deepEqual(lockDrift({ ossBase, ossHead, eeBase: ee, eeHead: ee }), ["ether-core: -old 1.0.0"]);
  // crates ee/ does not build are skipped
  const other = lock(pkg("ether-x", "0.0.1", ["serde_json"]), pkg("serde_json", "1.0.154", [], REG));
  assert.deepEqual(lockDrift({ ossBase: other, ossHead: lock(pkg("ether-x", "0.0.1", ["rand"]), pkg("rand", "0.9.0", [], REG)), eeBase: ee, eeHead: ee }), []);
});

test("manifestDeps: normal and build deps, optional ones only when a default feature turns them on", () => {
  const toml = `[package]
name = "ether-stretch"
version.workspace = true

[features]
default = ["signalsmith", "rust-stretch"]
signalsmith = []
rust-stretch = [
  "dep:rustfft",
  "dep:realfft",
]
extra = ["dep:never"]

[dependencies]
rustfft = { workspace = true, optional = true }
realfft = { workspace = true, optional = true }
never = { version = "1", optional = true }
serde.workspace = true # a comment

[dependencies.thiserror]
version = "2"

[target.'cfg(windows)'.dependencies]
windows = "0.61"

[dev-dependencies]
proptest.workspace = true

[build-dependencies]
cc = "1"
`;
  const r = manifestDeps(toml);
  assert.equal(r.name, "ether-stretch");
  assert.deepEqual([...r.deps].sort(), ["cc", "realfft", "rustfft", "serde", "thiserror", "windows"]);
});

test("describe fits a status description", () => {
  const d = describe("EE app types not regenerated", ["A.ts", "B.ts", "C.ts", "D.ts"], "cd ee && just gen-types; commit; just feat push");
  assert.equal(d, "EE app types not regenerated (A.ts, B.ts +2): cd ee && just gen-types; commit; just feat push");
  assert.ok(describe("x", ["y".repeat(300)], "z").length <= 139);
});
