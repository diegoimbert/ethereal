// The AudioWorklet processor that runs `ether-core` (wasm instance #2). Each render quantum the
// Rust side drains the control ring, renders, and writes reports; this class only copies the
// rendered planar output from wasm memory into the Web Audio output buffers.
//
// `./engine.worklet.ts` registers it. An extension's worklet module (`@/ext` "web.engine"
// `workletUrl`) may register a subclass instead, for outputs of its own (`afterRender`).
import "./textCodec";
import { initSync, WasmEngine } from "@ether-wasm/ether_wasm.js";
import type { WebclapModuleMsg, WebclapPortMsg } from "./protocol";
import { WebclapHost } from "./webclap/host";

// AudioWorkletGlobalScope (not in lib.dom).
declare const sampleRate: number;
declare class AudioWorkletProcessor {
  readonly port: MessagePort;
  constructor();
}

export interface EngineProcessorOptions {
  processorOptions: {
    module: WebAssembly.Module;
    control: SharedArrayBuffer;
    reports: SharedArrayBuffer;
    /** The installed WebCLAP modules (more come as `webclap:module` port messages). */
    webclap?: Omit<WebclapModuleMsg, "type">[];
    /** An extension's own options (its subclass reads them). */
    [extra: string]: unknown;
  };
}

export const PROCESSOR_NAME = "ether-engine";
export const QUANTUM = 128;

export class EtherEngineProcessor extends AudioWorkletProcessor {
  protected readonly memory: WebAssembly.Memory;
  protected readonly engine: WasmEngine;
  /** The WebCLAP plugin instances (each its own WebAssembly instance), driven by the engine. */
  protected readonly webclap: WebclapHost;
  private views: Float32Array[] = [];
  private dead = false;

  constructor(options: EngineProcessorOptions) {
    super();
    const { module, control, reports } = options.processorOptions;
    this.memory = initSync({ module }).memory;
    this.engine = new WasmEngine(sampleRate, control, reports);
    this.webclap = new WebclapHost({
      engineMemory: () => this.memory,
      ui: (device, bytes) => {
        const data = bytes.buffer as ArrayBuffer;
        this.port.postMessage({ type: "webclap:ui", device, data } satisfies WebclapPortMsg, [data]);
      },
      log: (device, severity, text) => {
        if (severity >= 2) console.warn(`webclap ${device}: ${text}`);
      },
    });
    for (const m of options.processorOptions.webclap ?? []) this.webclap.addModule(m.id, m.module, m.info);
    this.engine.set_webclap_host(this.webclap);
    // A listener (not `onmessage`): a subclass may have its own.
    this.port.addEventListener("message", (e: MessageEvent<WebclapPortMsg>) => this.onWebclap(e.data));
    this.port.start();
  }

  /** The page's WebCLAP messages: modules and the plugin UIs (between blocks: the main phase). */
  private onWebclap(msg: WebclapPortMsg): void {
    const post = (m: WebclapPortMsg, transfer: Transferable[] = []) => this.port.postMessage(m, transfer);
    switch (msg.type) {
      case "webclap:module":
        this.webclap.addModule(msg.id, msg.module, msg.info);
        break;
      case "webclap:ui":
        this.webclap.byDevice(msg.device)?.receive(new Uint8Array(msg.data));
        break;
      case "webclap:open": {
        const inst = this.webclap.byDevice(msg.device);
        try {
          if (!inst) throw new Error("the plugin is not running");
          post({ type: "webclap:opened", req: msg.req, uri: inst.openUi() });
        } catch (e) {
          post({ type: "webclap:opened", req: msg.req, error: e instanceof Error ? e.message : String(e) });
        }
        break;
      }
      case "webclap:close":
        this.webclap.byDevice(msg.device)?.closeUi();
        break;
      case "webclap:resource": {
        const r = this.webclap.byDevice(msg.device)?.resource(msg.path) ?? null;
        const data = r ? (r.bytes.buffer as ArrayBuffer) : undefined;
        post({ type: "webclap:resourced", req: msg.req, mime: r?.mime, data }, data ? [data] : []);
        break;
      }
      default:
        break;
    }
  }

  /** A view of output channel `i` in wasm memory (recreated when memory grows). */
  protected channel(i: number): Float32Array {
    let v = this.views[i];
    if (!v || v.buffer !== this.memory.buffer) {
      v = new Float32Array(this.memory.buffer, this.engine.output_ptr(i), QUANTUM);
      this.views[i] = v;
    }
    return v;
  }

  /** After each rendered quantum (`frames` long): a subclass fills its own outputs (1 and up). */
  protected afterRender(outputs: Float32Array[][], frames: number): void {
    void outputs;
    void frames;
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
    this.afterRender(outputs, frames);
    return true;
  }
}
