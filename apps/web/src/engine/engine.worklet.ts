// AudioWorklet processor: runs `ether-core` (wasm instance #2). Each render quantum the
// Rust side drains the control ring, renders, and writes reports; this file only copies the
// rendered planar output from wasm memory into the Web Audio output buffers.
//
// Output 1 is the stream tap ("listen on <peer>", docs/COLLAB.md §9.1: master + metronome,
// minus the preview voice) for the web sender, and its transport state goes to the tap
// clock buffer (ui/src/features/collab/host/tapClock.ts).
import "./textCodec";
import { initSync, WasmEngine } from "@ether-wasm/ether_wasm.js";
import { HEADER, TAP_CLOCK_SLOT, TAP_OUTPUT_BASE, TapClockWriter } from "@/features/collab/host/tapClock";

// AudioWorkletGlobalScope (not in lib.dom).
declare const sampleRate: number;
declare const currentFrame: number;
declare function registerProcessor(name: string, ctor: new (options: ProcessorOptions) => unknown): void;
declare class AudioWorkletProcessor {
  readonly port: MessagePort;
  constructor();
}
interface ProcessorOptions {
  processorOptions: {
    module: WebAssembly.Module;
    control: SharedArrayBuffer;
    reports: SharedArrayBuffer;
    /** Tap clock (`tapClockBuffer()`); absent: no tap output. */
    tapClock?: SharedArrayBuffer;
  };
}

export const PROCESSOR_NAME = "ether-engine";
const QUANTUM = 128;

class EtherEngineProcessor extends AudioWorkletProcessor {
  private readonly memory: WebAssembly.Memory;
  private readonly engine: WasmEngine;
  private views: Float32Array[] = [];
  private header: Float64Array | null = null;
  private readonly tapClock: TapClockWriter | null;
  private dead = false;

  constructor(options: ProcessorOptions) {
    super();
    const { module, control, reports, tapClock } = options.processorOptions;
    this.memory = initSync({ module }).memory;
    this.engine = new WasmEngine(sampleRate, control, reports);
    this.tapClock = tapClock ? new TapClockWriter(tapClock, sampleRate) : null;
  }

  /** The tap clock header in wasm memory (`f64`s; recreated when memory grows). */
  private tapHeader(): Float64Array {
    let h = this.header;
    if (!h || h.buffer !== this.memory.buffer) {
      h = new Float64Array(this.memory.buffer, this.engine.output_ptr(TAP_CLOCK_SLOT), HEADER.LEN);
      this.header = h;
    }
    return h;
  }

  /** Views of the output (0, 1) and tap (2, 3) channels in wasm memory (recreated when memory grows). */
  private channel(i: number): Float32Array {
    let v = this.views[i];
    if (!v || v.buffer !== this.memory.buffer) {
      v = new Float32Array(this.memory.buffer, this.engine.output_ptr(i), QUANTUM);
      this.views[i] = v;
    }
    return v;
  }

  process(_inputs: Float32Array[][], outputs: Float32Array[][]): boolean {
    if (this.dead) return false;
    const out = outputs[0];
    const frames = out?.[0]?.length ?? QUANTUM;
    try {
      this.engine.process(frames);
    } catch (e) {
      // A Rust panic poisons the instance: report once and stop rendering.
      this.dead = true;
      this.port.postMessage({ type: "fatal", message: e instanceof Error ? e.message : String(e) });
      return false;
    }
    if (out) {
      const left = this.channel(0);
      const right = this.channel(1);
      out[0]?.set(frames === QUANTUM ? left : left.subarray(0, frames));
      if (out[1]) out[1].set(frames === QUANTUM ? right : right.subarray(0, frames));
    }
    const tap = outputs[1];
    if (tap && this.tapClock) {
      const left = this.channel(TAP_OUTPUT_BASE);
      const right = this.channel(TAP_OUTPUT_BASE + 1);
      tap[0]?.set(frames === QUANTUM ? left : left.subarray(0, frames));
      tap[1]?.set(frames === QUANTUM ? right : right.subarray(0, frames));
      this.tapClock.write(currentFrame, this.tapHeader());
    }
    return true;
  }
}

registerProcessor(PROCESSOR_NAME, EtherEngineProcessor);
