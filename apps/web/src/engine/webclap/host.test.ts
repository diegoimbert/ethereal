// The WebCLAP host glue against real plugins compiled to wasm: the repository's test
// plugin (crates/ether-webclap-fixture, built on demand) and two third-party examples
// (test-fixtures/webclap/examples). The engine side is played by a plain WebAssembly
// memory holding an `Exchange` laid out as crates/ether-wasm/src/webclap/node.rs does.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { beforeAll, describe, expect, it } from "vitest";
// @ts-expect-error: a plain .mjs build script
import { buildFixture } from "../../../scripts/build-webclap-fixture.mjs";
import { readBundle } from "./bundle";
import { WebclapHost } from "./host";
import { MAX_UI_MESSAGE_BYTES } from "./instance";
import { EXCHANGE, KIND, NOTE, REC, STATUS, XFLAGS } from "./layout";
import { scanModule } from "./scan";
import { moduleInfo, type ModuleInfo } from "./wasm";

const SR = 48_000;
const MAX = 128;
const examples = (p: string) => fileURLToPath(new URL(`../../../test-fixtures/webclap/examples/${p}`, import.meta.url));

/** The engine's side: an `Exchange`, stereo in/out buffers and event arrays. */
class FakeEngine {
  readonly memory = new WebAssembly.Memory({ initial: 4 });
  readonly x = 1024;
  readonly ins = [4096, 4096 + 4 * MAX];
  readonly outs = [8192, 8192 + 4 * MAX];
  readonly events = 12_288;
  readonly outEvents = 16_384;
  private n = 0;
  constructor() {
    const dv = this.dv;
    dv.setUint32(this.x + EXCHANGE.eventCapacity, 64, true);
    dv.setUint32(this.x + EXCHANGE.events, this.events, true);
    dv.setUint32(this.x + EXCHANGE.outEvents, this.outEvents, true);
    this.ins.forEach((p, c) => dv.setUint32(this.x + EXCHANGE.inputs + 4 * c, p, true));
    this.outs.forEach((p, c) => dv.setUint32(this.x + EXCHANGE.outputs + 4 * c, p, true));
    dv.setFloat64(this.x + EXCHANGE.bpm, 120, true);
    dv.setUint32(this.x + EXCHANGE.tsNum, 4, true);
    dv.setUint32(this.x + EXCHANGE.tsDen, 4, true);
  }
  get dv(): DataView {
    return new DataView(this.memory.buffer);
  }
  /** Set up a block of `frames` with input `input(i)` (stereo) and channel counts. */
  block(frames: number, input: (i: number) => number, inCh = 2, outCh = 2): void {
    const dv = this.dv;
    dv.setUint32(this.x + EXCHANGE.frames, frames, true);
    dv.setUint32(this.x + EXCHANGE.inChannels, inCh, true);
    dv.setUint32(this.x + EXCHANGE.outChannels, outCh, true);
    dv.setUint32(this.x + EXCHANGE.flags, XFLAGS.playing, true);
    const f = new Float32Array(this.memory.buffer);
    for (const p of this.ins) for (let i = 0; i < frames; i++) f[(p >> 2) + i] = input(i);
    for (const p of this.outs) for (let i = 0; i < MAX; i++) f[(p >> 2) + i] = NaN;
    this.n = 0;
    dv.setUint32(this.x + EXCHANGE.nEvents, 0, true);
  }
  event(offset: number, kind: number, id: number, value: number, channel = 0, key = 0): void {
    const dv = this.dv;
    const r = this.events + this.n * REC.size;
    dv.setUint32(r + REC.offset, offset, true);
    dv.setUint8(r + REC.kind, kind);
    dv.setUint8(r + REC.channel, channel);
    dv.setUint8(r + REC.key, key);
    dv.setUint32(r + REC.id, id, true);
    dv.setFloat64(r + REC.value, value, true);
    this.n++;
    dv.setUint32(this.x + EXCHANGE.nEvents, this.n, true);
  }
  out(c: number, frames: number): number[] {
    return [...new Float32Array(this.memory.buffer, this.outs[c], frames)];
  }
  outEventsList(): { kind: number; id: number; value: number }[] {
    const dv = this.dv;
    const n = dv.getUint32(this.x + EXCHANGE.nOutEvents, true);
    return Array.from({ length: n }, (_, i) => {
      const r = this.outEvents + i * REC.size;
      return { kind: dv.getUint8(r + REC.kind), id: dv.getUint32(r + REC.id, true), value: dv.getFloat64(r + REC.value, true) };
    });
  }
}

let fixture: { module: WebAssembly.Module; info: ModuleInfo };

beforeAll(() => {
  const path = buildFixture({ ifMissing: true }) as string;
  const bytes = new Uint8Array(readFileSync(path));
  fixture = { module: new WebAssembly.Module(bytes), info: moduleInfo(bytes) };
}, 600_000);

function setup(pluginId: string, state?: Uint8Array) {
  const engine = new FakeEngine();
  const ui: Uint8Array[] = [];
  const logs: string[] = [];
  const host = new WebclapHost({
    engineMemory: () => engine.memory,
    ui: (_device, bytes) => ui.push(bytes),
    log: (_device, severity, text) => logs.push(`${severity}:${text}`),
  });
  host.addModule("m", fixture.module, fixture.info);
  const handle = host.create("m", pluginId, "dev-1", SR, MAX, state);
  expect(host.ready(handle), host.error(handle)).toBe(1);
  return { engine, host, handle, ui, logs };
}

const text = (b: Uint8Array) => new TextDecoder().decode(b);

describe("the test plugin", () => {
  it("is scanned: plugins, params with labels, ports, notes, web ui", () => {
    const scan = scanModule("m", fixture.module, fixture.info);
    expect(scan.failed).toEqual([]);
    expect(scan.plugins.map((p) => p.id)).toEqual(["ether.test.gain", "ether.test.delay", "ether.test.synth"]);
    const gain = scan.plugins[0]!;
    expect(gain.name).toBe("Test Gain");
    expect(gain.features).toContain("audio-effect");
    expect(gain.params.map((p) => [p.id, p.name, p.min, p.max, p.default])).toEqual([
      [1, "gain", 0, 2, 1],
      [2, "mode", 0, 2, 0],
    ]);
    expect(gain.params[1]!.labels).toEqual(["soft", "medium", "hard"]);
    expect(gain.audioInputs).toEqual([{ channels: 2, main: true }]);
    expect(gain.webview).toBe(true);
    const synth = scan.plugins[2]!;
    expect(synth.audioInputs).toEqual([]);
    expect(synth.noteInputs).toBe(1);
    expect(synth.noteExpressions).toBe(true);
    expect(synth.webview).toBe(false);
  });

  it("processes with sample-accurate params, sleeps on silence", () => {
    const { engine, host, handle, logs } = setup("ether.test.gain");
    engine.block(128, () => 0.5);
    engine.event(64, KIND.param, 1, 0.5);
    const status = host.process(handle, engine.x);
    expect(status).toBe(1); // CONTINUE
    const left = engine.out(0, 128);
    expect(left.slice(0, 64).every((s) => s === 0.5)).toBe(true);
    expect(left.slice(64).every((s) => s === 0.25)).toBe(true);
    expect(engine.out(1, 128)[100]).toBe(0.25);
    // Silent input: SLEEP.
    engine.block(128, () => 0);
    expect(host.process(handle, engine.x)).toBe(4);
    expect(host.latency(handle)).toBe(0);
    // Thread checks held: init on main, start_processing on audio.
    expect(logs.filter((l) => l.startsWith("3:"))).toEqual([]);
    expect(logs).toContain("0:ether test plugin ready");
  });

  it("round-trips its state and reports values", () => {
    const a = setup("ether.test.gain");
    a.engine.block(32, () => 1);
    a.engine.event(0, KIND.param, 1, 1.5);
    a.engine.event(0, KIND.param, 2, 2);
    a.host.process(a.handle, a.engine.x);
    expect([...a.host.paramValues(a.handle)!]).toEqual([1, 1.5, 2, 2]);
    const state = a.host.saveState(a.handle)!;
    expect(state.length).toBe(16);
    const b = setup("ether.test.gain", state);
    expect([...b.host.paramValues(b.handle)!]).toEqual([1, 1.5, 2, 2]);
    b.engine.block(16, () => 1);
    b.host.process(b.handle, b.engine.x);
    expect(b.engine.out(0, 16)[0]).toBe(1.5);
  });

  it("reports latency and delays by it", () => {
    const { engine, host, handle } = setup("ether.test.delay");
    expect(host.latency(handle)).toBe(32);
    engine.block(128, (i) => (i === 0 ? 1 : 0));
    host.process(handle, engine.x);
    const out = engine.out(0, 128);
    expect(out.indexOf(1)).toBe(32);
  });

  it("plays notes from their sample, all-notes-off releases", () => {
    const { engine, host, handle } = setup("ether.test.synth");
    engine.block(128, () => 0, 0);
    engine.event(10, KIND.noteOn, 7, 0.75, 0, 60);
    expect(host.process(handle, engine.x)).toBe(1);
    const out = engine.out(0, 128);
    expect(out.slice(0, 10).every((s) => s === 0)).toBe(true);
    expect(out.slice(10).every((s) => s === 0.75)).toBe(true);
    engine.block(128, () => 0, 0);
    engine.event(5, KIND.allNotesOff, 0, 0);
    expect(host.process(handle, engine.x)).toBe(1);
    expect(engine.out(1, 128)[4]).toBe(0.75);
    expect(engine.out(1, 128)[5]).toBe(0);
    // Released and silent: it sleeps.
    engine.block(128, () => 0, 0);
    expect(host.process(handle, engine.x)).toBe(4);
  });

  it("talks to its web ui and reports the ui's edits", () => {
    const { engine, host, handle, ui } = setup("ether.test.gain");
    const inst = host.byDevice("dev-1")!;
    expect(inst.openUi()).toBe("/ui/index.html");
    const page = inst.resource("/ui/index.html")!;
    expect(page.mime).toBe("text/html");
    expect(text(page.bytes)).toContain("set:0.5");
    expect(inst.resource("/nope")).toBeNull();
    expect(inst.receive(new TextEncoder().encode("ready"))).toBe(true);
    expect(ui.map(text)).toEqual(["gain:1"]);
    expect(inst.receive(new TextEncoder().encode("set:0.5"))).toBe(true);
    expect(ui.map(text)).toEqual(["gain:1", "gain:0.5"]);
    // `mark_dirty`, then the edit comes out at the next block (`request_flush` woke it).
    expect(host.takeWake(handle)).toBe(true);
    engine.block(16, () => 1);
    host.process(handle, engine.x);
    expect(engine.out(0, 16)[0]).toBe(0.5);
    expect(engine.outEventsList()).toEqual([{ kind: KIND.param, id: 1, value: 0.5 }]);
    const notes = host.takeNotifications() as [number, number, number, number, string][];
    expect(notes.map((n) => n[1])).toEqual([NOTE.dirty, NOTE.begin, NOTE.param, NOTE.end]);
    expect(notes[2]!.slice(2, 4)).toEqual([1, 0.5]);
    expect(host.takeNotifications()).toBeUndefined();
    // Over the cap: dropped, the plugin never sees it.
    expect(inst.receive(new Uint8Array(MAX_UI_MESSAGE_BYTES + 1))).toBe(false);
    inst.closeUi();
    expect(inst.receive(new TextEncoder().encode("ready"))).toBe(false);
  });

  it("waits for its module, and fails cleanly", () => {
    const engine = new FakeEngine();
    const host = new WebclapHost({ engineMemory: () => engine.memory });
    const h = host.create("later", "ether.test.gain", "d", SR, MAX, new Uint8Array(new Float64Array([0.25, 0]).buffer));
    expect(host.ready(h)).toBe(0);
    engine.block(16, () => 1);
    expect(host.process(h, engine.x)).toBe(STATUS.pending);
    host.addModule("later", fixture.module, fixture.info);
    expect(host.ready(h)).toBe(1);
    const bad = host.create("later", "no.such.plugin", "d2", SR, MAX);
    expect(host.ready(bad)).toBe(-1);
    expect(host.error(bad)).toContain("no plugin no.such.plugin");
    expect(host.process(bad, engine.x)).toBe(STATUS.faulted);
    expect(host.process(12345, engine.x)).toBe(STATUS.faulted);
    host.destroy(h);
    expect(host.ready(h)).toBe(-1);
  });

  it("processes many blocks without growing", () => {
    const { engine, host, handle } = setup("ether.test.gain");
    for (let b = 0; b < 2000; b++) {
      engine.block(128, (i) => Math.sin(i));
      engine.event(b % 128, KIND.param, 1, (b % 3) / 2);
      host.process(handle, engine.x);
    }
    expect(host.takeNotifications()).toBeUndefined();
  });
});

describe("third-party examples", () => {
  it("clack gain: scans and processes", () => {
    const bytes = new Uint8Array(readFileSync(examples("clack/clack_plugin_gain.wasm")));
    const info = moduleInfo(bytes);
    const module = new WebAssembly.Module(bytes);
    const scan = scanModule("clack", module, info);
    expect(scan.failed).toEqual([]);
    expect(scan.plugins.length).toBe(1);
    const p = scan.plugins[0]!;
    expect(p.params.length).toBeGreaterThan(0);
    const engine = new FakeEngine();
    const host = new WebclapHost({ engineMemory: () => engine.memory });
    host.addModule("clack", module, info);
    const h = host.create("clack", p.id, "d", SR, MAX);
    expect(host.ready(h), host.error(h)).toBe(1);
    engine.block(128, (i) => Math.sin(i / 4));
    const gainParam = p.params[0]!;
    engine.event(0, KIND.param, gainParam.id, gainParam.default);
    expect(host.process(h, engine.x)).toBeGreaterThan(0);
    const out = engine.out(0, 128);
    expect(out.some((s) => Math.abs(s) > 0.01)).toBe(true);
    expect(out.every((s) => Number.isFinite(s))).toBe(true);
    const state = host.saveState(h);
    expect(state).not.toBeNull();
  });

  it("signalsmith basics: a bundle with shared memory and web uis", async () => {
    const bundle = await readBundle(new Uint8Array(readFileSync(examples("signalsmith-basics/basics.wclap.tar.gz"))));
    expect(bundle.files.has("html/index.html")).toBe(true);
    const info = moduleInfo(bundle.wasm);
    expect(info.memory?.shared).toBe(true);
    const module = new WebAssembly.Module(bundle.wasm as BufferSource);
    const scan = scanModule("basics", module, info);
    expect(scan.failed).toEqual([]);
    expect(scan.plugins.length).toBeGreaterThan(1);
    expect(scan.plugins.every((p) => p.webview)).toBe(true);
    const engine = new FakeEngine();
    const host = new WebclapHost({ engineMemory: () => engine.memory, ui: () => undefined });
    host.addModule("basics", module, info);
    for (const p of scan.plugins) {
      const h = host.create("basics", p.id, p.id, SR, MAX);
      expect(host.ready(h), `${p.id}: ${host.error(h)}`).toBe(1);
      for (let b = 0; b < 8; b++) {
        engine.block(128, (i) => 0.5 * Math.sin((b * 128 + i) / 8));
        expect(host.process(h, engine.x), p.id).toBeGreaterThan(0);
      }
      expect(
        engine.out(0, 128).every((s) => Number.isFinite(s)),
        p.id,
      ).toBe(true);
      const ui = host.byDevice(p.id)!.openUi();
      expect(ui).toMatch(/html\/index\.html/);
      host.destroy(h);
    }
  });
});
