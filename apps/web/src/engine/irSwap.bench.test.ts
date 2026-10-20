// Worklet cost of switching a playing convolution reverb to the longest factory IR, in real
// wasm (V8, as in the AudioWorklet), through the real path (WebBridge::update_builtin →
// ring → EngineHost; crates/ether-wasm/src/perf.rs `measure_ir_swap`). The IR is prepared
// on the controller Worker and only assembled in bounded steps on the Worklet. Needs
// a wasm build with the `bench` feature (`just bench-wasm`); skipped without it. Prints the
// numbers used in the PR.
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const pkg = fileURLToPath(new URL("../wasm/pkg/", import.meta.url));
// The `bench_ir_swap` export is only in builds with the `bench` feature.
const built =
  existsSync(`${pkg}ether_wasm_bg.wasm`) &&
  existsSync(`${pkg}ether_wasm.js`) &&
  readFileSync(`${pkg}ether_wasm.js`, "utf8").includes("export function bench_ir_swap(");

interface IrSwapCosts {
  ir: string;
  ir_seconds: number;
  steady_quantum_ms: number;
  swap_quantum_ms: number;
  quantum_budget_ms: number;
}

describe.skipIf(!built)("IR swap in wasm", () => {
  it("switching to the longest factory IR stays within the quantum budget", async () => {
    const wasm = await import(/* @vite-ignore */ `${pkg}ether_wasm.js`);
    wasm.initSync({ module: new WebAssembly.Module(readFileSync(`${pkg}ether_wasm_bg.wasm`)) });
    const c = JSON.parse(wasm.bench_ir_swap(5)) as IrSwapCosts;
    const f = (ms: number) => `${ms.toFixed(3)} ms`;
    console.log(
      `ir-swap (wasm/V8): switch to "${c.ir}" (${c.ir_seconds} s): worst quantum ${f(c.swap_quantum_ms)}; ` +
        `steady ${f(c.steady_quantum_ms)}; budget ${f(c.quantum_budget_ms)}`,
    );
    // The swap's worst quantum stays well inside the budget (the rest is the render).
    expect(c.swap_quantum_ms).toBeLessThan(c.quantum_budget_ms / 2);
  }, 120_000);

  // A user's own IR (media), prepared on the Worker from its audio, vs built by the
  // Worklet in one go as before (`measure_media_ir_swap`).
  it("switching to a 6 s user IR stays within the quantum budget", async () => {
    const wasm = await import(/* @vite-ignore */ `${pkg}ether_wasm.js`);
    wasm.initSync({ module: new WebAssembly.Module(readFileSync(`${pkg}ether_wasm_bg.wasm`)) });
    const c = JSON.parse(wasm.bench_media_ir_swap(3)) as MediaIrSwapCosts;
    const f = (ms: number) => `${ms.toFixed(3)} ms`;
    console.log(
      `media-ir-swap (wasm/V8): switch to a ${c.ir_seconds} s stereo user IR: worst quantum ${f(c.swap_quantum_ms)} ` +
        `(built on the worklet: ${f(c.before_quantum_ms)}); budget ${f(c.quantum_budget_ms)}`,
    );
    expect(c.swap_quantum_ms).toBeLessThan(c.quantum_budget_ms / 2);
  }, 120_000);
});

interface MediaIrSwapCosts {
  ir_seconds: number;
  before_quantum_ms: number;
  swap_quantum_ms: number;
  quantum_budget_ms: number;
}
