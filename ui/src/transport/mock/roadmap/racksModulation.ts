/**
 * Mock of `Rack::*` and `Modulation::*` (v0.2, contracts-3). Owned by `racks-modulation`.
 * Rack chains, chain devices, macros (rack params 0..8) and Bitwig-style modulators with
 * mappings (`ether_model::{rack, modulation}`), mirroring
 * `crates/ether-controller/src/racks/mod.rs`: chain content rules (audio effect racks hold
 * audio effects, MIDI effect racks MIDI effects, no nesting), mapping scope (a modulator
 * reaches its device and, on a rack, the rack's chain devices and its selector; a macro its
 * rack's chain devices and the rack's non-macro params), one mapping per (source, target),
 * `Group` (modulators re-hosted on the rack) and the cascades the core reducer calls
 * (`onDeviceDeletedRacks`, `copyRackExtras`, `copyTrackRackExtras`).
 * `ListModulatorKinds` answers from the generated `devices/modulators.json`.
 */

import type {
  BuiltinDeviceType,
  Device,
  DeviceCategory,
  DeviceId,
  ModSource,
  Modulator,
  ModulationCommand,
  RackChain,
  RackChainId,
  RackCommand,
  ReplyValue,
  Zone,
} from "@/generated";
import { keyForInsert } from "@/state/orderKey";
import { builtinDescriptor, newBuiltinDevice } from "../builtinDevices";
import { defaultParams } from "../demoProject";
import { MODULATOR_DESCRIPTORS } from "../devices";
import { fail, type ReducerContext } from "../documentReducer";
import { byOrder, clamp, onDeviceDeleted } from "./shared";

const RACK_TYPES: ReadonlyArray<BuiltinDeviceType> = ["InstrumentRack", "AudioEffectRack", "MidiEffectRack"];
const SELECTOR_PARAM = 8;
const MACROS = 8;
const FULL: Zone = { lo: 0, hi: 127 };

export function isChainRack(d: Device): boolean {
  return d.kind.type === "Builtin" && RACK_TYPES.includes(d.kind.device.type);
}

function rackType(d: Device): BuiltinDeviceType | null {
  return d.kind.type === "Builtin" && RACK_TYPES.includes(d.kind.device.type) ? d.kind.device.type : null;
}

function device(ctx: ReducerContext, id: DeviceId): Device {
  return ctx.tx.get("Device", id) ?? fail("NotFound", `device ${id}`);
}

function rack(ctx: ReducerContext, id: DeviceId): Device {
  const d = device(ctx, id);
  if (!isChainRack(d) || d.pad !== null || d.chain != null) fail("InvalidArgument", `device ${id} is not a rack`);
  return d;
}

function chain(ctx: ReducerContext, id: RackChainId): RackChain {
  return ctx.tx.get("RackChain", id) ?? fail("NotFound", `rack chain ${id}`);
}

export function chainsOf(ctx: ReducerContext, rackId: DeviceId, except?: RackChainId): RackChain[] {
  return ctx.tx
    .all("RackChain")
    .filter((c) => c.rack === rackId && c.id !== except)
    .sort(byOrder);
}

export function chainDevices(ctx: ReducerContext, chainId: RackChainId, except?: DeviceId): Device[] {
  return ctx.tx
    .all("Device")
    .filter((d) => d.chain === chainId && d.id !== except)
    .sort(byOrder);
}

function trackChain(ctx: ReducerContext, track: string, except?: DeviceId): Device[] {
  return ctx.tx
    .all("Device")
    .filter((d) => d.track === track && d.pad === null && d.chain == null && d.id !== except)
    .sort(byOrder);
}

function modulatorsOf(ctx: ReducerContext, host: DeviceId): Modulator[] {
  return ctx.tx
    .all("Modulator")
    .filter((m) => m.device === host)
    .sort(byOrder);
}

function checkFit(rackTy: BuiltinDeviceType, category: DeviceCategory): void {
  if (rackTy === "AudioEffectRack" && category !== "AudioEffect") fail("InvalidArgument", "audio effect racks only hold audio effects");
  if (rackTy === "MidiEffectRack" && category !== "NoteEffect") fail("InvalidArgument", "MIDI effect racks only hold MIDI effects");
}

function checkNotRack(ty: BuiltinDeviceType): void {
  if (RACK_TYPES.includes(ty) || ty === "DrumRack") fail("InvalidArgument", "racks cannot be nested in rack chains");
}

function checkZone(what: string, z: Zone): void {
  if (!(Number.isInteger(z.lo) && Number.isInteger(z.hi) && z.lo >= 0 && z.lo <= z.hi && z.hi <= 127)) {
    fail("InvalidArgument", `${what} zone must be lo <= hi <= 127`);
  }
}

function sameSource(a: ModSource, b: ModSource): boolean {
  if (a.type === "Modulator" && b.type === "Modulator") return a.modulator === b.modulator;
  if (a.type === "Macro" && b.type === "Macro") return a.rack === b.rack && a.index === b.index;
  return false;
}

/** The device a source lives in (modulator host or macro rack). */
function sourceHost(ctx: ReducerContext, s: ModSource): DeviceId | null {
  return s.type === "Modulator" ? (ctx.tx.get("Modulator", s.modulator)?.device ?? null) : s.rack;
}

/** Rack of the chain `d` sits on, if any. */
function rackOfDevice(ctx: ReducerContext, d: Device): DeviceId | null {
  return d.chain != null ? (ctx.tx.get("RackChain", d.chain)?.rack ?? null) : null;
}

/** Scope rules of `ether_model::modulation` (model invariants in Rust). */
function checkScope(ctx: ReducerContext, source: ModSource, target: Device, param: number): void {
  if (source.type === "Modulator") {
    const host = sourceHost(ctx, source) ?? fail("NotFound", `modulator ${source.modulator}`);
    if (host !== target.id && rackOfDevice(ctx, target) !== host) {
      fail("InvalidArgument", "a modulator can only target its device or devices inside its rack");
    }
    if (host === target.id && isChainRack(target) && param < SELECTOR_PARAM) fail("InvalidArgument", "modulators cannot target their rack's macros");
  } else {
    if (!(Number.isInteger(source.index) && source.index >= 0 && source.index < MACROS)) fail("InvalidArgument", "macro index must be < 8");
    rack(ctx, source.rack);
    const inRack = rackOfDevice(ctx, target) === source.rack;
    const own = target.id === source.rack && param >= SELECTOR_PARAM;
    if (!inRack && !own) fail("InvalidArgument", "a macro can only target devices inside its rack or the rack's own params");
  }
}

/** Drop mappings targeting `d` whose source is outside `newRack` (moves), with no warning. */
function dropOutOfScope(ctx: ReducerContext, d: Device, newRack: DeviceId | null): void {
  for (const m of ctx.tx.all("ModMapping")) {
    if (m.device !== d.id) continue;
    const host = sourceHost(ctx, m.source);
    if (host !== d.id && host !== newRack) ctx.tx.remove("ModMapping", m.id);
  }
}

function newDevice(ctx: ReducerContext, ty: BuiltinDeviceType, id: DeviceId, track: string, order: string, chainId: RackChainId | undefined): Device {
  const kind = newBuiltinDevice(ty);
  const d: Device = {
    id,
    track,
    order,
    name: builtinDescriptor(kind).name,
    enabled: true,
    kind: { type: "Builtin", device: kind },
    params: defaultParams(kind),
    sidechain: null,
    pad: null,
  };
  if (chainId !== undefined) d.chain = chainId;
  ctx.tx.upsert("Device", d);
  return d;
}

function onChain(d: Device, chainId: RackChainId | null, order: string): Device {
  const next: Device = { ...d, order, sidechain: chainId === null ? d.sidechain : null };
  if (chainId === null) delete next.chain;
  else next.chain = chainId;
  return next;
}

/** Document command, dispatched from `roadmap/index.ts`. */
export function rackCommand(ctx: ReducerContext, c: RackCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "AddChain": {
      if (tx.get("RackChain", c.id)) return;
      const r = rack(ctx, c.rack);
      const siblings = chainsOf(ctx, r.id);
      tx.upsert("RackChain", {
        id: c.id,
        rack: r.id,
        order: keyForInsert(siblings, c.before),
        name: c.name ?? `Chain ${siblings.length + 1}`,
        color: null,
        volume: 0,
        pan: 0,
        mute: false,
        solo: false,
        keys: FULL,
        velocities: FULL,
        select: FULL,
      });
      return;
    }
    case "RemoveChain":
      chain(ctx, c.id);
      deleteChain(ctx, c.id, (d) => deleteChainDevice(ctx, d));
      return;
    case "RenameChain":
      tx.upsert("RackChain", { ...chain(ctx, c.id), name: c.name });
      return;
    case "SetChainColor":
      tx.upsert("RackChain", { ...chain(ctx, c.id), color: c.color });
      return;
    case "MoveChain": {
      const ch = chain(ctx, c.id);
      if (c.before === ch.id) return;
      tx.upsert("RackChain", { ...ch, order: keyForInsert(chainsOf(ctx, ch.rack, ch.id), c.before) });
      return;
    }
    case "SetChainMix": {
      const ch = chain(ctx, c.id);
      const next = { ...ch };
      if (c.volume != null) next.volume = clamp(c.volume, -144, 6);
      if (c.pan != null) next.pan = clamp(c.pan, -1, 1);
      if (c.mute != null) next.mute = c.mute;
      if (c.solo != null) next.solo = c.solo;
      tx.upsert("RackChain", next);
      return;
    }
    case "SetChainZones": {
      const ch = chain(ctx, c.id);
      const next = { ...ch };
      if (c.keys != null) {
        checkZone("key", c.keys);
        next.keys = c.keys;
      }
      if (c.velocities != null) {
        checkZone("velocity", c.velocities);
        next.velocities = c.velocities;
      }
      if (c.select != null) {
        checkZone("selector", c.select);
        next.select = c.select;
      }
      tx.upsert("RackChain", next);
      return;
    }
    case "InsertDevice": {
      if (tx.get("Device", c.id)) return;
      const ch = chain(ctx, c.chain);
      const r = rack(ctx, ch.rack);
      if (c.device.type === "Plugin") fail("Unsupported", "plugins are not available in the mock engine");
      const ty = c.device.device.type;
      checkNotRack(ty);
      checkFit(rackType(r)!, builtinDescriptor(c.device.device).category);
      newDevice(ctx, ty, c.id, r.track, keyForInsert(chainDevices(ctx, ch.id), c.before), ch.id);
      return;
    }
    case "MoveDevice": {
      const d = device(ctx, c.id);
      if (c.before === d.id) return;
      if (d.pad !== null) fail("InvalidArgument", `device ${d.id} is on a drum pad: use DrumRack::MoveDevice`);
      if (c.chain === null) {
        dropOutOfScope(ctx, d, null);
        tx.upsert("Device", onChain(d, null, keyForInsert(trackChain(ctx, d.track, d.id), c.before)));
        return;
      }
      const ch = chain(ctx, c.chain);
      const r = rack(ctx, ch.rack);
      if (r.track !== d.track) fail("InvalidArgument", "a chain device must be on the rack's track");
      if (d.kind.type === "Builtin") {
        checkNotRack(d.kind.device.type);
        checkFit(rackType(r)!, builtinDescriptor(d.kind.device).category);
      }
      if (modulatorsOf(ctx, d.id).length > 0) fail("InvalidArgument", "devices with modulators stay on the track chain (put the modulators on the rack)");
      dropOutOfScope(ctx, d, r.id);
      tx.upsert("Device", onChain(d, ch.id, keyForInsert(chainDevices(ctx, ch.id, d.id), c.before)));
      return;
    }
    case "Group": {
      if (tx.get("Device", c.rack)) return;
      if (!RACK_TYPES.includes(c.rack_type)) fail("InvalidArgument", `${c.rack_type} is not a rack type`);
      if (c.devices.length === 0) fail("InvalidArgument", "select at least one device to group");
      const devices = c.devices.map((id) => device(ctx, id));
      const track = devices[0]!.track;
      if (devices.some((d) => d.track !== track || d.pad !== null || d.chain != null)) fail("InvalidArgument", "grouped devices must be on one track chain");
      const order = trackChain(ctx, track).map((d) => d.id);
      const idx = [...new Set(devices.map((d) => order.indexOf(d.id)))].sort((a, b) => a - b);
      if (idx.length !== devices.length || idx.some((v, i) => i > 0 && v !== idx[i - 1]! + 1)) {
        fail("InvalidArgument", "grouped devices must be consecutive and distinct");
      }
      const sorted = idx.map((i) => device(ctx, order[i]!));
      for (const d of sorted) {
        if (d.kind.type === "Builtin") {
          checkNotRack(d.kind.device.type);
          checkFit(c.rack_type, builtinDescriptor(d.kind.device).category);
        }
      }
      const t = tx.get("Track", track)!;
      if (builtinDescriptor(c.rack_type).category === "Instrument" && t.kind !== "Midi") fail("InvalidArgument", "instrument racks can only be on MIDI tracks");
      newDevice(ctx, c.rack_type, c.rack, track, sorted[0]!.order, undefined);
      tx.upsert("RackChain", {
        id: c.chain,
        rack: c.rack,
        order: keyForInsert([], null),
        name: sorted[0]!.name,
        color: null,
        volume: 0,
        pan: 0,
        mute: false,
        solo: false,
        keys: FULL,
        velocities: FULL,
        select: FULL,
      });
      // Modulators move to the rack (new ids), their mappings follow.
      const grouped = new Set(sorted.map((d) => d.id));
      const mods = tx.all("Modulator").filter((m) => grouped.has(m.device));
      const newIds = new Map<string, string>();
      let last: string | null = null;
      for (const m of mods) {
        const id = ctx.newId();
        newIds.set(m.id, id);
        const order: string = keyForInsert(last === null ? [] : [{ id: "_", order: last }], null);
        last = order;
        tx.remove("Modulator", m.id);
        tx.upsert("Modulator", { ...m, id, device: c.rack, order });
      }
      for (const m of tx.all("ModMapping")) {
        if (m.source.type !== "Modulator" || !newIds.has(m.source.modulator)) continue;
        tx.remove("ModMapping", m.id);
        tx.upsert("ModMapping", { ...m, id: ctx.newId(), source: { type: "Modulator", modulator: newIds.get(m.source.modulator)! } });
      }
      for (const d of sorted) {
        tx.upsert("Device", onChain(tx.get("Device", d.id)!, c.chain, d.order));
      }
      return;
    }
  }
}

/** Document command (all but `ListModulatorKinds`), dispatched from `roadmap/index.ts`. */
export function modulationCommand(ctx: ReducerContext, c: ModulationCommand): void {
  const { tx } = ctx;
  const modulator = (id: string) => tx.get("Modulator", id) ?? fail("NotFound", `modulator ${id}`);
  const mapping = (id: string) => tx.get("ModMapping", id) ?? fail("NotFound", `modulation mapping ${id}`);
  const paramInfo = (m: Modulator, param: number) =>
    MODULATOR_DESCRIPTORS.find((k) => k.kind === m.kind)?.params.find((p) => p.id === param) ?? fail("NotFound", `modulator param ${param}`);
  switch (c.type) {
    case "AddModulator": {
      if (tx.get("Modulator", c.id)) return;
      const d = device(ctx, c.device);
      if (d.pad !== null || d.chain != null) fail("InvalidArgument", "modulators live on track-chain devices (put them on the rack)");
      const desc = MODULATOR_DESCRIPTORS.find((k) => k.kind === c.kind) ?? fail("InvalidArgument", `unknown modulator kind ${c.kind}`);
      const siblings = modulatorsOf(ctx, d.id);
      const same = siblings.filter((m) => m.kind === c.kind).length;
      tx.upsert("Modulator", {
        id: c.id,
        device: d.id,
        order: keyForInsert(siblings, null),
        name: c.name ?? (same === 0 ? desc.name : `${desc.name} ${same + 1}`),
        kind: c.kind,
        params: Object.fromEntries(desc.params.map((p) => [p.id, p.default])),
      });
      return;
    }
    case "RemoveModulator":
      modulator(c.id);
      for (const m of tx.all("ModMapping")) if (m.source.type === "Modulator" && m.source.modulator === c.id) tx.remove("ModMapping", m.id);
      tx.remove("Modulator", c.id);
      return;
    case "RenameModulator":
      tx.upsert("Modulator", { ...modulator(c.id), name: c.name });
      return;
    case "SetModulatorParam": {
      const m = modulator(c.modulator);
      if (!Number.isFinite(c.value)) fail("InvalidArgument", "param values must be finite");
      const info = paramInfo(m, c.param);
      let v = clamp(c.value, Math.min(info.min, info.max), Math.max(info.min, info.max));
      if (info.step != null && info.step > 0) v = info.min + Math.round((v - info.min) / info.step) * info.step;
      tx.upsert("Modulator", { ...m, params: { ...m.params, [c.param]: v } });
      return;
    }
    case "ResetModulatorParam": {
      const m = modulator(c.modulator);
      const info = paramInfo(m, c.param);
      tx.upsert("Modulator", { ...m, params: { ...m.params, [c.param]: info.default } });
      return;
    }
    case "Map": {
      if (tx.get("ModMapping", c.id)) return;
      if (!Number.isFinite(c.depth)) fail("InvalidArgument", "depth must be finite");
      const target = device(ctx, c.device);
      if (target.kind.type === "Builtin") {
        const info = builtinDescriptor(target.kind.device).params.find((p) => p.id === c.param) ?? fail("NotFound", `param ${c.param}`);
        if (!info.automatable) fail("InvalidArgument", `"${info.name}" cannot be modulated`);
      }
      if (c.source.type === "Modulator") modulator(c.source.modulator);
      checkScope(ctx, c.source, target, c.param);
      if (tx.all("ModMapping").some((m) => sameSource(m.source, c.source) && m.device === c.device && m.param === c.param)) {
        fail("InvalidArgument", "this source already modulates that parameter");
      }
      tx.upsert("ModMapping", { id: c.id, source: c.source, device: c.device, param: c.param, depth: clamp(c.depth, -1, 1) });
      return;
    }
    case "SetDepth": {
      const m = mapping(c.id);
      if (!Number.isFinite(c.depth)) fail("InvalidArgument", "depth must be finite");
      tx.upsert("ModMapping", { ...m, depth: clamp(c.depth, -1, 1) });
      return;
    }
    case "Unmap":
      mapping(c.id);
      tx.remove("ModMapping", c.id);
      return;
    case "SetSidechain": {
      const m = modulator(c.modulator);
      if (m.kind !== "EnvelopeFollower") fail("InvalidArgument", "only envelope followers take a sidechain");
      const next: Modulator = { ...m };
      if (c.source === null) delete next.sidechain;
      else {
        const t = tx.get("Track", c.source) ?? fail("NotFound", `track ${c.source}`);
        if (t.kind === "Master") fail("InvalidArgument", "the master track cannot be a sidechain source");
        if (device(ctx, m.device).track === c.source) fail("InvalidArgument", "a modulator cannot follow its own track");
        next.sidechain = c.source;
      }
      tx.upsert("Modulator", next);
      return;
    }
    case "ListModulatorKinds":
      fail("Unsupported", "not a document command");
  }
}

/** `Modulation::ListModulatorKinds` (runtime), dispatched from `MockTransport.execute`. */
export function listModulatorKinds(): ReplyValue {
  return { type: "ModulatorKinds", kinds: [...MODULATOR_DESCRIPTORS] };
}

// ─── Cascades used by the core reducer ─────────────────────────────────────────────────

/** Delete a chain device with what points at it (lanes, MIDI and modulation mappings). */
function deleteChainDevice(ctx: ReducerContext, id: DeviceId): void {
  const { tx } = ctx;
  for (const l of tx.all("AutomationLane")) {
    if (l.target.type !== "DeviceParam" || l.target.device !== id) continue;
    for (const p of tx.all("AutomationPoint")) if (p.lane === l.id) tx.remove("AutomationPoint", p.id);
    tx.remove("AutomationLane", l.id);
  }
  onDeviceDeleted(ctx, id);
  for (const m of tx.all("ModMapping")) if (m.device === id) tx.remove("ModMapping", m.id);
  tx.remove("Device", id);
}

function deleteChain(ctx: ReducerContext, id: RackChainId, del: (id: DeviceId) => void): void {
  for (const d of chainDevices(ctx, id)) del(d.id);
  ctx.tx.remove("RackChain", id);
}

/**
 * A device is being deleted (before it is removed): mappings targeting it or sourced from
 * it (its modulators, its macros), its modulators, and a rack's chains with their devices
 * (through `deleteDevice`, the core cascade).
 */
export function onDeviceDeletedRacks(ctx: ReducerContext, id: DeviceId, deleteDevice: (id: DeviceId) => void): void {
  const { tx } = ctx;
  for (const m of tx.all("ModMapping")) {
    const fromIt = m.source.type === "Macro" ? m.source.rack === id : tx.get("Modulator", m.source.modulator)?.device === id;
    if (m.device === id || fromIt) tx.remove("ModMapping", m.id);
  }
  for (const m of tx.all("Modulator")) if (m.device === id) tx.remove("Modulator", m.id);
  for (const c of chainsOf(ctx, id)) deleteChain(ctx, c.id, deleteDevice);
}

/** Copy the modulators of the devices in `ids` (old → new) and the mappings among them. */
function copyModulation(ctx: ReducerContext, ids: Map<DeviceId, DeviceId>): void {
  const { tx } = ctx;
  const mods = new Map<string, string>();
  for (const m of tx.all("Modulator")) {
    const host = ids.get(m.device);
    if (host === undefined) continue;
    const id = ctx.newId();
    mods.set(m.id, id);
    tx.upsert("Modulator", { ...m, id, device: host });
  }
  for (const m of tx.all("ModMapping")) {
    const target = ids.get(m.device);
    if (target === undefined) continue;
    const source: ModSource | null =
      m.source.type === "Modulator"
        ? mods.has(m.source.modulator)
          ? { type: "Modulator", modulator: mods.get(m.source.modulator)! }
          : null
        : ids.has(m.source.rack)
          ? { type: "Macro", rack: ids.get(m.source.rack)!, index: m.source.index }
          : null;
    if (source) tx.upsert("ModMapping", { ...m, id: ctx.newId(), source, device: target });
  }
}

function copyChains(ctx: ReducerContext, src: DeviceId, dst: DeviceId, ids: Map<DeviceId, DeviceId>): void {
  const track = device(ctx, dst).track;
  for (const c of chainsOf(ctx, src)) {
    const chainId = ctx.newId();
    ctx.tx.upsert("RackChain", { ...c, id: chainId, rack: dst });
    for (const d of chainDevices(ctx, c.id)) {
      const id = ctx.newId();
      ids.set(d.id, id);
      ctx.tx.upsert("Device", { ...d, id, track, chain: chainId });
    }
  }
}

/** `Device::Duplicate`: a rack's chains with their devices, modulators and mappings. */
export function copyRackExtras(ctx: ReducerContext, src: DeviceId, dst: DeviceId): void {
  const ids = new Map<DeviceId, DeviceId>([[src, dst]]);
  copyChains(ctx, src, dst, ids);
  copyModulation(ctx, ids);
}

/** `Track::Duplicate`: `ids` maps the copied track devices (extended with chain devices). */
export function copyTrackRackExtras(ctx: ReducerContext, ids: Map<DeviceId, DeviceId>): void {
  for (const [src, dst] of [...ids]) {
    const d = ctx.tx.get("Device", src);
    if (d && isChainRack(d)) copyChains(ctx, src, dst, ids);
  }
  copyModulation(ctx, ids);
}
