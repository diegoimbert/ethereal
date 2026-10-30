// One WebCLAP plugin instance: its own WebAssembly instance (its own memory), the CLAP host
// it sees, and every call into it. Used by the Worklet's host (`./host.ts`) to process,
// by the controller Worker's host for offline renders, and by the install scan
// (`./scan.ts`).
//
// Threads: the AudioWorklet is single-threaded, so "main thread" and "audio thread" are
// phases of the same thread: `process` (and `reset`, and `flush` while processing) run in
// the audio phase, everything else in the main phase, between blocks. `clap.thread-check`
// answers by phase.
//
// Real-time rule: `process` allocates nothing (typed-array views are made once and only
// re-made when the plugin's memory grows); samples and events are copied with plain loops.

import {
  AUDIO_BUFFER,
  AUDIO_PORT_INFO,
  AUDIO_PORT_IS_MAIN,
  AUDIO_PORTS_EXT,
  CLAP_VERSION,
  CORE_EVENT_SPACE,
  DESC,
  ENTRY,
  EV,
  EVENT,
  EVENT_IS_LIVE,
  EVENT_SLOT,
  EXT,
  FACTORY,
  GET_EXT,
  GUI_EXT,
  HOST,
  IN_EVENTS,
  LOG,
  MIDI,
  NOTE,
  NOTE_DIALECT_CLAP,
  NOTE_EXPR,
  NOTE_PORT_INFO,
  NOTE_PORTS_EXT,
  OUT_EVENTS,
  PARAM_FLAGS,
  PARAM_GESTURE,
  PARAM_INFO,
  PARAM_VALUE,
  PARAMS_EXT,
  PLUGIN,
  PROCESS,
  STATE_EXT,
  STREAM,
  TIME_FACTOR,
  TRANSPORT,
  TRANSPORT_FLAGS,
  WEBVIEW_EXT,
} from "./abi";
import { EXCHANGE, KIND, NOTE as NOTE_KIND, REC, STATUS, XFLAGS } from "./layout";
import { type ModuleInfo, type Sig, trampolines } from "./wasm";
import { wasiImports } from "./wasi";

/** Where the plugin believes it lives (`clap_entry.init`): its bundle has no files here. */
export const PLUGIN_PATH = "/plugin";

/**
 * The biggest message between a plugin and its web UI, either way (bigger ones are dropped):
 * the UI's land in the plugin's memory on the audio thread, the plugin's on the page.
 */
export const MAX_UI_MESSAGE_BYTES = 4 * 1024 * 1024;

/** Events per block into the plugin (all-notes-off expands to two). */
const IN_EVENT_SLOTS = 2048;
/** Default ceiling of an imported memory without a declared maximum (64 KiB pages). */
const DEFAULT_MAX_PAGES = 32768;

export interface PluginDesc {
  id: string;
  name: string;
  vendor: string;
  version: string;
  description: string;
  url: string;
  features: string[];
}

export interface PortInfo {
  channels: number;
  main: boolean;
}

export interface ParamInfo {
  id: number;
  flags: number;
  name: string;
  module: string;
  min: number;
  max: number;
  default: number;
  labels?: string[];
}

/** The engine's memory, viewed (refreshed by the host when it grows). */
export interface EngineView {
  dv: DataView;
  f32: Float32Array;
}

export interface InstanceOptions {
  sampleRate: number;
  maxFrames: number;
  state?: Uint8Array | null;
  /** Activate (the default); the scan only reads metadata. */
  activate?: boolean;
  /** A notification (`./layout.ts` NOTE kinds). */
  notify?: (kind: number, a: number, b: number, text?: string) => void;
  /** A message from the plugin to its web UI. */
  ui?: (bytes: Uint8Array) => boolean;
  log?: (severity: number, text: string) => void;
}

type Fn = (...args: number[]) => number;

interface Port {
  channels: number;
  /** Plugin-memory byte addresses of each channel's buffer. */
  bufs: number[];
}

const HOST_SIGS: Sig[] = [
  { params: ["i32", "i32"], result: "i32" }, // 0 host.get_extension
  { params: ["i32"] }, // 1 request_restart
  { params: ["i32"] }, // 2 request_process
  { params: ["i32"] }, // 3 request_callback
  { params: ["i32"], result: "i32" }, // 4 input_events.size
  { params: ["i32", "i32"], result: "i32" }, // 5 input_events.get
  { params: ["i32", "i32"], result: "i32" }, // 6 output_events.try_push
  { params: ["i32", "i32", "i64"], result: "i64" }, // 7 istream.read
  { params: ["i32", "i32", "i64"], result: "i64" }, // 8 ostream.write
  { params: ["i32", "i32"] }, // 9 params.rescan
  { params: ["i32", "i32", "i32"] }, // 10 params.clear
  { params: ["i32"] }, // 11 params.request_flush
  { params: ["i32"] }, // 12 state.mark_dirty
  { params: ["i32"] }, // 13 latency.changed
  { params: ["i32"] }, // 14 tail.changed
  { params: ["i32", "i32", "i32"] }, // 15 log.log
  { params: ["i32"], result: "i32" }, // 16 thread_check.is_main_thread
  { params: ["i32"], result: "i32" }, // 17 thread_check.is_audio_thread
  { params: ["i32", "i32", "i32"], result: "i32" }, // 18 webview.send
  { params: ["i32"] }, // 19 gui.resize_hints_changed
  { params: ["i32", "i32", "i32"], result: "i32" }, // 20 gui.request_resize
  { params: ["i32"], result: "i32" }, // 21 gui.request_show
  { params: ["i32"], result: "i32" }, // 22 gui.request_hide
  { params: ["i32", "i32"] }, // 23 gui.closed
  { params: ["i32", "i32"], result: "i32" }, // 24 audio_ports.is_rescan_flag_supported
  { params: ["i32", "i32"] }, // 25 audio_ports.rescan
  { params: ["i32"], result: "i32" }, // 26 note_ports.supported_dialects
  { params: ["i32", "i32"] }, // 27 note_ports.rescan
];

function message(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** A WebAssembly instance of a WebCLAP module, with its entry initialized. */
class Module {
  readonly instance: WebAssembly.Instance;
  readonly memory: WebAssembly.Memory;
  readonly table: WebAssembly.Table;
  private readonly malloc: (size: number) => number;
  readonly entry: number;
  private buf: ArrayBuffer | SharedArrayBuffer | null = null;
  u8!: Uint8Array;
  dv!: DataView;
  f32!: Float32Array;

  constructor(module: WebAssembly.Module, info: ModuleInfo, log: (fd: number, line: string) => void) {
    const imports: Record<string, Record<string, WebAssembly.ExportValue | ((...a: never[]) => unknown)>> = {};
    let memory: WebAssembly.Memory | null = null;
    if (info.memory) {
      if (info.memory.memory64) throw new Error("64-bit webclap modules are not supported");
      memory = new WebAssembly.Memory({
        initial: info.memory.initial,
        maximum: info.memory.maximum ?? DEFAULT_MAX_PAGES,
        shared: info.memory.shared,
      } as WebAssembly.MemoryDescriptor);
    }
    const wasi: string[] = [];
    for (const imp of WebAssembly.Module.imports(module)) {
      const ns = (imports[imp.module] ??= {});
      if (imp.kind === "memory" && memory) ns[imp.name] = memory;
      else if (imp.module === "wasi_snapshot_preview1" && imp.kind === "function") wasi.push(imp.name);
      else if (imp.kind === "function") ns[imp.name] = () => 0;
      else throw new Error(`unsupported import ${imp.module}.${imp.name} (${imp.kind})`);
    }
    if (wasi.length) {
      Object.assign((imports.wasi_snapshot_preview1 ??= {}), wasiImports({ memory: () => this.memory, log }, wasi));
    }
    this.instance = new WebAssembly.Instance(module, imports as WebAssembly.Imports);
    const ex = this.instance.exports;
    const exported = Object.values(ex).find((v): v is WebAssembly.Memory => v instanceof WebAssembly.Memory);
    const mem = memory ?? exported;
    if (!mem) throw new Error("the module has no memory");
    this.memory = mem;
    const tables = Object.values(ex).filter((v): v is WebAssembly.Table => v instanceof WebAssembly.Table);
    if (tables.length !== 1) throw new Error("a webclap module exports exactly one function table");
    this.table = tables[0]!;
    if (typeof ex.malloc === "function") {
      const malloc = ex.malloc as (n: number) => number;
      this.malloc = (n) => malloc(n);
    } else if (typeof ex.cabi_realloc === "function") {
      const realloc = ex.cabi_realloc as (a: number, b: number, c: number, d: number) => number;
      this.malloc = (n) => realloc(0, 0, 16, n);
    } else {
      throw new Error("the module exports no malloc");
    }
    if (typeof ex._initialize === "function") (ex._initialize as () => void)();
    const entry = ex.clap_entry;
    if (!(entry instanceof WebAssembly.Global)) throw new Error("the module exports no clap_entry");
    const value: unknown = entry.value;
    if (typeof value !== "number") throw new Error("64-bit webclap modules are not supported");
    this.entry = value;
    this.views();
  }

  /** Refresh the views after the memory grew. */
  views(): void {
    const buf = this.memory.buffer;
    if (buf === this.buf && this.u8.byteLength === buf.byteLength) return;
    this.buf = buf;
    this.u8 = new Uint8Array(buf);
    this.dv = new DataView(buf);
    this.f32 = new Float32Array(buf);
  }

  /** `size` bytes of the plugin's memory, 16-aligned, zeroed. */
  alloc(size: number): number {
    const p = this.malloc(size + 16);
    if (!p) throw new Error("the plugin's malloc failed");
    this.views();
    const aligned = (p + 15) & ~15;
    this.u8.fill(0, aligned, aligned + size);
    return aligned;
  }

  fn(index: number): Fn {
    const f = this.table.get(index) as Fn | null;
    if (!f) throw new Error(`null function pointer ${index}`);
    return f;
  }

  cstring(ptr: number, max = 4096): string {
    if (!ptr) return "";
    this.views();
    let end = ptr;
    while (end < ptr + max && end < this.u8.length && this.u8[end] !== 0) end++;
    return new TextDecoder().decode(this.u8.slice(ptr, end));
  }

  writeString(s: string): number {
    const bytes = new TextEncoder().encode(s);
    const p = this.alloc(bytes.length + 1);
    this.u8.set(bytes, p);
    this.u8[p + bytes.length] = 0;
    return p;
  }

  /** The factory and its plugins' descriptors (entry initialized once per instance). */
  factory(): number {
    const dv = this.dv;
    const init = this.fn(dv.getUint32(this.entry + ENTRY.init, true));
    if (!init(this.writeString(PLUGIN_PATH))) throw new Error("clap_entry.init failed");
    this.views();
    const getFactory = this.fn(this.dv.getUint32(this.entry + ENTRY.getFactory, true));
    const factory = getFactory(this.writeString("clap.plugin-factory"));
    if (!factory) throw new Error("the module has no plugin factory");
    return factory;
  }

  descriptor(ptr: number): PluginDesc {
    this.views();
    const dv = this.dv;
    const s = (o: number) => this.cstring(dv.getUint32(ptr + o, true));
    const features: string[] = [];
    const fp = dv.getUint32(ptr + DESC.features, true);
    for (let i = 0; fp && i < 64; i++) {
      const f = dv.getUint32(fp + 4 * i, true);
      if (!f) break;
      features.push(this.cstring(f));
    }
    return {
      id: s(DESC.id),
      name: s(DESC.name),
      vendor: s(DESC.vendor),
      version: s(DESC.version),
      description: s(DESC.description),
      url: s(DESC.url),
      features,
    };
  }

  descriptors(factory: number): { ptr: number; desc: PluginDesc }[] {
    const count = this.fn(this.dv.getUint32(factory + FACTORY.count, true))(factory);
    const getDesc = this.fn(this.dv.getUint32(factory + FACTORY.descriptor, true));
    const out: { ptr: number; desc: PluginDesc }[] = [];
    for (let i = 0; i < count; i++) {
      const ptr = getDesc(factory, i);
      if (ptr) out.push({ ptr, desc: this.descriptor(ptr) });
    }
    return out;
  }
}

/** The plugins a module provides (instantiates it once). */
export function listPlugins(module: WebAssembly.Module, info: ModuleInfo): PluginDesc[] {
  const m = new Module(module, info, () => undefined);
  const list = m.descriptors(m.factory()).map((d) => d.desc);
  try {
    m.fn(m.dv.getUint32(m.entry + ENTRY.deinit, true))();
  } catch {
    // deinit failing changes nothing for a listing.
  }
  return list;
}

export class ClapInstance {
  private readonly m: Module;
  readonly desc: PluginDesc;
  private readonly opts: InstanceOptions;
  private readonly plugin: number;
  private readonly fns: {
    process: Fn;
    reset: Fn;
    activate: Fn;
    deactivate: Fn;
    start: Fn;
    stop: Fn;
    onMainThread: Fn;
  };
  private readonly ext: {
    params: number;
    state: number;
    audioPorts: number;
    notePorts: number;
    latency: number;
    tail: number;
    webview: number;
    gui: number;
  };
  private readonly hostExt: Record<string, number> = {};
  private phase: "main" | "audio" = "main";
  private active = false;
  private processing = false;
  failed: string | null = null;
  /** `request_process` / `request_flush` / a restart or callback to do: wake the node. */
  wake = false;
  private restart = false;
  private callback = false;
  private flush = false;
  latency = 0;
  /** Port layout: plugin ports and which the engine feeds. */
  inputs: Port[] = [];
  outputs: Port[] = [];
  private mainIn = -1;
  private mainOut = -1;
  private aux = -1;
  // Process structures (plugin memory).
  private proc = 0;
  private transport = 0;
  private inList = 0;
  private emptyList = 0;
  private outList = 0;
  private evBase = 0;
  private nIn = 0;
  private stream = { in: 0, out: 0 };
  private streamData: Uint8Array = new Uint8Array(0);
  private streamPos = 0;
  private streamOut: Uint8Array[] = [];
  private scratch = 0;
  private msg = { ptr: 0, cap: 0 };
  // The exchange of the block being processed (engine memory), for output events.
  private engine: EngineView | null = null;
  private x = 0;
  private nOut = 0;
  private uiOpen = false;

  constructor(module: WebAssembly.Module, info: ModuleInfo, pluginId: string, opts: InstanceOptions) {
    this.opts = opts;
    const log = (fd: number, line: string) => opts.log?.(fd === 2 ? LOG.warning : LOG.info, line);
    const m = (this.m = new Module(module, info, log));
    const factory = m.factory();
    const found = m.descriptors(factory).find((d) => d.desc.id === pluginId);
    if (!found) throw new Error(`the module has no plugin ${pluginId}`);
    this.desc = found.desc;

    // Host callbacks into the plugin's function table.
    const fns = trampolines(HOST_SIGS, this.hostFunctions() as ((...a: never[]) => unknown)[]);
    const base = m.table.grow(fns.length);
    fns.forEach((f, i) => m.table.set(base + i, f));
    const fp = (i: number) => base + i;
    const struct = (...entries: number[]) => {
      const p = m.alloc(4 * entries.length);
      entries.forEach((e, i) => m.dv.setUint32(p + 4 * i, fp(e), true));
      return p;
    };
    this.hostExt[EXT.params] = struct(9, 10, 11);
    this.hostExt[EXT.state] = struct(12);
    this.hostExt[EXT.latency] = struct(13);
    this.hostExt[EXT.tail] = struct(14);
    this.hostExt[EXT.log] = struct(15);
    this.hostExt[EXT.threadCheck] = struct(16, 17);
    this.hostExt[EXT.webview] = struct(18);
    this.hostExt[EXT.gui] = struct(19, 20, 21, 22, 23);
    this.hostExt[EXT.audioPorts] = struct(24, 25);
    this.hostExt[EXT.notePorts] = struct(26, 27);

    const host = m.alloc(HOST.size);
    const dv = m.dv;
    CLAP_VERSION.forEach((v, i) => dv.setUint32(host + 4 * i, v, true));
    m.dv.setUint32(host + HOST.name, m.writeString("Ethereal"), true);
    m.dv.setUint32(host + HOST.vendor, m.writeString("Ethereal"), true);
    m.dv.setUint32(host + HOST.url, m.writeString("https://ethereal.ws"), true);
    m.dv.setUint32(host + HOST.version, m.writeString("1.0"), true);
    m.dv.setUint32(host + HOST.getExtension, fp(0), true);
    m.dv.setUint32(host + HOST.requestRestart, fp(1), true);
    m.dv.setUint32(host + HOST.requestProcess, fp(2), true);
    m.dv.setUint32(host + HOST.requestCallback, fp(3), true);

    // Event lists and streams (`ctx` unused: one instance per module instance).
    this.inList = m.alloc(IN_EVENTS.size);
    this.emptyList = m.alloc(IN_EVENTS.size);
    for (const l of [this.inList, this.emptyList]) {
      m.dv.setUint32(l + IN_EVENTS.sizeFn, fp(4), true);
      m.dv.setUint32(l + IN_EVENTS.get, fp(5), true);
    }
    this.outList = m.alloc(OUT_EVENTS.size);
    m.dv.setUint32(this.outList + OUT_EVENTS.tryPush, fp(6), true);
    this.stream.in = m.alloc(STREAM.size);
    m.dv.setUint32(this.stream.in + STREAM.fn, fp(7), true);
    this.stream.out = m.alloc(STREAM.size);
    m.dv.setUint32(this.stream.out + STREAM.fn, fp(8), true);
    this.scratch = m.alloc(PARAM_INFO.size + 4096);

    const create = m.fn(m.dv.getUint32(factory + FACTORY.create, true));
    const plugin = create(factory, host, m.writeString(pluginId));
    if (!plugin) throw new Error(`creating ${pluginId} failed`);
    this.plugin = plugin;
    const pf = (o: number) => m.fn(m.dv.getUint32(plugin + o, true));
    if (!pf(PLUGIN.init)(plugin)) throw new Error(`${pluginId}: init failed`);
    this.fns = {
      process: pf(PLUGIN.process),
      reset: pf(PLUGIN.reset),
      activate: pf(PLUGIN.activate),
      deactivate: pf(PLUGIN.deactivate),
      start: pf(PLUGIN.startProcessing),
      stop: pf(PLUGIN.stopProcessing),
      onMainThread: pf(PLUGIN.onMainThread),
    };
    const getExt = pf(PLUGIN.getExtension);
    const ext = (id: string) => getExt(plugin, m.writeString(id));
    this.ext = {
      params: ext(EXT.params),
      state: ext(EXT.state),
      audioPorts: ext(EXT.audioPorts),
      notePorts: ext(EXT.notePorts),
      latency: ext(EXT.latency),
      tail: ext(EXT.tail),
      webview: ext(EXT.webview),
      gui: ext(EXT.gui),
    };
    if (opts.state) this.loadState(opts.state);
    if (opts.activate ?? true) this.prepare();
  }

  // ─── Host callbacks ─────────────────────────────────────────────────────────────────

  private hostFunctions(): ((...a: never[]) => unknown)[] {
    const notify = (kind: number, a = 0, b = 0, text?: string) => this.opts.notify?.(kind, a, b, text);
    return [
      (_host: number, id: number) => this.hostExt[this.m.cstring(id, 256)] ?? 0,
      () => {
        this.restart = true;
        this.wake = true;
      },
      () => {
        this.wake = true;
      },
      () => {
        this.callback = true;
        this.wake = true;
      },
      (list: number) => (list === this.inList ? this.nIn : 0),
      (list: number, i: number) => (list === this.inList && i < this.nIn ? this.evBase + i * EVENT_SLOT : 0),
      (_list: number, ev: number) => (this.outputEvent(ev) ? 1 : 0),
      (_stream: number, buf: number, size: bigint) => {
        const n = Math.min(Number(size), this.streamData.length - this.streamPos);
        if (n <= 0) return 0n;
        this.m.views();
        this.m.u8.set(this.streamData.subarray(this.streamPos, this.streamPos + n), buf);
        this.streamPos += n;
        return BigInt(n);
      },
      (_stream: number, buf: number, size: bigint) => {
        const n = Number(size);
        this.m.views();
        this.streamOut.push(this.m.u8.slice(buf, buf + n));
        return size;
      },
      () => notify(NOTE_KIND.params),
      () => undefined,
      () => {
        this.flush = true;
        this.wake = true;
      },
      () => notify(NOTE_KIND.dirty),
      () => {
        // A latency change needs a restart (CLAP: the host deactivates and re-activates).
        this.restart = true;
        this.wake = true;
      },
      () => undefined,
      (_host: number, severity: number, msg: number) => this.opts.log?.(severity, this.m.cstring(msg)),
      () => (this.phase === "main" ? 1 : 0),
      () => (this.phase === "audio" ? 1 : 0),
      (_host: number, buf: number, size: number) => {
        if (!this.uiOpen || !this.opts.ui || size > MAX_UI_MESSAGE_BYTES) return 0;
        this.m.views();
        return this.opts.ui(this.m.u8.slice(buf, buf + size)) ? 1 : 0;
      },
      () => undefined,
      () => 0,
      () => 0,
      () => 0,
      () => undefined,
      () => 0,
      () => undefined,
      () => 3, // CLAP | MIDI
      () => undefined,
    ];
  }

  /** `try_push`: the plugin's output event at `ev`. */
  private outputEvent(ev: number): boolean {
    const dv = this.m.dv;
    if (dv.getUint16(ev + EVENT.spaceId, true) !== CORE_EVENT_SPACE) return true;
    const type = dv.getUint16(ev + EVENT.type, true);
    const time = dv.getUint32(ev + EVENT.time, true);
    switch (type) {
      case EV.paramValue: {
        const id = dv.getUint32(ev + PARAM_VALUE.paramId, true);
        const value = dv.getFloat64(ev + PARAM_VALUE.value, true);
        this.opts.notify?.(NOTE_KIND.param, id, value);
        return this.pushOut(time, KIND.param, 0, 0, 0, id, value);
      }
      case EV.gestureBegin:
        this.opts.notify?.(NOTE_KIND.begin, dv.getUint32(ev + PARAM_GESTURE.paramId, true), 0);
        return true;
      case EV.gestureEnd:
        this.opts.notify?.(NOTE_KIND.end, dv.getUint32(ev + PARAM_GESTURE.paramId, true), 0);
        return true;
      case EV.noteOn:
      case EV.noteOff:
      case EV.noteChoke: {
        const kind = type === EV.noteOn ? KIND.noteOn : type === EV.noteOff ? KIND.noteOff : KIND.noteChoke;
        const channel = dv.getInt16(ev + NOTE.channel, true);
        const key = dv.getInt16(ev + NOTE.key, true);
        if (channel < 0 || key < 0) return true;
        return this.pushOut(time, kind, channel, key, 0, dv.getInt32(ev + NOTE.noteId, true) >>> 0, dv.getFloat64(ev + NOTE.velocity, true));
      }
      case EV.midi: {
        const d = ev + MIDI.data;
        const id = dv.getUint8(d) | (dv.getUint8(d + 1) << 8) | (dv.getUint8(d + 2) << 16);
        return this.pushOut(time, KIND.midi, 0, 0, 0, id, 0);
      }
      default:
        return true;
    }
  }

  /** An output event into the exchange (engine memory), while a block is processed. */
  private pushOut(time: number, kind: number, channel: number, key: number, extra: number, id: number, value: number): boolean {
    const e = this.engine;
    if (!e || !this.x) return true;
    const cap = e.dv.getUint32(this.x + EXCHANGE.eventCapacity, true);
    if (this.nOut >= cap) return false;
    const r = e.dv.getUint32(this.x + EXCHANGE.outEvents, true) + this.nOut * REC.size;
    e.dv.setUint32(r + REC.offset, time, true);
    e.dv.setUint8(r + REC.kind, kind);
    e.dv.setUint8(r + REC.channel, channel & 0xff);
    e.dv.setUint8(r + REC.key, key & 0xff);
    e.dv.setUint8(r + REC.extra, extra);
    e.dv.setUint32(r + REC.id, id >>> 0, true);
    e.dv.setFloat64(r + REC.value, value, true);
    this.nOut++;
    return true;
  }

  // ─── Lifecycle ──────────────────────────────────────────────────────────────────────

  private fail(e: unknown): void {
    if (this.failed) return;
    this.failed = message(e);
    this.opts.notify?.(NOTE_KIND.crashed, 0, 0, this.failed);
  }

  /** Port layout, buffers and the process structures; then activate. */
  private prepare(): void {
    const m = this.m;
    const max = this.opts.maxFrames;
    const ins = this.audioPorts(true) ?? [{ channels: 2, main: true }];
    const outs = this.audioPorts(false) ?? [{ channels: 2, main: true }];
    const mainOf = (ports: PortInfo[]) => {
      const i = ports.findIndex((p) => p.main);
      return i >= 0 ? i : ports.length ? 0 : -1;
    };
    this.mainIn = mainOf(ins);
    this.mainOut = mainOf(outs);
    this.aux = ins.findIndex((p, i) => i !== this.mainIn && p.channels > 0);
    const buffers = (ports: PortInfo[]): [number, Port[]] => {
      const arr = m.alloc(Math.max(1, ports.length) * AUDIO_BUFFER.size);
      const out = ports.map((p, i) => {
        const ptrs = m.alloc(4 * Math.max(1, p.channels));
        const bufs: number[] = [];
        for (let c = 0; c < p.channels; c++) {
          const b = m.alloc(4 * max);
          m.dv.setUint32(ptrs + 4 * c, b, true);
          bufs.push(b);
        }
        const s = arr + i * AUDIO_BUFFER.size;
        m.dv.setUint32(s + AUDIO_BUFFER.data32, ptrs, true);
        m.dv.setUint32(s + AUDIO_BUFFER.channelCount, p.channels, true);
        return { channels: p.channels, bufs };
      });
      return [arr, out];
    };
    const [inArr, inPorts] = buffers(ins);
    const [outArr, outPorts] = buffers(outs);
    this.inputs = inPorts;
    this.outputs = outPorts;
    this.evBase = m.alloc(IN_EVENT_SLOTS * EVENT_SLOT);
    this.transport = m.alloc(TRANSPORT.size);
    const proc = (this.proc = m.alloc(PROCESS.size));
    const dv = m.dv;
    dv.setUint32(proc + PROCESS.transport, this.transport, true);
    dv.setUint32(proc + PROCESS.audioInputs, inArr, true);
    dv.setUint32(proc + PROCESS.audioOutputs, outArr, true);
    dv.setUint32(proc + PROCESS.inputsCount, ins.length, true);
    dv.setUint32(proc + PROCESS.outputsCount, outs.length, true);
    dv.setUint32(proc + PROCESS.inEvents, this.inList, true);
    dv.setUint32(proc + PROCESS.outEvents, this.outList, true);
    dv.setUint32(this.transport + EVENT.size, TRANSPORT.size, true);
    dv.setUint16(this.transport + EVENT.type, EV.transport, true);
    this.activate();
  }

  private activate(): void {
    if (!this.fns.activate(this.plugin, this.opts.sampleRate, 1, this.opts.maxFrames)) {
      throw new Error(`${this.desc.id}: activate failed`);
    }
    this.active = true;
    this.latency = this.ext.latency ? this.m.fn(this.m.dv.getUint32(this.ext.latency + GET_EXT.get, true))(this.plugin) >>> 0 : 0;
  }

  /** Between blocks: what the plugin asked for on the main thread. */
  private maintain(): void {
    if (this.restart && this.active) {
      this.restart = false;
      if (this.processing) {
        this.phase = "audio";
        try {
          this.fns.stop(this.plugin);
        } finally {
          this.phase = "main";
        }
        this.processing = false;
      }
      this.fns.deactivate(this.plugin);
      this.active = false;
      this.activate();
    }
    if (this.callback) {
      this.callback = false;
      this.fns.onMainThread(this.plugin);
    }
  }

  /**
   * **RT.** Process the block the engine describes at `x` (an `Exchange` in `engine`).
   * Returns a CLAP process status, or a `STATUS` code.
   */
  process(engine: EngineView, x: number): number {
    if (this.failed || !this.active) return STATUS.faulted;
    const m = this.m;
    const e = engine.dv;
    const ef = engine.f32;
    try {
      this.maintain();
      m.views();
      const dv = m.dv;
      const pf = m.f32;
      const frames = e.getUint32(x + EXCHANGE.frames, true);
      if (frames > this.opts.maxFrames) return STATUS.error;
      // Inputs: main port ← the engine's input (mono feeds every channel), aux port ← the
      // sidechain, others silent.
      const inCh = e.getUint32(x + EXCHANGE.inChannels, true);
      const scCh = e.getUint32(x + EXCHANGE.scChannels, true);
      for (let p = 0; p < this.inputs.length; p++) {
        const port = this.inputs[p]!;
        for (let c = 0; c < port.channels; c++) {
          let src = 0;
          if (p === this.mainIn && inCh > 0) src = e.getUint32(x + EXCHANGE.inputs + 4 * Math.min(c, inCh - 1), true);
          else if (p === this.aux && scCh > 0) src = c < 2 ? e.getUint32(x + EXCHANGE.sidechain + 4 * Math.min(c, scCh - 1), true) : 0;
          const d = port.bufs[c]! >> 2;
          if (src) {
            const s = src >> 2;
            for (let i = 0; i < frames; i++) pf[d + i] = ef[s + i]!;
          } else {
            for (let i = 0; i < frames; i++) pf[d + i] = 0;
          }
        }
      }
      this.writeEvents(engine, x);
      this.writeTransport(e, x);
      const proc = this.proc;
      dv.setUint32(proc + PROCESS.frames, frames, true);
      const steady = e.getFloat64(x + EXCHANGE.steadyTime, true);
      dv.setUint32(proc + PROCESS.steadyTime, steady % 4294967296, true);
      dv.setUint32(proc + PROCESS.steadyTime + 4, Math.floor(steady / 4294967296), true);

      this.engine = engine;
      this.x = x;
      this.nOut = 0;
      this.phase = "audio";
      if (!this.processing) {
        if (!this.fns.start(this.plugin)) throw new Error(`${this.desc.id}: start_processing failed`);
        this.processing = true;
      }
      if (this.flush && this.ext.params) {
        this.flush = false;
        m.fn(dv.getUint32(this.ext.params + PARAMS_EXT.flush, true))(this.plugin, this.emptyList, this.outList);
      }
      const status = this.fns.process(this.plugin, proc);
      this.phase = "main";
      e.setUint32(x + EXCHANGE.nOutEvents, this.nOut, true);
      this.engine = null;
      this.x = 0;

      // Outputs: the main port (mono feeds every channel).
      m.views();
      const outCh = e.getUint32(x + EXCHANGE.outChannels, true);
      const out = this.mainOut >= 0 ? this.outputs[this.mainOut] : undefined;
      for (let c = 0; c < outCh; c++) {
        const dst = e.getUint32(x + EXCHANGE.outputs + 4 * c, true) >> 2;
        if (!dst) continue;
        if (out && out.channels > 0) {
          const s = out.bufs[Math.min(c, out.channels - 1)]! >> 2;
          const p = m.f32;
          for (let i = 0; i < frames; i++) ef[dst + i] = p[s + i]!;
        } else {
          for (let i = 0; i < frames; i++) ef[dst + i] = 0;
        }
      }
      return status;
    } catch (err) {
      this.phase = "main";
      this.engine = null;
      this.x = 0;
      this.fail(err);
      return STATUS.faulted;
    }
  }

  /** The next input event slot, header written. */
  private slot(size: number, time: number, type: number): number {
    const dv = this.m.dv;
    const p = this.evBase + this.nIn * EVENT_SLOT;
    this.nIn++;
    dv.setUint32(p + EVENT.size, size, true);
    dv.setUint32(p + EVENT.time, time, true);
    dv.setUint16(p + EVENT.spaceId, CORE_EVENT_SPACE, true);
    dv.setUint16(p + EVENT.type, type, true);
    dv.setUint32(p + EVENT.flags, EVENT_IS_LIVE, true);
    return p;
  }

  private note(time: number, type: number, noteId: number, channel: number, key: number, velocity: number): void {
    const dv = this.m.dv;
    const p = this.slot(NOTE.size, time, type);
    dv.setInt32(p + NOTE.noteId, noteId, true);
    dv.setInt16(p + NOTE.port, 0, true);
    dv.setInt16(p + NOTE.channel, channel, true);
    dv.setInt16(p + NOTE.key, key, true);
    dv.setFloat64(p + NOTE.velocity, velocity, true);
  }

  private midi(time: number, a: number, b: number, c: number): void {
    const dv = this.m.dv;
    const p = this.slot(MIDI.size, time, EV.midi);
    dv.setUint16(p + MIDI.port, 0, true);
    dv.setUint8(p + MIDI.data, a);
    dv.setUint8(p + MIDI.data + 1, b);
    dv.setUint8(p + MIDI.data + 2, c);
  }

  /** The block's `EventRec`s → CLAP events in the plugin's memory. */
  private writeEvents(engine: EngineView, x: number): void {
    const e = engine.dv;
    const dv = this.m.dv;
    const n = Math.min(e.getUint32(x + EXCHANGE.nEvents, true), e.getUint32(x + EXCHANGE.eventCapacity, true));
    const recs = e.getUint32(x + EXCHANGE.events, true);
    this.nIn = 0;
    for (let i = 0; i < n && this.nIn < IN_EVENT_SLOTS - 1; i++) {
      const r = recs + i * REC.size;
      const time = e.getUint32(r + REC.offset, true);
      const kind = e.getUint8(r + REC.kind);
      const channel = e.getUint8(r + REC.channel);
      const key = e.getUint8(r + REC.key);
      const id = e.getUint32(r + REC.id, true);
      const value = e.getFloat64(r + REC.value, true);
      switch (kind) {
        case KIND.noteOn:
          this.note(time, EV.noteOn, id | 0, channel, key, value);
          break;
        case KIND.noteOff:
          this.note(time, EV.noteOff, id | 0, channel, key, value);
          break;
        case KIND.noteChoke:
          this.note(time, EV.noteChoke, id | 0, channel, key, 0);
          break;
        case KIND.allNotesOff:
          // Wildcard note-off (CLAP dialect) + CC 123 (MIDI dialect).
          this.note(time, EV.noteOff, -1, -1, -1, 0);
          this.midi(time, 0xb0, 123, 0);
          break;
        case KIND.param: {
          const p = this.slot(PARAM_VALUE.size, time, EV.paramValue);
          dv.setUint32(p + PARAM_VALUE.paramId, id, true);
          dv.setUint32(p + PARAM_VALUE.cookie, 0, true);
          dv.setInt32(p + PARAM_VALUE.noteId, -1, true);
          dv.setInt16(p + PARAM_VALUE.port, -1, true);
          dv.setInt16(p + PARAM_VALUE.channel, -1, true);
          dv.setInt16(p + PARAM_VALUE.key, -1, true);
          dv.setFloat64(p + PARAM_VALUE.value, value, true);
          break;
        }
        case KIND.midi:
          this.midi(time, id & 0xff, (id >> 8) & 0xff, (id >> 16) & 0xff);
          break;
        case KIND.noteExpression: {
          const p = this.slot(NOTE_EXPR.size, time, EV.noteExpression);
          dv.setInt32(p + NOTE_EXPR.expressionId, e.getUint8(r + REC.extra), true);
          dv.setInt32(p + NOTE_EXPR.noteId, id | 0, true);
          dv.setInt16(p + NOTE_EXPR.port, 0, true);
          dv.setInt16(p + NOTE_EXPR.channel, channel, true);
          dv.setInt16(p + NOTE_EXPR.key, key, true);
          dv.setFloat64(p + NOTE_EXPR.value, value, true);
          break;
        }
        default:
          break;
      }
    }
  }

  /** A CLAP fixed-point time (`clap_beattime` / `clap_sectime`, 64-bit) at `p`. */
  private fixed(p: number, v: number): void {
    const n = Math.round(v * TIME_FACTOR);
    const dv = this.m.dv;
    dv.setUint32(p, ((n % 4294967296) + 4294967296) % 4294967296, true);
    dv.setInt32(p + 4, Math.floor(n / 4294967296), true);
  }

  private writeTransport(e: DataView, x: number): void {
    const dv = this.m.dv;
    const t = this.transport;
    const f = e.getUint32(x + EXCHANGE.flags, true);
    let flags = TRANSPORT_FLAGS.hasTempo | TRANSPORT_FLAGS.hasBeatsTimeline | TRANSPORT_FLAGS.hasSecondsTimeline | TRANSPORT_FLAGS.hasTimeSignature;
    if (f & XFLAGS.playing) flags |= TRANSPORT_FLAGS.isPlaying;
    if (f & XFLAGS.recording) flags |= TRANSPORT_FLAGS.isRecording;
    if (f & XFLAGS.loopActive) flags |= TRANSPORT_FLAGS.isLoopActive;
    dv.setUint32(t + TRANSPORT.flags, flags, true);
    this.fixed(t + TRANSPORT.songPosBeats, e.getFloat64(x + EXCHANGE.position, true));
    this.fixed(t + TRANSPORT.songPosSeconds, e.getFloat64(x + EXCHANGE.seconds, true));
    dv.setFloat64(t + TRANSPORT.tempo, e.getFloat64(x + EXCHANGE.bpm, true), true);
    dv.setFloat64(t + TRANSPORT.tempoInc, e.getFloat64(x + EXCHANGE.tempoInc, true), true);
    this.fixed(t + TRANSPORT.loopStartBeats, e.getFloat64(x + EXCHANGE.loopStart, true));
    this.fixed(t + TRANSPORT.loopEndBeats, e.getFloat64(x + EXCHANGE.loopEnd, true));
    this.fixed(t + TRANSPORT.loopStartSeconds, 0);
    this.fixed(t + TRANSPORT.loopEndSeconds, 0);
    this.fixed(t + TRANSPORT.barStart, e.getFloat64(x + EXCHANGE.barStart, true));
    dv.setInt32(t + TRANSPORT.barNumber, e.getInt32(x + EXCHANGE.barNumber, true), true);
    dv.setUint16(t + TRANSPORT.tsigNum, e.getUint32(x + EXCHANGE.tsNum, true), true);
    dv.setUint16(t + TRANSPORT.tsigDenom, e.getUint32(x + EXCHANGE.tsDen, true), true);
  }

  /** **RT.** `clap_plugin.reset`. */
  reset(): void {
    if (this.failed || !this.active) return;
    this.phase = "audio";
    try {
      this.fns.reset(this.plugin);
    } catch (e) {
      this.fail(e);
    } finally {
      this.phase = "main";
    }
  }

  destroy(): void {
    try {
      if (this.processing) {
        this.phase = "audio";
        this.fns.stop(this.plugin);
        this.phase = "main";
      }
      if (this.active) this.fns.deactivate(this.plugin);
      this.m.fn(this.m.dv.getUint32(this.plugin + PLUGIN.destroy, true))(this.plugin);
      this.m.fn(this.m.dv.getUint32(this.m.entry + ENTRY.deinit, true))();
    } catch {
      // A failing teardown leaves nothing to clean: the instance is dropped anyway.
    }
    this.processing = false;
    this.active = false;
  }

  // ─── Main-thread calls ──────────────────────────────────────────────────────────────

  private extFn(ext: number, offset: number): Fn {
    return this.m.fn(this.m.dv.getUint32(ext + offset, true));
  }

  /** `clap_plugin_state.save`; `null` without the extension. */
  saveState(): Uint8Array | null {
    if (this.failed) throw new Error(this.failed);
    if (!this.ext.state) return null;
    this.streamOut = [];
    const ok = this.extFn(this.ext.state, STATE_EXT.save)(this.plugin, this.stream.out);
    const parts = this.streamOut;
    this.streamOut = [];
    if (!ok) throw new Error(`${this.desc.id} failed to save its state`);
    const total = parts.reduce((n, p) => n + p.length, 0);
    const out = new Uint8Array(total);
    let at = 0;
    for (const p of parts) {
      out.set(p, at);
      at += p.length;
    }
    return out;
  }

  loadState(bytes: Uint8Array): void {
    if (!this.ext.state) return;
    this.streamData = bytes.slice();
    this.streamPos = 0;
    const ok = this.extFn(this.ext.state, STATE_EXT.load)(this.plugin, this.stream.in);
    this.streamData = new Uint8Array(0);
    if (!ok) this.opts.log?.(LOG.warning, `${this.desc.id} refused its saved state`);
  }

  /** `[id, value, …]` of every param. */
  paramValues(): Float64Array {
    if (!this.ext.params) return new Float64Array(0);
    const count = this.extFn(this.ext.params, PARAMS_EXT.count)(this.plugin);
    const getInfo = this.extFn(this.ext.params, PARAMS_EXT.getInfo);
    const getValue = this.extFn(this.ext.params, PARAMS_EXT.getValue);
    const out: number[] = [];
    const info = this.scratch;
    const val = this.scratch + PARAM_INFO.size;
    for (let i = 0; i < count; i++) {
      if (!getInfo(this.plugin, i, info)) continue;
      this.m.views();
      const id = this.m.dv.getUint32(info + PARAM_INFO.id, true);
      if (!getValue(this.plugin, id, val)) continue;
      this.m.views();
      out.push(id, this.m.dv.getFloat64(val, true));
    }
    return new Float64Array(out);
  }

  /** `value_to_text`. */
  paramText(id: number, value: number): string | null {
    if (!this.ext.params) return null;
    const buf = this.scratch + PARAM_INFO.size + 16;
    if (!this.extFn(this.ext.params, PARAMS_EXT.valueToText)(this.plugin, id, value, buf, 256)) return null;
    return this.m.cstring(buf, 256);
  }

  /** Every param's metadata (and labels of small stepped ranges), for the scan. */
  paramInfos(): ParamInfo[] {
    if (!this.ext.params) return [];
    const count = this.extFn(this.ext.params, PARAMS_EXT.count)(this.plugin);
    const getInfo = this.extFn(this.ext.params, PARAMS_EXT.getInfo);
    const out: ParamInfo[] = [];
    const info = this.scratch;
    for (let i = 0; i < count; i++) {
      if (!getInfo(this.plugin, i, info)) continue;
      this.m.views();
      const dv = this.m.dv;
      const p: ParamInfo = {
        id: dv.getUint32(info + PARAM_INFO.id, true),
        flags: dv.getUint32(info + PARAM_INFO.flags, true),
        name: this.m.cstring(info + PARAM_INFO.name, 256),
        module: this.m.cstring(info + PARAM_INFO.module, 1024),
        min: dv.getFloat64(info + PARAM_INFO.min, true),
        max: dv.getFloat64(info + PARAM_INFO.max, true),
        default: dv.getFloat64(info + PARAM_INFO.default, true),
      };
      const stepped = (p.flags & (PARAM_FLAGS.stepped | PARAM_FLAGS.enum)) !== 0;
      if (stepped && p.max > p.min && p.max - p.min <= 128 && Number.isInteger(p.min) && Number.isInteger(p.max)) {
        const labels: string[] = [];
        for (let v = p.min; v <= p.max; v++) labels.push(this.paramText(p.id, v) ?? String(v));
        p.labels = labels;
      }
      out.push(p);
    }
    return out;
  }

  /** Audio ports on one side; `null` without the extension. */
  audioPorts(isInput: boolean): PortInfo[] | null {
    if (!this.ext.audioPorts) return null;
    const count = this.extFn(this.ext.audioPorts, AUDIO_PORTS_EXT.count)(this.plugin, isInput ? 1 : 0);
    const get = this.extFn(this.ext.audioPorts, AUDIO_PORTS_EXT.get);
    const info = this.scratch;
    const out: PortInfo[] = [];
    for (let i = 0; i < count; i++) {
      if (!get(this.plugin, i, isInput ? 1 : 0, info)) {
        out.push({ channels: 0, main: false });
        continue;
      }
      this.m.views();
      out.push({
        channels: this.m.dv.getUint32(info + AUDIO_PORT_INFO.channelCount, true),
        main: (this.m.dv.getUint32(info + AUDIO_PORT_INFO.flags, true) & AUDIO_PORT_IS_MAIN) !== 0,
      });
    }
    return out;
  }

  /** Note input ports, and whether the first takes the CLAP dialect. */
  noteInputs(): { count: number; clapDialect: boolean } {
    if (!this.ext.notePorts) return { count: 0, clapDialect: false };
    const count = this.extFn(this.ext.notePorts, NOTE_PORTS_EXT.count)(this.plugin, 1);
    let clapDialect = false;
    if (count > 0 && this.extFn(this.ext.notePorts, NOTE_PORTS_EXT.get)(this.plugin, 0, 1, this.scratch)) {
      this.m.views();
      clapDialect = (this.m.dv.getUint32(this.scratch + NOTE_PORT_INFO.supportedDialects, true) & NOTE_DIALECT_CLAP) !== 0;
    }
    return { count, clapDialect };
  }

  get hasWebview(): boolean {
    return this.ext.webview !== 0;
  }

  // ─── Web UI (`clap.webview/3`) ──────────────────────────────────────────────────────

  /** Open the UI: its start URI (`get_uri`), and `clap.gui` told when it supports webviews. */
  openUi(): string {
    if (!this.ext.webview) throw new Error("this plugin has no web ui");
    const getUri = this.extFn(this.ext.webview, WEBVIEW_EXT.getUri);
    const cap = 4096;
    const buf = this.scratch + PARAM_INFO.size;
    const n = getUri(this.plugin, buf, cap);
    if (n <= 0 || n > cap) throw new Error("the plugin gave no ui address");
    const uri = this.m.cstring(buf, cap);
    if (this.ext.gui) {
      const api = this.m.writeString("webview");
      if (this.extFn(this.ext.gui, GUI_EXT.isApiSupported)(this.plugin, api, 0)) {
        if (this.extFn(this.ext.gui, GUI_EXT.create)(this.plugin, api, 0)) this.extFn(this.ext.gui, GUI_EXT.show)(this.plugin);
      }
    }
    this.uiOpen = true;
    return uri;
  }

  closeUi(): void {
    if (!this.uiOpen) return;
    this.uiOpen = false;
    if (this.ext.gui) {
      try {
        this.extFn(this.ext.gui, GUI_EXT.hide)(this.plugin);
        this.extFn(this.ext.gui, GUI_EXT.destroy)(this.plugin);
      } catch (e) {
        this.fail(e);
      }
    }
  }

  /** `get_resource`: a file of the UI, by absolute path. */
  resource(path: string): { mime: string; bytes: Uint8Array } | null {
    if (!this.ext.webview) return null;
    const mime = this.scratch + PARAM_INFO.size;
    this.streamOut = [];
    const ok = this.extFn(this.ext.webview, WEBVIEW_EXT.getResource)(this.plugin, this.m.writeString(path), mime, 256, this.stream.out);
    const parts = this.streamOut;
    this.streamOut = [];
    if (!ok) return null;
    const total = parts.reduce((n, p) => n + p.length, 0);
    const bytes = new Uint8Array(total);
    let at = 0;
    for (const p of parts) {
      bytes.set(p, at);
      at += p.length;
    }
    return { mime: this.m.cstring(mime, 256), bytes };
  }

  /** A message from the UI (`receive`); over `MAX_UI_MESSAGE_BYTES`, dropped. */
  receive(bytes: Uint8Array): boolean {
    if (!this.ext.webview || !this.uiOpen || this.failed || bytes.length > MAX_UI_MESSAGE_BYTES) return false;
    try {
      // The buffer only grows (the plugin's memory has no `free` we can count on): bounded by the cap.
      if (bytes.length > this.msg.cap) this.msg = { ptr: this.m.alloc(bytes.length), cap: bytes.length };
      this.m.views();
      this.m.u8.set(bytes, this.msg.ptr);
      return this.extFn(this.ext.webview, WEBVIEW_EXT.receive)(this.plugin, this.msg.ptr, bytes.length) !== 0;
    } catch (e) {
      this.fail(e);
      return false;
    }
  }
}
