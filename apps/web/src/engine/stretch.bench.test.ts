// Stretcher cost per AudioWorklet quantum in real wasm (V8): one stereo voice of Complex
// (Signalsmith Stretch's Rust port, web preset) and of Complex+, ×1.25, and Complex+ at +3 st
// with formants kept (crates/ether-wasm/src/perf.rs `measure_stretch`). Needs a wasm build
// with the `bench` feature (`just bench-wasm`); skipped without it. Prints the numbers of
// docs/WARP.md.
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const pkg = fileURLToPath(new URL("../wasm/pkg/", import.meta.url));
// The `bench_stretch` export is only in builds with the `bench` feature.
const built =
  existsSync(`${pkg}ether_wasm_bg.wasm`) &&
  existsSync(`${pkg}ether_wasm.js`) &&
  readFileSync(`${pkg}ether_wasm.js`, "utf8").includes("export function bench_stretch(");

interface StretchCosts {
  complex_quantum_ms: number;
  complex_worst_ms: number;
  plus_quantum_ms: number;
  plus_worst_ms: number;
  plus_transposed_quantum_ms: number;
  plus_transposed_worst_ms: number;
  quantum_budget_ms: number;
}

describe.skipIf(!built)("stretchers in wasm", () => {
  it("a Complex+ voice fits the quantum many times over", async () => {
    const wasm = await import(/* @vite-ignore */ `${pkg}ether_wasm.js`);
    wasm.initSync({ module: new WebAssembly.Module(readFileSync(`${pkg}ether_wasm_bg.wasm`)) });
    const c = JSON.parse(wasm.bench_stretch(3)) as StretchCosts;
    const f = (ms: number) => `${(ms * 1000).toFixed(1)} µs`;
    console.log(
      `stretch (wasm/V8, per 128-frame stereo quantum, mean / worst): complex ${f(c.complex_quantum_ms)} / ${f(c.complex_worst_ms)}; ` +
        `complex+ ${f(c.plus_quantum_ms)} / ${f(c.plus_worst_ms)}; complex+ +3 st ${f(c.plus_transposed_quantum_ms)} / ${f(c.plus_transposed_worst_ms)}; ` +
        `budget ${c.quantum_budget_ms.toFixed(3)} ms`,
    );
    // The web's voice limit (12) of Complex+ voices stays within half the quantum on average.
    expect(c.plus_quantum_ms * 12).toBeLessThan(c.quantum_budget_ms / 2);
  }, 300_000);
});
