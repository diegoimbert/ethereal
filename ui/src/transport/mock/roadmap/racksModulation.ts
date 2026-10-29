/**
 * Mock of `Rack::*` and `Modulation::*` (v0.2).
 * Rack chains, chain devices, macros (16: rack params 0..8 then 9..17; a drum rack's 2..18) and Bitwig-style modulators with
 * mappings (`ether_model::{rack, modulation}`), mirroring
 * `crates/ether-controller/src/racks/mod.rs`: chain content rules (audio effect racks hold
 * audio effects, MIDI effect racks MIDI effects, instrument racks anything), nesting (racks
 * in rack chains up to `MAX_RACK_DEPTH` levels counting drum racks, never inside themselves;
 * a drum rack on track chains or instrument rack chains, never under a drum pad; fresh audio
 * / MIDI effect racks start with "chain 1"), mapping scope (a modulator reaches its device and, on a rack, the devices
 * inside it at any depth (chains and pads) and its selector; a macro the devices inside its
 * rack (or drum rack) at any depth and the rack's non-macro params), one mapping per (source, target),
 * `Group` (modulators re-hosted on the rack) and the cascades the core reducer calls
 * (`onDeviceDeletedRacks`, `copyTrackRackExtras`). Racks may sit on drum pads.
 * `ListModulatorKinds` answers from the generated `devices/modulators.json`.
 */

import type {
  BuiltinDevice,
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
  RackMacros,
  ReplyValue,
  Zone,
} from "@/generated";
import { keyForInsert } from "@/state/orderKey";
import { builtinDescriptor, newBuiltinDevice } from "../builtinDevices";
import { defaultParams } from "../demoProject";
import { MODULATOR_DESCRIPTORS } from "../devices";
import { deriveId } from "../../ids";
import { fail, type ReducerContext } from "../documentReducer";
import { copyRackPads, onRackDeleted } from "./drumRack";
import {
  CHAIN_RACK_TYPES,
  MAX_RACK_DEPTH,
  chainDevicesOf,
  checkChainOrder,
  clamp,
  defaultDeviceName,
  devicePath,
  hasNestedDrumRack,
  isAnyRack,
  isChainRack,
  isDrumRack,
  isUnderDrumPad,
  modulatorsOf,
  nestedRackHeight,
  onDeviceDeleted,
  onSidechainChainDeleted,
  rackChainsOf,
  targetDevice,
  trackChainOf,
} from "./shared";

/** `ether_model::DEVICE_ENABLED_PARAM`: a mapping to the device's on/off switch. */
const DEVICE_ENABLED_PARAM = 0xffff_ffff;
const MACROS = 16;
/** A drum rack's macro 1 (after its volume and pan; `ether_model::DRUM_RACK_FIRST_MACRO`). */
const DRUM_RACK_FIRST_MACRO = 2;
/** `ether_model::DEFAULT_VISIBLE_MACROS`. */
const DEFAULT_VISIBLE_MACROS = 8;

/** Param id of `d`'s macro `index` (`ether_model::rack::macro_param`), or `null`. */
function macroParam(d: Device, index: number): number | null {
  if (index < 0 || index >= MACROS) return null;
  if (isDrumRack(d)) return DRUM_RACK_FIRST_MACRO + index;
  if (!isChainRack(d)) return null;
  return index < 8 ? index : 9 + (index - 8);
}

/** Macro index of `d`'s param `param` (`ether_model::rack::macro_index`), or `null`. */
function macroIndex(d: Device, param: number): number | null {
  for (let i = 0; i < MACROS; i++) if (macroParam(d, i) === param) return i;
  return null;
}

/**
 * `Device::RenameMacro` / `Device::SetMacroCount` (`ether_model::RackMacros` on any rack):
 * names trimmed and cut to 64 chars (empty = the default), 1..=16 shown, the defaults stored
 * as no `macros`.
 */
export function setRackMacros(ctx: ReducerContext, id: DeviceId, edit: (m: RackMacros) => void): void {
  const d = device(ctx, id);
  if (d.kind.type !== "Builtin" || !isAnyRack(d)) fail("InvalidArgument", `device ${id} is not a rack`);
  const b = d.kind.device as Extract<BuiltinDevice, { type: "InstrumentRack" }>;
  const m: RackMacros = { visible: b.macros?.visible ?? DEFAULT_VISIBLE_MACROS, names: [...(b.macros?.names ?? [])] };
  edit(m);
  m.visible = Math.min(MACROS, Math.max(1, Math.round(m.visible)));
  m.names = m.names.slice(0, MACROS).map((n) => [...(n ?? "").trim()].slice(0, 64).join(""));
  while (m.names.length > 0 && m.names[m.names.length - 1] === "") m.names.pop();
  const macros = m.visible === DEFAULT_VISIBLE_MACROS && m.names.length === 0 ? undefined : m;
  if (JSON.stringify(macros ?? null) === JSON.stringify(b.macros ?? null)) return;
  const { macros: _old, ...rest } = b;
  const next = (macros ? { ...rest, macros } : rest) as BuiltinDevice;
  ctx.tx.upsert("Device", { ...d, kind: { type: "Builtin", device: next } });
}

export function renameMacro(ctx: ReducerContext, id: DeviceId, index: number, name: string): void {
  if (!(Number.isInteger(index) && index >= 0 && index < MACROS)) fail("InvalidArgument", `macro index must be < ${MACROS}`);
  setRackMacros(ctx, id, (m) => {
    while (m.names.length <= index) m.names.push("");
    m.names[index] = name;
  });
}
const FULL: Zone = { lo: 0, hi: 127 };
export { MAX_RACK_DEPTH };

function rackType(d: Device): BuiltinDeviceType | null {
  return d.kind.type === "Builtin" && CHAIN_RACK_TYPES.includes(d.kind.device.type) ? d.kind.device.type : null;
}

function device(ctx: ReducerContext, id: DeviceId): Device {
  return ctx.tx.get("Device", id) ?? fail("NotFound", `device ${id}`);
}

function rack(ctx: ReducerContext, id: DeviceId): Device {
  const d = device(ctx, id);
  if (!isChainRack(d)) fail("InvalidArgument", `device ${id} is not a rack`);
  return d;
}

function chain(ctx: ReducerContext, id: RackChainId): RackChain {
  return ctx.tx.get("RackChain", id) ?? fail("NotFound", `rack chain ${id}`);
}

function checkFit(rackTy: BuiltinDeviceType, category: DeviceCategory): void {
  if (rackTy === "AudioEffectRack" && category !== "AudioEffect") fail("InvalidArgument", "audio effect racks only hold audio effects");
  if (rackTy === "MidiEffectRack" && category !== "NoteEffect") fail("InvalidArgument", "MIDI effect racks only hold MIDI effects");
}

/** The racks `id` sits inside (rack chains and drum pads), innermost first (`null`: cyclic
 *  or too deep). */
function enclosingRacks(ctx: ReducerContext, id: DeviceId): DeviceId[] | null {
  const out: DeviceId[] = [];
  let d = ctx.tx.get("Device", id);
  while (d && (d.chain != null || d.pad !== null)) {
    const r = d.chain != null ? ctx.tx.get("RackChain", d.chain)?.rack : ctx.tx.get("DrumPad", d.pad!)?.rack;
    if (r === undefined || r === id || out.includes(r) || out.length > MAX_RACK_DEPTH) return null;
    out.push(r);
    d = ctx.tx.get("Device", r);
  }
  return out;
}

/** Whether `id` sits inside `rackId`, at any depth. */
function isInside(ctx: ReducerContext, id: DeviceId, rackId: DeviceId): boolean {
  return enclosingRacks(ctx, id)?.includes(rackId) ?? false;
}

/**
 * Whether a device of type `ty` (`id`: an existing one being moved) may go onto a chain of
 * rack `host`: drum racks only on instrument rack chains and never under a drum pad, never
 * inside itself, at most `MAX_RACK_DEPTH` levels (every rack counts, drum racks included).
 */
function checkNestable(ctx: ReducerContext, ty: BuiltinDeviceType, id: DeviceId | null, host: DeviceId): void {
  const p = ctx.tx.project;
  const drums = ty === "DrumRack" || (id !== null && hasNestedDrumRack(p, id));
  if (ty === "DrumRack" && rackType(device(ctx, host)) !== "InstrumentRack") fail("InvalidArgument", "drum racks only go on instrument rack chains");
  if (drums && isUnderDrumPad(p, host)) fail("InvalidArgument", "drum racks cannot be nested in drum pads");
  if (!CHAIN_RACK_TYPES.includes(ty) && ty !== "DrumRack") return;
  const path = devicePath(p, host);
  if (id !== null && (path === null || path.includes(id))) fail("InvalidArgument", "a rack cannot go inside itself");
  const above = path?.length ?? MAX_RACK_DEPTH;
  const below = id === null ? 0 : nestedRackHeight(p, id);
  if (above + 1 + below > MAX_RACK_DEPTH) fail("InvalidArgument", `racks nest at most ${MAX_RACK_DEPTH} deep`);
}

/**
 * `racks::add_first_chain`: a freshly inserted audio / MIDI effect rack starts with one
 * empty chain, "chain 1" (id `deriveId(rack, 0)`), in the same undo step (an empty rack lets
 * nothing through). Instrument racks start empty (their chains come with an instrument).
 */
export function addFirstChain(ctx: ReducerContext, rack: DeviceId, ty: BuiltinDeviceType): void {
  if (ty !== "AudioEffectRack" && ty !== "MidiEffectRack") return;
  // (A test's non-ULID rack id gets a fresh one.)
  let id: RackChainId;
  try {
    id = deriveId(rack, 0);
  } catch {
    id = ctx.newId();
  }
  if (ctx.tx.get("RackChain", id)) return;
  ctx.tx.upsert("RackChain", {
    id,
    rack,
    order: keyForInsert([], null),
    name: "chain 1",
    color: null,
    volume: 0,
    pan: 0,
    mute: false,
    solo: false,
    keys: FULL,
    velocities: FULL,
    select: FULL,
  });
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

/** Whether `d` sits inside `rackId` at any depth (chains and pads, nested racks included). */
function inRack(ctx: ReducerContext, d: Device, rackId: DeviceId): boolean {
  return isInside(ctx, d.id, rackId);
}

/** Scope rules of `ether_model::modulation` (model invariants in Rust). */
function checkScope(ctx: ReducerContext, source: ModSource, target: Device, param: number): void {
  if (source.type === "Modulator") {
    const host = sourceHost(ctx, source) ?? fail("NotFound", `modulator ${source.modulator}`);
    if (host !== target.id && !inRack(ctx, target, host)) {
      fail("InvalidArgument", "a modulator can only target its device or devices inside its rack");
    }
    if (host === target.id && macroIndex(target, param) !== null) fail("InvalidArgument", "modulators cannot target their rack's macros");
  } else {
    if (!(Number.isInteger(source.index) && source.index >= 0 && source.index < MACROS)) fail("InvalidArgument", `macro index must be < ${MACROS}`);
    if (!isAnyRack(device(ctx, source.rack))) fail("InvalidArgument", `device ${source.rack} is not a rack`);
    const inside = inRack(ctx, target, source.rack);
    // Never the rack's own macros nor its switch.
    const own = target.id === source.rack && macroIndex(target, param) === null && param !== DEVICE_ENABLED_PARAM;
    if (!inside && !own) fail("InvalidArgument", "a macro can only target devices inside its rack or the rack's own params");
  }
}

/**
 * Before moving `d` onto a chain of `newRack` (`null`: the track chain): drop the mappings
 * its new place puts out of scope, for `d` and every device inside it, with no warning.
 */
function dropOutOfScope(ctx: ReducerContext, d: Device, newRack: DeviceId | null): void {
  const outside = newRack === null ? [] : [newRack, ...(enclosingRacks(ctx, newRack) ?? [])];
  for (const m of ctx.tx.all("ModMapping")) {
    if (m.device !== d.id && !isInside(ctx, m.device, d.id)) continue;
    const host = sourceHost(ctx, m.source);
    if (host === null) {
      ctx.tx.remove("ModMapping", m.id);
      continue;
    }
    // Racks between the target and `d` (they move together), and `d` itself.
    const between: DeviceId[] = [];
    if (m.device !== d.id) {
      for (const r of enclosingRacks(ctx, m.device) ?? []) {
        between.push(r);
        if (r === d.id) break;
      }
    }
    if (host !== m.device && !between.includes(host) && !outside.includes(host)) ctx.tx.remove("ModMapping", m.id);
  }
}

function newDevice(ctx: ReducerContext, ty: BuiltinDeviceType, id: DeviceId, track: string, order: string, chainId: RackChainId | undefined): Device {
  const kind = newBuiltinDevice(ty);
  const d: Device = {
    id,
    track,
    order,
    name: defaultDeviceName(kind),
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
      const siblings = rackChainsOf(ctx.tx.project, r.id);
      tx.upsert("RackChain", {
        id: c.id,
        rack: r.id,
        order: keyForInsert(siblings, c.before),
        name: c.name ?? `chain ${siblings.length + 1}`,
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
      tx.upsert("RackChain", { ...ch, order: keyForInsert(rackChainsOf(ctx.tx.project, ch.rack, ch.id), c.before) });
      return;
    }
    case "CopyChain":
      copyChainCommand(ctx, c);
      return;
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
      checkNestable(ctx, ty, null, r.id);
      checkFit(rackType(r)!, builtinDescriptor(c.device.device).category);
      newDevice(ctx, ty, c.id, r.track, keyForInsert(chainDevicesOf(ctx.tx.project, ch.id), c.before), ch.id);
      addFirstChain(ctx, c.id, ty);
      return;
    }
    case "MoveDevice": {
      const d = device(ctx, c.id);
      if (c.before === d.id) return;
      if (d.pad !== null) fail("InvalidArgument", `device ${d.id} is on a drum pad: use DrumRack::MoveDevice`);
      if (c.chain === null) {
        dropOutOfScope(ctx, d, null);
        tx.upsert("Device", onChain(d, null, keyForInsert(trackChainOf(ctx.tx.project, d.track, d.id), c.before)));
        checkChainOrder(ctx.tx.project, d.track);
        return;
      }
      const ch = chain(ctx, c.chain);
      const r = rack(ctx, ch.rack);
      if (r.track !== d.track) fail("InvalidArgument", "a chain device must be on the rack's track");
      if (d.kind.type === "Builtin") {
        checkNestable(ctx, d.kind.device.type, d.id, r.id);
        checkFit(rackType(r)!, builtinDescriptor(d.kind.device).category);
      }
      if (modulatorsOf(ctx.tx.project, d.id).length > 0)
        fail("InvalidArgument", "devices with modulators stay on the track chain (put the modulators on the rack)");
      dropOutOfScope(ctx, d, r.id);
      tx.upsert("Device", onChain(d, ch.id, keyForInsert(chainDevicesOf(ctx.tx.project, ch.id, d.id), c.before)));
      return;
    }
    case "Group": {
      if (tx.get("Device", c.rack)) return;
      if (!CHAIN_RACK_TYPES.includes(c.rack_type)) fail("InvalidArgument", `${c.rack_type} is not a rack type`);
      if (c.devices.length === 0) fail("InvalidArgument", "select at least one device to group");
      const devices = c.devices.map((id) => device(ctx, id));
      const track = devices[0]!.track;
      if (devices.some((d) => d.track !== track || d.pad !== null || d.chain != null)) fail("InvalidArgument", "grouped devices must be on one track chain");
      const order = trackChainOf(ctx.tx.project, track).map((d) => d.id);
      const idx = [...new Set(devices.map((d) => order.indexOf(d.id)))].sort((a, b) => a - b);
      if (idx.length !== devices.length || idx.some((v, i) => i > 0 && v !== idx[i - 1]! + 1)) {
        fail("InvalidArgument", "grouped devices must be consecutive and distinct");
      }
      const sorted = idx.map((i) => device(ctx, order[i]!));
      for (const d of sorted) {
        if (d.kind.type === "Builtin") {
          // The new rack is level 1: grouped racks go one level down with what they hold. A
          // drum rack groups into an instrument rack only.
          if (d.kind.device.type === "DrumRack" && c.rack_type !== "InstrumentRack") fail("InvalidArgument", "drum racks only go on instrument rack chains");
          if (isAnyRack(d) && 2 + nestedRackHeight(ctx.tx.project, d.id) > MAX_RACK_DEPTH) fail("InvalidArgument", `racks nest at most ${MAX_RACK_DEPTH} deep`);
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
    case "Ungroup": {
      const r = rack(ctx, c.rack);
      const p = () => ctx.tx.project;
      // The leaves in signal order (nested chain racks flattened), with whether they play.
      const leaves: Array<{ d: Device; on: boolean }> = [];
      const walk = (id: DeviceId, on: boolean) => {
        for (const ch of rackChainsOf(p(), id))
          for (const d of chainDevicesOf(p(), ch.id)) {
            const live = on && !ch.mute && d.enabled;
            if (isChainRack(d)) walk(d.id, live);
            else leaves.push({ d, on: live });
          }
      };
      walk(r.id, r.enabled);
      const section = (d: Device) => {
        const cat = d.kind.type === "Builtin" ? builtinDescriptor(d.kind.device).category : "AudioEffect";
        return cat === "NoteEffect" ? 0 : cat === "Instrument" ? 1 : 2;
      };
      const sorted = leaves.map((l, i) => ({ ...l, i })).sort((a, b) => section(a.d) - section(b.d) || a.i - b.i);
      const host = r.chain != null ? chainDevicesOf(p(), r.chain, r.id) : trackChainOf(p(), r.track, r.id);
      const instruments = sorted.filter((l) => section(l.d) === 1).length;
      if (instruments > 0 && instruments + host.filter((d) => section(d) === 1).length > 1) {
        fail("InvalidArgument", "a rack with several instruments can't be flattened into one chain");
      }
      for (const { d, on } of sorted) {
        const siblings = r.chain != null ? chainDevicesOf(p(), r.chain, d.id) : trackChainOf(p(), r.track, d.id);
        dropOutOfScope(ctx, d, r.chain != null ? chain(ctx, r.chain).rack : null);
        const moved = onChain(tx.get("Device", d.id)!, r.chain ?? null, keyForInsert(siblings, r.id));
        tx.upsert("Device", on ? moved : { ...moved, enabled: false });
      }
      deleteChainDevice(ctx, r.id);
      checkChainOrder(p(), r.track);
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
      const siblings = modulatorsOf(ctx.tx.project, d.id);
      const same = siblings.filter((m) => m.kind === c.kind).length;
      tx.upsert("Modulator", {
        id: c.id,
        device: d.id,
        order: keyForInsert(siblings, null),
        name: c.name ?? (same === 0 ? desc.name.toLowerCase() : `${desc.name.toLowerCase()} ${same + 1}`),
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
      if (target.kind.type === "Builtin" && c.param !== DEVICE_ENABLED_PARAM) {
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

/**
 * Delete a chain device with what points at it (lanes, MIDI and modulation mappings) and,
 * for a nested rack, its own chains, modulators and the mappings from it.
 */
function deleteChainDevice(ctx: ReducerContext, id: DeviceId): void {
  const { tx } = ctx;
  // A nested drum rack takes its pads (and their chains) with it.
  onRackDeleted(ctx, id, (x) => deleteChainDevice(ctx, x));
  onDeviceDeletedRacks(ctx, id, (x) => deleteChainDevice(ctx, x));
  for (const l of tx.all("AutomationLane")) {
    if (targetDevice(l.target) !== id) continue;
    for (const p of tx.all("AutomationPoint")) if (p.lane === l.id) tx.remove("AutomationPoint", p.id);
    tx.remove("AutomationLane", l.id);
  }
  onDeviceDeleted(ctx, id);
  for (const m of tx.all("ModMapping")) if (m.device === id) tx.remove("ModMapping", m.id);
  tx.remove("Device", id);
}

function deleteChain(ctx: ReducerContext, id: RackChainId, del: (id: DeviceId) => void): void {
  onSidechainChainDeleted(ctx, { type: "Rack", id });
  for (const d of chainDevicesOf(ctx.tx.project, id)) del(d.id);
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
  for (const c of rackChainsOf(ctx.tx.project, id)) deleteChain(ctx, c.id, deleteDevice);
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
  for (const c of rackChainsOf(ctx.tx.project, src)) copyChain(ctx, c, ctx.newId(), dst, c.order, ids);
}

/** `racks::copy_chain`: chain `c` as `chainId` onto rack `dst` (devices, nested racks and pads). */
function copyChain(ctx: ReducerContext, c: RackChain, chainId: RackChainId, dst: DeviceId, order: string, ids: Map<DeviceId, DeviceId>): void {
  const track = device(ctx, dst).track;
  ctx.tx.upsert("RackChain", { ...c, id: chainId, rack: dst, order });
  for (const d of chainDevicesOf(ctx.tx.project, c.id)) {
    const id = ctx.newId();
    ids.set(d.id, id);
    // A sidechain that can't hold at the new place is cut (`racks::copy_chain`).
    const cutSidechain = d.sidechain === track && (d.track !== track || d.sidechain_tap?.chain != null);
    const copy: Device = { ...d, id, track, chain: chainId };
    if (cutSidechain) {
      copy.sidechain = null;
      delete copy.sidechain_tap;
    }
    ctx.tx.upsert("Device", copy);
    // A nested rack: its chains too (depth bounded by the model).
    if (isChainRack(d)) copyChains(ctx, d.id, id, ids);
    // A nested drum rack: its pads with their chains, and the chains of the racks on them.
    if (isDrumRack(d)) {
      const pads = new Map<DeviceId, DeviceId>();
      copyRackPads(ctx, d.id, id, track, pads);
      for (const [from, to] of pads) {
        ids.set(from, to);
        const x = ctx.tx.get("Device", from);
        if (x && isChainRack(x)) copyChains(ctx, from, to, ids);
      }
    }
  }
}

/**
 * `Rack::CopyChain` (`racks::copy_chain_command`): copy a chain onto any rack, `cut` then
 * removes the source. Mappings from outside the chain stay when their source surrounds the
 * new place; a cut on the same track carries its devices' automation lanes.
 */
function copyChainCommand(ctx: ReducerContext, c: Extract<RackCommand, { type: "CopyChain" }>): void {
  const { tx } = ctx;
  if (tx.get("RackChain", c.new_id)) return;
  const src = chain(ctx, c.id);
  const r = rack(ctx, c.rack);
  for (const d of chainDevicesOf(tx.project, src.id)) {
    if (d.kind.type !== "Builtin") continue;
    checkNestable(ctx, d.kind.device.type, d.id, r.id);
    checkFit(rackType(r)!, builtinDescriptor(d.kind.device).category);
  }
  if (c.before !== null && tx.get("RackChain", c.before)?.rack !== r.id) fail("InvalidArgument", "`before` must be a chain of the target rack");
  const ids = new Map<DeviceId, DeviceId>();
  copyChain(ctx, src, c.new_id, r.id, keyForInsert(rackChainsOf(tx.project, r.id), c.before), ids);
  copyModulation(ctx, ids);
  const around = [r.id, ...(enclosingRacks(ctx, r.id) ?? [])];
  for (const m of tx.all("ModMapping")) {
    const target = ids.get(m.device);
    if (target === undefined) continue;
    const host = sourceHost(ctx, m.source);
    if (host === null || ids.has(host) || !around.includes(host)) continue;
    tx.upsert("ModMapping", { ...m, id: ctx.newId(), device: target });
  }
  if (!c.cut) return;
  if (device(ctx, src.rack).track === r.track) {
    for (const l of tx.all("AutomationLane")) {
      const old = targetDevice(l.target);
      const to = old === null ? undefined : ids.get(old);
      if (to === undefined) continue;
      const lane = ctx.newId();
      const target = l.target.type === "DeviceParam" ? { ...l.target, device: to } : l.target.type === "DeviceEnabled" ? { ...l.target, device: to } : l.target;
      tx.upsert("AutomationLane", { ...l, id: lane, target });
      for (const pt of tx.all("AutomationPoint")) if (pt.lane === l.id) tx.upsert("AutomationPoint", { ...pt, id: ctx.newId(), lane });
    }
  }
  deleteChain(ctx, src.id, (d) => deleteChainDevice(ctx, d));
}

/** `Track::Duplicate` / `Device::Duplicate`: `ids` maps the copied devices (a track's, or a
 *  device with its drum pads' devices), extended with the copied racks' chain devices. */
export function copyTrackRackExtras(ctx: ReducerContext, ids: Map<DeviceId, DeviceId>): void {
  for (const [src, dst] of [...ids]) {
    const d = ctx.tx.get("Device", src);
    if (d && isChainRack(d)) copyChains(ctx, src, dst, ids);
  }
  copyModulation(ctx, ids);
}
