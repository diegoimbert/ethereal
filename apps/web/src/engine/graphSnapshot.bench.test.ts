// Graph snapshot cost in real wasm (V8, as in the AudioWorklet): the large fixture (64
// tracks, 500 clips, automation) through the binary `Publish` path, next to the old JSON
// decode (crates/ether-wasm/src/perf.rs). Needs the wasm build (scripts/build-wasm.mjs);
// skipped without it. Prints the numbers used in the web-perf PR.
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const pkg = fileURLToPath(new URL("../wasm/pkg/", import.meta.url));
const built = existsSync(`${pkg}ether_wasm_bg.wasm`);

interface SnapshotCosts {
  tracks: number;
  clips: number;
  json_bytes: number;
  binary_bytes: number;
  json_encode_ms: number;
  json_decode_ms: number;
  binary_encode_ms: number;
  binary_decode_ms: number;
  compile_ms: number;
  publish_quantum_ms: number;
  quantum_budget_ms: number;
}

/**
 * Decode bound for the large fixture in wasm: 3x the native `DECODE_BOUND`
 * (crates/ether-core/tests/codec_alloc.rs, 5 ms). Measured ~0.6 ms; fastest of 11 runs.
 */
const WASM_DECODE_BOUND_MS = 15;

describe.skipIf(!built)("graph snapshot in wasm", () => {
  it("binary decode is bounded and beats JSON", async () => {
    const wasm = await import(/* @vite-ignore */ `${pkg}ether_wasm.js`);
    wasm.initSync({ module: new WebAssembly.Module(readFileSync(`${pkg}ether_wasm_bg.wasm`)) });
    const c = JSON.parse(wasm.bench_graph_snapshot(11)) as SnapshotCosts;
    const f = (ms: number) => `${ms.toFixed(3)} ms`;
    console.log(
      [
        `web-perf (wasm/V8): large fixture (${c.tracks} tracks, ${c.clips} clips)`,
        "| step | JSON (before) | binary (after) |",
        "|---|---|---|",
        `| size | ${c.json_bytes} B | ${c.binary_bytes} B |`,
        `| encode (Worker) | ${f(c.json_encode_ms)} | ${f(c.binary_encode_ms)} |`,
        `| decode (Worklet) | ${f(c.json_decode_ms)} | ${f(c.binary_decode_ms)} |`,
        `compile: ${f(c.compile_ms)}; worst publish quantum (after): ${f(c.publish_quantum_ms)}; budget ${f(c.quantum_budget_ms)}`,
      ].join("\n"),
    );
    expect(c.binary_decode_ms).toBeLessThan(c.json_decode_ms);
    expect(c.binary_decode_ms).toBeLessThan(WASM_DECODE_BOUND_MS);
  }, 60_000);
});
