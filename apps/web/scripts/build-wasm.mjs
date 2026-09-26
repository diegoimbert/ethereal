#!/usr/bin/env node
// Build crates/ether-wasm for the browser: cargo (wasm32-unknown-unknown) + wasm-bindgen
// (--target web) into apps/web/src/wasm/pkg (git-ignored). Used by `pnpm dev`/`build`/`test:e2e`.
//
//   ETHER_WASM_PROFILE=release|dev   cargo profile (default: release; the engine is DSP code)
//   ETHER_WASM_SKIP=1                skip (use the existing pkg)
//
// wasm-bindgen-cli must match the `wasm-bindgen` version in Cargo.lock:
//   cargo install wasm-bindgen-cli --version <version>
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, "../../..");
const outDir = resolve(here, "../src/wasm/pkg");

if (process.env.ETHER_WASM_SKIP === "1" && existsSync(join(outDir, "ether_wasm_bg.wasm"))) {
  console.log("ether-wasm: skipped (ETHER_WASM_SKIP=1)");
  process.exit(0);
}

const profile = process.env.ETHER_WASM_PROFILE === "dev" ? "dev" : "release";
const lock = readFileSync(join(repo, "Cargo.lock"), "utf8");
const want = /name = "wasm-bindgen"\nversion = "([^"]+)"/.exec(lock)?.[1];

let have = null;
try {
  have = execFileSync("wasm-bindgen", ["--version"], { encoding: "utf8" }).trim().split(" ")[1];
} catch {
  // not installed
}
if (!have || (want && have !== want)) {
  console.error(
    `ether-wasm: wasm-bindgen-cli ${want ?? ""} is required (found ${have ?? "none"}).\n` +
      `  cargo install wasm-bindgen-cli --version ${want ?? "<see Cargo.lock>"}`,
  );
  process.exit(1);
}

const run = (cmd, args) => execFileSync(cmd, args, { cwd: repo, stdio: "inherit" });

run("cargo", ["build", "-p", "ether-wasm", "--target", "wasm32-unknown-unknown", "--profile", profile]);
const targetDir = JSON.parse(
  execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps"], { cwd: repo, encoding: "utf8" }),
).target_directory;
const dir = profile === "dev" ? "debug" : "release";
const wasm = join(targetDir, "wasm32-unknown-unknown", dir, "ether_wasm.wasm");
run("wasm-bindgen", ["--target", "web", "--out-dir", outDir, "--out-name", "ether_wasm", wasm]);
console.log(`ether-wasm: built (${profile}) → ${outDir}`);
