// Worklet render cost per quantum in real wasm (V8) for a few playing projects: the engine
// alone (empty tracks), a synth per track, near-free devices (per-device overhead) and a
// small mix (crates/ether-wasm/src/perf.rs `measure_render`). Needs a wasm build with the
// `bench` feature (`just bench-wasm`); skipped without it.
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const pkg = fileURLToPath(new URL("../wasm/pkg/", import.meta.url));
// The `bench_render` export is only in builds with the `bench` feature.
const built =
  existsSync(`${pkg}ether_wasm_bg.wasm`) &&
  existsSync(`${pkg}ether_wasm.js`) &&
  readFileSync(`${pkg}ether_wasm.js`, "utf8").includes("export function bench_render(");

interface RenderCosts {
  scenarios: { scenario: string; tracks: number; devices: number; mean_ms: number; worst_ms: number }[];
  quantum_budget_ms: number;
}

describe.skipIf(!built)("render in wasm", () => {
  it("every scenario fits the quantum", async () => {
    const wasm = await import(/* @vite-ignore */ `${pkg}ether_wasm.js`);
    wasm.initSync({ module: new WebAssembly.Module(readFileSync(`${pkg}ether_wasm_bg.wasm`)) });
    const c = JSON.parse(wasm.bench_render(3)) as RenderCosts;
    for (const s of c.scenarios) {
      console.log(
        `render (wasm/V8) ${s.scenario}: ${s.tracks} tracks, ${s.devices} devices: mean ${(s.mean_ms * 1000).toFixed(1)} µs, worst ${(s.worst_ms * 1000).toFixed(1)} µs per quantum`,
      );
      expect(s.mean_ms).toBeLessThan(c.quantum_budget_ms);
    }
  }, 300_000);
});
