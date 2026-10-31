// The WebCLAP host of one JS realm: the Worklet's (live instances, next to the engine) or
// the controller Worker's (offline renders). The engine's Rust side calls it
// (crates/ether-wasm/src/webclap/js.rs `JsWebclapHost`): `create`, `process` each block,
// state and value requests, notifications. The page sends it the compiled modules
// (`addModule`) and, in the Worklet, the plugin UIs' messages (`./uiChannel.ts`).
//
// Instances whose module hasn't arrived yet are kept pending and instantiated when it does
// (`process` answers `STATUS.pending` meanwhile).

import { ClapInstance, type EngineView } from "./instance";
import { NOTE, STATUS } from "./layout";
import type { ModuleInfo } from "./wasm";

interface Entry {
  module: string;
  pluginId: string;
  device: string;
  sampleRate: number;
  maxFrames: number;
  state: Uint8Array | null;
  instance: ClapInstance | null;
  error: string | null;
}

export interface WebclapHostOptions {
  /** The engine's wasm memory (the `Exchange` blocks live there). */
  engineMemory: () => WebAssembly.Memory;
  /** A plugin's message to its UI (the Worklet posts it to the page). */
  ui?: (device: string, bytes: Uint8Array) => void;
  log?: (device: string, severity: number, text: string) => void;
}

export class WebclapHost {
  private readonly modules = new Map<string, { module: WebAssembly.Module; info: ModuleInfo }>();
  private readonly entries = new Map<number, Entry>();
  private next = 1;
  private notes: unknown[] = [];
  private view: EngineView | null = null;
  private viewBuffer: ArrayBufferLike | null = null;

  constructor(private readonly opts: WebclapHostOptions) {}

  /** A compiled module the page installed; instantiates what waited for it. */
  addModule(id: string, module: WebAssembly.Module, info: ModuleInfo): void {
    this.modules.set(id, { module, info });
    for (const [handle, e] of this.entries) {
      if (e.module === id && !e.instance && !e.error) this.instantiate(handle, e);
    }
  }

  hasModule(id: string): boolean {
    return this.modules.has(id);
  }

  private instantiate(handle: number, e: Entry): void {
    const m = this.modules.get(e.module);
    if (!m) return;
    try {
      e.instance = new ClapInstance(m.module, m.info, e.pluginId, {
        sampleRate: e.sampleRate,
        maxFrames: e.maxFrames,
        state: e.state,
        notify: (kind, a, b, text) => this.notes.push([handle, kind, a, b, text ?? ""]),
        ui: this.opts.ui ? (bytes) => (this.opts.ui!(e.device, bytes), true) : undefined,
        log: (severity, text) => this.opts.log?.(e.device, severity, text),
      });
      e.state = null;
    } catch (err) {
      e.error = err instanceof Error ? err.message : String(err);
      this.notes.push([handle, NOTE.crashed, 0, 0, `${e.pluginId}: ${e.error}`]);
    }
  }

  private engine(): EngineView {
    const buf = this.opts.engineMemory().buffer;
    if (!this.view || buf !== this.viewBuffer) {
      this.viewBuffer = buf;
      this.view = { dv: new DataView(buf), f32: new Float32Array(buf) };
    }
    return this.view;
  }

  /** The live instance of `device` (UI calls). */
  byDevice(device: string): ClapInstance | null {
    for (const e of this.entries.values()) if (e.device === device && e.instance) return e.instance;
    return null;
  }

  // ─── Called by the engine (Rust) ────────────────────────────────────────────────────

  create(module: string, pluginId: string, device: string, sampleRate: number, maxFrames: number, state?: Uint8Array): number {
    const handle = this.next++;
    const e: Entry = { module, pluginId, device, sampleRate, maxFrames, state: state ? state.slice() : null, instance: null, error: null };
    this.entries.set(handle, e);
    this.instantiate(handle, e);
    return handle;
  }

  /** **RT.** */
  process(handle: number, exchange: number): number {
    const e = this.entries.get(handle);
    if (!e) return STATUS.faulted;
    if (!e.instance) return e.error ? STATUS.faulted : STATUS.pending;
    return e.instance.process(this.engine(), exchange);
  }

  ready(handle: number): number {
    const e = this.entries.get(handle);
    if (!e || e.error || e.instance?.failed) return -1;
    return e.instance ? 1 : 0;
  }

  error(handle: number): string {
    const e = this.entries.get(handle);
    return e?.error ?? e?.instance?.failed ?? "no such instance";
  }

  latency(handle: number): number {
    return this.entries.get(handle)?.instance?.latency ?? 0;
  }

  takeWake(handle: number): boolean {
    const i = this.entries.get(handle)?.instance;
    if (!i?.wake) return false;
    i.wake = false;
    return true;
  }

  reset(handle: number): void {
    this.entries.get(handle)?.instance?.reset();
  }

  destroy(handle: number): void {
    const e = this.entries.get(handle);
    this.entries.delete(handle);
    e?.instance?.closeUi();
    e?.instance?.destroy();
  }

  /** The instance's state (`null`: it has none); throws past `maxBytes` ([`MAX_STATE_BYTES`]). */
  saveState(handle: number, maxBytes?: number): Uint8Array | null {
    const e = this.entries.get(handle);
    if (!e) throw new Error("no such instance");
    if (!e.instance) {
      if (e.error) throw new Error(e.error);
      throw new Error("not instantiated yet");
    }
    return e.instance.saveState(maxBytes);
  }

  paramValues(handle: number): Float64Array | null {
    const i = this.entries.get(handle)?.instance;
    if (!i || i.failed) return null;
    try {
      return i.paramValues();
    } catch {
      return null;
    }
  }

  takeNotifications(): unknown[] | undefined {
    if (this.notes.length === 0) return undefined;
    const out = this.notes;
    this.notes = [];
    return out;
  }
}
