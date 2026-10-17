// `node --test scripts/ci/*.test.mjs`: what the full-product CI may print of a tool's output.
import assert from "node:assert/strict";
import { test } from "node:test";
import { failures } from "./failures.mjs";

test("names what failed, never the messages", () => {
  const log = [
    "   Compiling secret-crate v0.1.0",
    "error[E0425]: cannot find value `private_thing` in this scope",
    "  --> crates/secret/src/lib.rs:3:5",
    "error: could not compile `secret-crate` (lib) due to 1 previous error",
    "        FAIL [   0.039s] (1090/1895) ether-devices::engine_text ui_engine_text_list_is_current",
    "    thread 'x' panicked at crates/secret/src/lib.rs:9:5: the private detail",
    "test share::tests::keeps_it ... FAILED",
    "  ✘  63 [webgl] › e2e/secret.spec.ts:6:1 › the title of a private feature (11.2s)",
    " FAIL  src/features/secret/x.test.ts > suite > case",
    "src/features/secret/y.ts(1,2): error TS2304: Cannot find name 'privateName'.",
    "[warn] ui/src/z.ts",
    "Diff in /home/runner/work/ethereal/ethereal/crates/secret/src/a.rs:12:",
  ].join("\n");
  assert.deepEqual(failures(log), [
    "compile error in secret-crate",
    "fail ether-devices::engine_text ui_engine_text_list_is_current",
    "fail share::tests::keeps_it",
    "fail [webgl] e2e/secret.spec.ts:6:1",
    "fail src/features/secret/x.test.ts",
    "type error in src/features/secret/y.ts",
    "not formatted: ui/src/z.ts",
    "not formatted: crates/secret/src/a.rs",
  ]);
  const all = failures(log).join("\n");
  for (const secret of ["private_thing", "the private detail", "the title of a private feature", "privateName"]) {
    assert.ok(!all.includes(secret), secret);
  }
});
