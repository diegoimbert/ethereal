#!/usr/bin/env node
// Build the test WebCLAP (crates/ether-webclap-fixture: a gain with a web UI, a delay, a
// synth) into apps/web/test-fixtures/webclap/ether-test.wclap.wasm (git-ignored). Used by
// the host glue's unit tests (src/engine/webclap/*.test.ts) and the e2e (e2e/webclap.spec.ts).
//
//   node scripts/build-webclap-fixture.mjs [--if-missing]
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "../../..");
export const FIXTURE = resolve(here, "../test-fixtures/webclap/ether-test.wclap.wasm");

export function buildFixture({ ifMissing = false } = {}) {
  if (ifMissing && existsSync(FIXTURE)) return FIXTURE;
  execFileSync("cargo", ["build", "-p", "ether-webclap-fixture", "--target", "wasm32-unknown-unknown", "--release"], {
    cwd: repo,
    stdio: "inherit",
  });
  const targetDir = JSON.parse(execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], { cwd: repo, encoding: "utf8" })).target_directory;
  mkdirSync(dirname(FIXTURE), { recursive: true });
  copyFileSync(join(targetDir, "wasm32-unknown-unknown", "release", "ether_webclap_fixture.wasm"), FIXTURE);
  return FIXTURE;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  console.log(`webclap fixture: ${buildFixture({ ifMissing: process.argv.includes("--if-missing") })}`);
}
