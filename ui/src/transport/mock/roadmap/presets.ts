/**
 * Mock of `Preset::*` (v0.2, owned by `presets`; CONTRACTS.md §12.5), same rules as the
 * engine (`crates/ether-controller/src/presets`):
 * - factory presets = the real `.etherpreset` files embedded in `ether-devices` (every
 *   device node's folder, picked up by `import.meta.glob`), ids `"<device-key>/<slug>"`;
 * - user presets = an in-memory user library per MockTransport, ids
 *   `"<device-key>/<name>.etherpreset"` (plugins: `plugins/<format>/<id>/…`);
 * - `Load` is one undo step (every descriptor param set: the preset's value clamped, or the
 *   default when missing); plugins get the param mirror (the mock has no plugin state);
 * - rack presets (v0.3, `rack-presets`, CONTRACTS.md §13.9): `Save` of a rack stores its
 *   chains, chain devices, the rack's modulators and the mappings inside it (`rack`); `Load`
 *   of such a preset needs `seed` and replaces them in the same undo step, ids
 *   `deriveId(seed, i)` in the engine's order (chains, each chain's devices, modulators,
 *   mappings), through the mock's `Rack::*` / `Modulation::*` reducers;
 * - `Save`/`Rename`/`Delete`/`SetMeta` are runtime and emit `PresetEvent::Changed`; names
 *   are unique per device type (case-insensitive), `Save` replaces only with `overwrite`.
 */

import type {
  BuiltinDeviceType,
  Color,
  Command,
  Device,
  DeviceId,
  ModSource,
  ModulatorKind,
  PresetCommand,
  PresetDevice,
  PresetInfo,
  PresetMeta,
  PresetRef,
  Project,
  RackChainId,
  Zone,
  ReplyValue,
} from "@/generated";
import { deriveId } from "@/features/comping/model";
import { cmd } from "../../cmd";
import { builtinDescriptor, clampParam, newBuiltinDevice } from "../builtinDevices";
import { fail } from "../documentReducer";
import type { MockHost } from "./host";

// The `.etherpreset` rack structure (`ether_model::preset::PresetRack`, version 2). It is a
// file format, not a wire type, so it is not in `@/generated`.
type PresetModSource = { type: "Macro"; index: number } | { type: "Modulator"; index: number };
type PresetModTarget = { type: "Rack" } | { type: "ChainDevice"; chain: number; device: number };
export interface PresetModMapping {
  source: PresetModSource;
  target: PresetModTarget;
  param: number;
  depth: number;
}
export interface PresetChainDevice {
  name: string;
  enabled: boolean;
  device: PresetDevice;
  params: Record<number, number>;
  kind: null;
  state: null;
}
export interface PresetChain {
  name: string;
  color: Color | null;
  volume: number;
  pan: number;
  mute: boolean;
  solo: boolean;
  keys: Zone;
  velocities: Zone;
  select: Zone;
  devices: PresetChainDevice[];
}
export interface PresetRack {
  chains: PresetChain[];
  modulators: { name: string; kind: ModulatorKind; params: Record<number, number> }[];
  mappings: PresetModMapping[];
}

/** The fields of an `.etherpreset` file the mock uses. */
interface PresetFileJson {
  format: string;
  preset: {
    name: string;
    device: PresetDevice;
    meta?: Partial<PresetMeta>;
    params?: Record<string, number>;
    rack?: PresetRack | null;
  };
}

interface StoredPreset {
  name: string;
  device: PresetDevice;
  meta: PresetMeta;
  params: Record<number, number>;
  /** Rack structure (v0.3, rack devices only). */
  rack?: PresetRack;
}

const FILES = import.meta.glob<string>("../../../../../crates/ether-devices/presets/*/*.etherpreset", {
  query: "?raw",
  import: "default",
  eager: true,
});

/** `BuiltinDeviceType::key()`: "PolySynth" → "poly-synth". */
export const deviceKey = (t: BuiltinDeviceType) => t.replace(/(?!^)([A-Z])/g, "-$1").toLowerCase();

function parseFile(json: string): StoredPreset | null {
  try {
    const f = JSON.parse(json) as PresetFileJson;
    if (f.format !== "ethereal-preset") return null;
    const params: Record<number, number> = {};
    for (const [k, v] of Object.entries(f.preset.params ?? {})) params[Number(k)] = v;
    return {
      name: f.preset.name,
      device: f.preset.device,
      meta: {
        tags: f.preset.meta?.tags ?? [],
        author: f.preset.meta?.author ?? null,
        description: f.preset.meta?.description ?? null,
      },
      params,
      ...(f.preset.rack ? { rack: f.preset.rack } : {}),
    };
  } catch {
    return null;
  }
}

/** Factory presets by id, parsed once. */
const FACTORY: ReadonlyMap<string, StoredPreset> = new Map(
  Object.entries(FILES).flatMap(([path, json]) => {
    const m = /presets\/([^/]+)\/([^/]+)\.etherpreset$/.exec(path);
    const p = parseFile(json);
    return m && p ? [[`${m[1]}/${m[2]}`, p] as const] : [];
  }),
);

export function sameDevice(a: PresetDevice, b: PresetDevice): boolean {
  if (a.type === "Builtin" && b.type === "Builtin") return a.device === b.device;
  if (a.type === "Plugin" && b.type === "Plugin") return a.format === b.format && a.plugin_id === b.plugin_id;
  return false;
}

function presetDevice(d: Device): PresetDevice {
  if (d.kind.type === "Builtin") return { type: "Builtin", device: d.kind.device.type };
  const { format, plugin_id, name, vendor } = d.kind.plugin;
  return { type: "Plugin", format, plugin_id, name, vendor };
}

function deviceDir(d: PresetDevice): string {
  if (d.type === "Builtin") return deviceKey(d.device);
  return `plugins/${d.format.toLowerCase()}/${d.plugin_id.replace(/[^A-Za-z0-9._-]/g, "_")}`;
}

/** Same file-name rules as the engine (`presets::files::file_stem`). */
export function fileStem(name: string): string {
  const cleaned = [...name]
    .map((c) => (c.charCodeAt(0) < 0x20 || c.charCodeAt(0) === 0x7f || '/\\:*?"<>|'.includes(c) ? "-" : c))
    .slice(0, 80)
    .join("");
  const t = cleaned.trim().replace(/^\.+/, "").replace(/\.+$/, "").trim();
  return t || "Preset";
}

function normalizeMeta(m: PresetMeta): PresetMeta {
  const tags = [...new Set(m.tags.map((t) => t.trim().toLowerCase()).filter((t) => t))].sort();
  const text = (s: string | null) => (s && s.trim() ? s.trim() : null);
  return { tags, author: text(m.author), description: text(m.description) };
}

function matches(p: StoredPreset, text: string | null): boolean {
  const q = text?.trim().toLowerCase();
  if (!q) return true;
  return p.name.toLowerCase().includes(q) || p.meta.tags.some((t) => t.toLowerCase().includes(q)) || (p.meta.author ?? "").toLowerCase().includes(q);
}

const info = (source: PresetRef["source"], id: string, p: StoredPreset): PresetInfo => ({
  preset: { source, id },
  name: p.name,
  device: p.device,
  meta: p.meta,
});

const byName = (a: PresetInfo, b: PresetInfo) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()) || a.preset.id.localeCompare(b.preset.id);

/** The runtime simulation (one per MockTransport). */
export class MockPresets {
  /** The user library: id → preset. */
  private readonly user = new Map<string, StoredPreset>();

  constructor(private readonly host: MockHost) {}

  command(c: PresetCommand): ReplyValue {
    switch (c.type) {
      case "List":
        return { type: "Presets", presets: this.list(c.device, c.text) };
      case "Load":
        this.load(c.device, c.preset, c.seed ?? null);
        return { type: "Unit" };
      case "Save":
        return this.changed({
          type: "Preset",
          preset: this.save(c.device, c.name, c.meta, c.overwrite),
        });
      case "Rename":
        return this.changed({
          type: "Preset",
          preset: this.rename(c.preset, c.name),
        });
      case "Delete":
        this.userPreset(c.preset);
        this.user.delete(c.preset.id);
        return this.changed({ type: "Unit" });
      case "SetMeta": {
        const p = this.userPreset(c.preset);
        p.meta = normalizeMeta(c.meta);
        return this.changed({
          type: "Preset",
          preset: info("User", c.preset.id, p),
        });
      }
    }
  }

  private changed(r: ReplyValue): ReplyValue {
    this.host.emit({ type: "Preset", event: { type: "Changed" } });
    return r;
  }

  private list(device: PresetDevice | null, text: string | null): PresetInfo[] {
    const pick = (p: StoredPreset) => (!device || sameDevice(device, p.device)) && matches(p, text);
    const factory = [...FACTORY].filter(([, p]) => pick(p)).map(([id, p]) => info("Factory", id, p));
    const user = [...this.user].filter(([, p]) => pick(p)).map(([id, p]) => info("User", id, p));
    return [...factory.sort(byName), ...user.sort(byName)];
  }

  private userPreset(ref: PresetRef): StoredPreset {
    if (ref.source !== "User") fail("InvalidArgument", "factory presets are read-only");
    const p = this.user.get(ref.id);
    if (!p) fail("NotFound", `preset ${ref.id}`);
    return p;
  }

  private device(id: string): Device {
    const d = this.host.project().devices[id];
    if (!d) fail("NotFound", `device ${id}`);
    return d;
  }

  private named(device: PresetDevice, name: string, except?: string): string | undefined {
    const n = name.toLowerCase();
    for (const [id, p] of this.user) if (id !== except && sameDevice(p.device, device) && p.name.toLowerCase() === n) return id;
    return undefined;
  }

  private freeId(dir: string, stem: string, except?: string): string {
    for (let n = 1; ; n++) {
      const id = `${dir}/${n === 1 ? stem : `${stem} ${n}`}.etherpreset`;
      const taken = [...this.user.keys()].some((k) => k.toLowerCase() === id.toLowerCase());
      if (!taken || id === except) return id;
    }
  }

  private load(deviceId: string, ref: PresetRef, seed: RackChainId | null): void {
    const p = ref.source === "Factory" ? FACTORY.get(ref.id) : this.userPreset(ref);
    if (!p) fail("NotFound", `factory preset ${ref.id}`);
    const d = this.device(deviceId);
    if (!sameDevice(presetDevice(d), p.device)) fail("InvalidArgument", `preset "${p.name}" is for another device type`);
    if (p.rack && !seed) fail("InvalidArgument", "loading a rack preset with chains needs a seed for the new ids");
    const commands: Command[] = p.rack && seed ? rackLoadCommands(this.host.project(), d.id, p.rack, seed) : [];
    if (d.kind.type === "Builtin") {
      for (const param of builtinDescriptor(d.kind.device).params) {
        const v = param.id in p.params ? clampParam(param, p.params[param.id]!) : param.default;
        if (d.params[param.id] !== v)
          commands.push(
            cmd("Device", {
              type: "SetParam",
              device: d.id,
              param: param.id,
              value: v,
            }),
          );
      }
    } else {
      for (const [id, v] of Object.entries(p.params))
        commands.push(
          cmd("Device", {
            type: "SetParam",
            device: d.id,
            param: Number(id),
            value: v,
          }),
        );
    }
    if (commands.length > 0) this.host.applyDocument(commands, "Load Preset");
  }

  private save(deviceId: string, rawName: string, meta: PresetMeta, overwrite: boolean): PresetInfo {
    const name = rawName.trim();
    if (!name) fail("InvalidArgument", "a preset needs a name");
    const d = this.device(deviceId);
    const device = presetDevice(d);
    const existing = this.named(device, name);
    if (existing && !overwrite) fail("InvalidArgument", `a preset named "${name}" already exists`);
    const params: Record<number, number> = {};
    if (d.kind.type === "Builtin") {
      for (const param of builtinDescriptor(d.kind.device).params) params[param.id] = d.params[param.id] ?? param.default;
    } else {
      for (const [id, v] of Object.entries(d.params)) if (v !== undefined) params[Number(id)] = v;
    }
    const id = existing ?? this.freeId(deviceDir(device), fileStem(name));
    const rack = isRack(d) ? rackSnapshot(this.host.project(), d.id) : undefined;
    const p: StoredPreset = { name, device, meta: normalizeMeta(meta), params, ...(rack ? { rack } : {}) };
    this.user.set(id, p);
    return info("User", id, p);
  }

  private rename(ref: PresetRef, rawName: string): PresetInfo {
    const name = rawName.trim();
    if (!name) fail("InvalidArgument", "a preset needs a name");
    const p = this.userPreset(ref);
    if (this.named(p.device, name, ref.id)) fail("InvalidArgument", `a preset named "${name}" already exists`);
    const dir = ref.id.includes("/") ? ref.id.slice(0, ref.id.lastIndexOf("/")) : "";
    const id = this.freeId(dir, fileStem(name), ref.id);
    const renamed = { ...p, name };
    this.user.delete(ref.id);
    this.user.set(id, renamed);
    return info("User", id, renamed);
  }
}

// ─── Rack presets (v0.3, `rack-presets`) ────────────────────────────────────────────────

const RACKS: ReadonlyArray<BuiltinDeviceType> = ["InstrumentRack", "AudioEffectRack", "MidiEffectRack"];
const isRack = (d: Device) => d.kind.type === "Builtin" && RACKS.includes(d.kind.device.type);
const byOrderKey = <T extends { order: string; id: string }>(a: T, b: T) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1);

const chainsOf = (p: Project, rack: DeviceId) =>
  Object.values(p.rack_chains)
    .filter((c) => c.rack === rack)
    .sort(byOrderKey);
const chainDevicesOf = (p: Project, chain: RackChainId) =>
  Object.values(p.devices)
    .filter((d) => d.chain === chain)
    .sort(byOrderKey);
const modulatorsOf = (p: Project, device: DeviceId) =>
  Object.values(p.modulators)
    .filter((m) => m.device === device)
    .sort(byOrderKey);

const targetKey = (m: PresetModMapping) =>
  m.target.type === "Rack"
    ? [0, 0, 0, m.param, m.source.type === "Macro" ? 0 : 1, m.source.index]
    : [1, m.target.chain, m.target.device, m.param, m.source.type === "Macro" ? 0 : 1, m.source.index];

/** The engine's `presets::rack::snapshot` (built-ins store every descriptor param). */
export function rackSnapshot(p: Project, rack: DeviceId): PresetRack {
  const where = new Map<DeviceId, [number, number]>();
  const chains = chainsOf(p, rack).map((c, ci) => ({
    name: c.name,
    color: c.color,
    volume: c.volume,
    pan: c.pan,
    mute: c.mute,
    solo: c.solo,
    keys: c.keys,
    velocities: c.velocities,
    select: c.select,
    devices: chainDevicesOf(p, c.id).map((d, di): PresetChainDevice => {
      where.set(d.id, [ci, di]);
      const params: Record<number, number> = {};
      if (d.kind.type === "Builtin") {
        for (const info of builtinDescriptor(d.kind.device).params) params[info.id] = d.params[info.id] ?? info.default;
      } else {
        for (const [k, v] of Object.entries(d.params)) if (v !== undefined) params[Number(k)] = v;
      }
      return { name: d.name, enabled: d.enabled, device: presetDevice(d), params, kind: null, state: null };
    }),
  }));
  const mods = modulatorsOf(p, rack);
  const mappings: PresetModMapping[] = [];
  for (const m of Object.values(p.mod_mappings)) {
    let source: PresetModSource;
    if (m.source.type === "Macro") {
      if (m.source.rack !== rack) continue;
      source = { type: "Macro", index: m.source.index };
    } else {
      const modulator = m.source.modulator;
      const index = mods.findIndex((x) => x.id === modulator);
      if (index < 0) continue;
      source = { type: "Modulator", index };
    }
    const at = where.get(m.device);
    if (m.device !== rack && !at) continue;
    mappings.push({
      source,
      target: at ? { type: "ChainDevice", chain: at[0], device: at[1] } : { type: "Rack" },
      param: m.param,
      depth: m.depth,
    });
  }
  mappings.sort((a, b) => {
    const [ka, kb] = [targetKey(a), targetKey(b)];
    for (let i = 0; i < ka.length; i++) if (ka[i] !== kb[i]) return ka[i]! - kb[i]!;
    return 0;
  });
  return {
    chains,
    modulators: mods.map((m) => ({ name: m.name, kind: m.kind, params: { ...m.params } })),
    mappings,
  };
}

/**
 * The document commands replacing `rack`'s structure with `preset` (the same undo step as
 * the rack's param sets that follow): the engine's `presets::rack::rebuild`, new ids
 * `deriveId(seed, i)` for chains, then each chain's devices, then modulators, then mappings.
 */
export function rackLoadCommands(p: Project, rack: DeviceId, preset: PresetRack, seed: string): Command[] {
  const out: Command[] = [];
  const mods = modulatorsOf(p, rack);
  const fromRack = (s: ModSource) => (s.type === "Macro" ? s.rack === rack : mods.some((x) => x.id === s.modulator));
  for (const m of Object.values(p.mod_mappings)) if (fromRack(m.source)) out.push(cmd("Modulation", { type: "Unmap", id: m.id }));
  for (const m of mods) out.push(cmd("Modulation", { type: "RemoveModulator", id: m.id }));
  for (const c of chainsOf(p, rack)) out.push(cmd("Rack", { type: "RemoveChain", id: c.id }));
  let next = 0;
  const chainIds = preset.chains.map(() => deriveId(seed, next++));
  preset.chains.forEach((c, i) => {
    const id = chainIds[i]!;
    out.push(cmd("Rack", { type: "AddChain", id, rack, name: c.name, before: null }));
    if (c.color !== null) out.push(cmd("Rack", { type: "SetChainColor", id, color: c.color }));
    out.push(cmd("Rack", { type: "SetChainMix", id, volume: c.volume, pan: c.pan, mute: c.mute, solo: c.solo }));
    out.push(cmd("Rack", { type: "SetChainZones", id, keys: c.keys, velocities: c.velocities, select: c.select }));
  });
  const deviceIds = preset.chains.map((c, ci) =>
    c.devices.map((d) => {
      const id = deriveId(seed, next++);
      if (d.device.type !== "Builtin") fail("Unsupported", "plugins are not available in the mock engine");
      const device = newBuiltinDevice(d.device.device);
      const desc = builtinDescriptor(device);
      out.push(cmd("Rack", { type: "InsertDevice", id, chain: chainIds[ci]!, device: { type: "Builtin", device }, before: null }));
      if (d.name.trim() && d.name !== desc.name) out.push(cmd("Device", { type: "Rename", id, name: d.name }));
      if (!d.enabled) out.push(cmd("Device", { type: "SetEnabled", id, enabled: false }));
      for (const info of desc.params) {
        const v = d.params[info.id];
        const value = v !== undefined && Number.isFinite(v) ? clampParam(info, v) : info.default;
        if (value !== info.default) out.push(cmd("Device", { type: "SetParam", device: id, param: info.id, value }));
      }
      return id;
    }),
  );
  const modIds = preset.modulators.map((m) => {
    const id = deriveId(seed, next++);
    out.push(cmd("Modulation", { type: "AddModulator", id, device: rack, kind: m.kind, name: m.name }));
    for (const [param, value] of Object.entries(m.params)) {
      if (value !== undefined && Number.isFinite(value)) out.push(cmd("Modulation", { type: "SetModulatorParam", modulator: id, param: Number(param), value }));
    }
    return id;
  });
  for (const m of preset.mappings) {
    const id = deriveId(seed, next++);
    const source: ModSource = m.source.type === "Macro" ? { type: "Macro", rack, index: m.source.index } : { type: "Modulator", modulator: modIds[m.source.index]! };
    const device = m.target.type === "Rack" ? rack : deviceIds[m.target.chain]![m.target.device]!;
    out.push(cmd("Modulation", { type: "Map", id, source, device, param: m.param, depth: m.depth }));
  }
  return out;
}
