/**
 * Mock of `DrumRack::*` (pads, pad chains) and `Slice::*` (sampler slices), plus the drum
 * rack structure rules the core reducer calls for `Device::{Move, Duplicate, Remove}` and
 * `Track::{Duplicate, Delete}` (mirrors `ether-controller/src/doc/{devices,mod,tracks}.rs`).
 * Owned by `drum-rack`.
 */

import type { AutoSlice, Device, DrumPad, DrumRackCommand, SliceCommand, SlicePadIds, SliceSettings, TrackId } from "@/generated";
import { BEATS_EPSILON } from "@/state/beats";
import { keyForInsert } from "@/state/orderKey";
import { newBuiltinDevice } from "../builtinDevices";
import { defaultParams } from "../demoProject";
import { fail, type ReducerContext } from "../documentReducer";
import { addFirstChain } from "./racksModulation";
import {
  CHAIN_RACK_TYPES,
  MAX_RACK_DEPTH,
  byOrder,
  clamp,
  defaultDeviceName,
  devicePath,
  hasNestedDrumRack,
  isChainRack,
  isUnderDrumPad,
  nestedRackHeight,
  onDeviceDeleted,
  onSidechainChainDeleted,
  padChainOf,
  targetDevice,
} from "./shared";

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/** MIDI note name with C3 = 60 (Ableton convention). */
export function noteName(note: number): string {
  return `${NOTE_NAMES[note % 12]}${Math.floor(note / 12) - 2}`;
}

function isRack(d: Device): boolean {
  return d.kind.type === "Builtin" && d.kind.device.type === "DrumRack";
}

function rack(ctx: ReducerContext, id: string): Device {
  const d = ctx.tx.get("Device", id) ?? fail("NotFound", `device ${id}`);
  if (!isRack(d) || d.pad !== null) fail("InvalidArgument", `device ${id} is not a drum rack`);
  return d;
}

/**
 * A chain rack (`id`: an existing one being moved, with what it holds) may go on a pad of
 * drum rack `rackId`: one level below the drum rack, at most `MAX_RACK_DEPTH` deep.
 */
function checkPadDepth(ctx: ReducerContext, rackId: string, id: string | null): void {
  const p = ctx.tx.project;
  const level = (devicePath(p, rackId)?.length ?? MAX_RACK_DEPTH) + 1;
  const below = id === null ? 0 : nestedRackHeight(p, id);
  if (level + below > MAX_RACK_DEPTH) fail("InvalidArgument", `racks nest at most ${MAX_RACK_DEPTH} deep`);
}

function pad(ctx: ReducerContext, id: string): DrumPad {
  return ctx.tx.get("DrumPad", id) ?? fail("NotFound", `drum pad ${id}`);
}

function checkNote(note: number): void {
  if (!(Number.isInteger(note) && note >= 0 && note <= 127)) fail("InvalidArgument", `invalid note ${note}`);
}

function padsOf(ctx: ReducerContext, rackId: string): DrumPad[] {
  return ctx.tx
    .all("DrumPad")
    .filter((p) => p.rack === rackId)
    .sort((a, b) => a.note - b.note || (a.id < b.id ? -1 : 1));
}

// ─── Structure rules used by the core reducer ──────────────────────────────────────────

/**
 * `Device::Move` rules: pad devices move with `DrumRack::MoveDevice`; a rack with pads
 * can't change track (its pad chains would be stranded).
 */
export function checkDeviceMove(ctx: ReducerContext, d: Device, track: TrackId): void {
  if (d.pad !== null) fail("InvalidArgument", `device ${d.id} is on a drum pad: use DrumRack::MoveDevice`);
  if (d.track !== track && padsOf(ctx, d.id).length > 0) {
    fail("InvalidArgument", "a drum rack with pads cannot move to another track (its pad chains live on its track); duplicate it there instead");
  }
}

/**
 * Copy rack `src`'s pads and pad chains onto rack `dst` on `track` (new ids).
 * `deviceIds` receives old → new pad-device ids.
 */
export function copyRackPads(ctx: ReducerContext, src: string, dst: string, track: TrackId, deviceIds?: Map<string, string>): void {
  for (const p of padsOf(ctx, src)) {
    const newPad = ctx.newId();
    ctx.tx.upsert("DrumPad", { ...p, id: newPad, rack: dst });
    for (const d of padChainOf(ctx.tx.project, p.id)) {
      const id = ctx.newId();
      deviceIds?.set(d.id, id);
      ctx.tx.upsert("Device", { ...d, id, track, pad: newPad });
    }
  }
}

/** Remove a pad with its chain (devices first, like the Rust cascade). */
export function deletePadCascade(ctx: ReducerContext, padId: string, deleteDevice: (id: string) => void): void {
  onSidechainChainDeleted(ctx, { type: "Pad", id: padId });
  for (const d of padChainOf(ctx.tx.project, padId)) deleteDevice(d.id);
  ctx.tx.remove("DrumPad", padId);
}

/** A deleted rack takes its pads (and their chains) with it. */
export function onRackDeleted(ctx: ReducerContext, rackId: string, deleteDevice: (id: string) => void): void {
  for (const p of padsOf(ctx, rackId)) deletePadCascade(ctx, p.id, deleteDevice);
}

// ─── Commands ───────────────────────────────────────────────────────────────────────────

function addPad(ctx: ReducerContext, id: string, rackId: string, note: number, name: string | null): void {
  checkNote(note);
  const r = rack(ctx, rackId);
  if (ctx.tx.all("DrumPad").some((p) => p.rack === r.id && p.note === note)) fail("InvalidArgument", `note ${note} already has a pad`);
  ctx.tx.upsert("DrumPad", { id, rack: r.id, note, name: name ?? noteName(note), color: null, choke_group: null, volume: 0, pan: 0, mute: false });
}

export function drumRackCommand(ctx: ReducerContext, c: DrumRackCommand, deleteDevice: (id: string) => void): void {
  const { tx } = ctx;
  switch (c.type) {
    case "AddPad":
      if (tx.get("DrumPad", c.id)) return;
      addPad(ctx, c.id, c.rack, c.note, c.name);
      break;
    case "RemovePad":
      pad(ctx, c.id);
      deletePadCascade(ctx, c.id, deleteDevice);
      break;
    case "SetPadNote": {
      checkNote(c.note);
      const p = pad(ctx, c.id);
      const other = tx.all("DrumPad").find((o) => o.rack === p.rack && o.note === c.note && o.id !== p.id);
      if (other) tx.upsert("DrumPad", { ...other, note: p.note });
      tx.upsert("DrumPad", { ...p, note: c.note });
      break;
    }
    case "RenamePad":
      tx.upsert("DrumPad", { ...pad(ctx, c.id), name: c.name });
      break;
    case "SetPadColor":
      tx.upsert("DrumPad", { ...pad(ctx, c.id), color: c.color });
      break;
    case "SetChokeGroup":
      if (c.group !== null && !(c.group >= 1 && c.group <= 16)) fail("InvalidArgument", "choke group must be 1..=16");
      tx.upsert("DrumPad", { ...pad(ctx, c.id), choke_group: c.group });
      break;
    case "SetPadVolume":
      tx.upsert("DrumPad", { ...pad(ctx, c.id), volume: clamp(c.volume, -144, 6) });
      break;
    case "SetPadPan":
      tx.upsert("DrumPad", { ...pad(ctx, c.id), pan: clamp(c.pan, -1, 1) });
      break;
    case "SetPadMute":
      tx.upsert("DrumPad", { ...pad(ctx, c.id), mute: c.mute });
      break;
    case "SetPadSolo":
      // Runtime state in the engine (compiled as mute of the other pads): no document
      // change, no undo step. The UI keeps the solo buttons' state (`features/drum-rack`).
      pad(ctx, c.id);
      break;
    case "InsertDevice": {
      if (tx.get("Device", c.id)) fail("InvalidArgument", `device ${c.id} already exists`);
      const p = pad(ctx, c.pad);
      const r = rack(ctx, p.rack);
      if (c.device.type === "Plugin") fail("Unsupported", "plugins are not available in the mock engine");
      if (c.device.device.type === "DrumRack") fail("InvalidArgument", "drum racks cannot be nested in pads");
      if (CHAIN_RACK_TYPES.includes(c.device.device.type)) checkPadDepth(ctx, r.id, null);
      tx.upsert("Device", {
        id: c.id,
        track: r.track,
        order: keyForInsert(padChainOf(ctx.tx.project, p.id), c.before),
        name: defaultDeviceName(c.device.device),
        enabled: true,
        kind: { type: "Builtin", device: c.device.device },
        params: defaultParams(c.device.device),
        sidechain: null,
        pad: p.id,
      });
      addFirstChain(ctx, c.id, c.device.device.type);
      break;
    }
    case "MoveDevice": {
      const d = tx.get("Device", c.id) ?? fail("NotFound", `device ${c.id}`);
      if (c.pad === null) {
        const chain = tx
          .all("Device")
          .filter((x) => x.track === d.track && x.pad === null && x.id !== d.id)
          .sort(byOrder);
        tx.upsert("Device", { ...d, pad: null, order: keyForInsert(chain, c.before) });
      } else {
        const p = pad(ctx, c.pad);
        const r = rack(ctx, p.rack);
        if (r.track !== d.track) fail("InvalidArgument", "a pad device must be on the rack's track");
        if (isRack(d) || hasNestedDrumRack(ctx.tx.project, d.id)) fail("InvalidArgument", "drum racks cannot be nested in pads");
        if (isChainRack(d)) {
          if (devicePath(ctx.tx.project, r.id)?.includes(d.id) ?? true) fail("InvalidArgument", "a rack cannot go inside itself");
          checkPadDepth(ctx, r.id, d.id);
        }
        tx.upsert("Device", { ...d, pad: p.id, order: keyForInsert(padChainOf(ctx.tx.project, p.id, d.id), c.before) });
      }
      break;
    }
    case "AddSamplePad": {
      if (!tx.get("Media", c.media)) fail("NotFound", `media ${c.media}`);
      if (tx.get("Device", c.device)) fail("InvalidArgument", `device ${c.device} already exists`);
      addPad(ctx, c.pad, c.rack, c.note, tx.get("Media", c.media)!.name.replace(/\.[^.]+$/, ""));
      const r = rack(ctx, c.rack);
      const device = newBuiltinDevice("Sampler");
      if (device.type === "Sampler") device.sample = c.media;
      tx.upsert("Device", {
        id: c.device,
        track: r.track,
        order: keyForInsert([], null),
        name: defaultDeviceName("Sampler"),
        enabled: true,
        kind: { type: "Builtin", device },
        params: defaultParams("Sampler"),
        sidechain: null,
        pad: c.pad,
      });
      break;
    }
  }
}

/** Markers of an automatic slicing of `lengthSeconds` (the mock has no onset detection). */
function autoMarkers(ctx: ReducerContext, mode: AutoSlice, lengthSeconds: number): number[] {
  let step: number;
  if (mode.type === "Grid") {
    if (!(mode.beats > 0)) fail("InvalidArgument", "grid must be > 0");
    const bpm = ctx.tx.all("TempoPoint").find((p) => p.time < BEATS_EPSILON)?.bpm ?? 120;
    step = (mode.beats * 60) / bpm;
  } else {
    // Transients = 8 equal slices.
    const count = mode.type === "Equal" ? Math.max(1, mode.count) : 8;
    step = lengthSeconds / count;
  }
  const markers: number[] = [];
  for (let t = 0; t < lengthSeconds - 1e-9 && markers.length < 128; t += step) markers.push(t);
  return markers;
}

export function sliceCommand(ctx: ReducerContext, c: SliceCommand, deleteDevice: (id: string) => void): void {
  // A retried ToDrumRack (the sampler is already gone, the rack exists) is harmless.
  if (c.type === "ToDrumRack" && ctx.tx.get("Device", c.rack) && !ctx.tx.get("Device", c.device)) return;
  if (c.type === "SampleToPads") return sampleToPads(ctx, c, deleteDevice);
  const d = ctx.tx.get("Device", c.device) ?? fail("NotFound", `device ${c.device}`);
  if (d.kind.type !== "Builtin" || d.kind.device.type !== "Sampler") fail("InvalidArgument", `device ${d.id} is not a sampler`);
  const sampler = d.kind.device;
  const media = sampler.sample !== null ? ctx.tx.get("Media", sampler.sample) : undefined;
  const lengthSeconds = media ? media.frames / media.sample_rate : 0;
  const s = sampler.slices;
  const sorted = (markers: number[]) => {
    const out: number[] = [];
    for (const m of [...markers].filter((x) => Number.isFinite(x) && x >= 0).sort((a, b) => a - b)) {
      if (out.length === 0 || m - out[out.length - 1]! >= 0.001) out.push(m);
    }
    return out;
  };
  let next: SliceSettings;
  switch (c.type) {
    case "SetEnabled":
      next = { ...s, enabled: c.enabled };
      break;
    case "SetBaseNote":
      checkNote(c.note);
      next = { ...s, base_note: c.note };
      break;
    case "Add":
      next = { ...s, markers: sorted([...s.markers, ...c.positions]) };
      break;
    case "Move": {
      if (c.index >= s.markers.length) fail("NotFound", `slice ${c.index}`);
      const markers = [...s.markers];
      markers[c.index] = c.position;
      next = { ...s, markers: sorted(markers) };
      break;
    }
    case "Remove": {
      const drop = new Set(c.indices);
      next = { ...s, markers: s.markers.filter((_, i) => !drop.has(i)) };
      break;
    }
    case "Auto": {
      if (!media) fail("InvalidState", "the sampler has no sample");
      next = { ...s, enabled: true, markers: sorted(autoMarkers(ctx, c.mode, lengthSeconds)) };
      break;
    }
    case "ToDrumRack":
      return toDrumRack(ctx, d, c.rack, c.pads, lengthSeconds);
  }
  ctx.tx.upsert("Device", { ...d, kind: { type: "Builtin", device: { ...sampler, slices: next } } });
}

/** Mirrors `ether-controller/src/drum_rack` `to_drum_rack`: one sampler pad per slice. */
function toDrumRack(ctx: ReducerContext, d: Device, rackId: string, ids: SlicePadIds[], lengthSeconds: number): void {
  const { tx } = ctx;
  if (tx.get("Device", rackId) && !tx.get("Device", d.id)) return;
  if (d.kind.type !== "Builtin" || d.kind.device.type !== "Sampler") fail("InvalidArgument", `device ${d.id} is not a sampler`);
  const sampler = d.kind.device;
  if (d.pad !== null || isUnderDrumPad(tx.project, d.id)) fail("InvalidArgument", "drum racks cannot be nested in pads");
  if (tx.get("Device", rackId)) fail("InvalidArgument", `device ${rackId} already exists`);
  if (sampler.sample === null) fail("InvalidState", "the sampler has no sample");
  const s = sampler.slices;
  const count = Math.min(s.markers.length, 128 - s.base_note);
  if (count === 0) fail("InvalidState", "the sampler has no slices");
  if (ids.length < count) fail("InvalidArgument", `${count} slices need ${count} pad ids (${ids.length} given)`);
  const used = ids.slice(0, count);
  const fresh = used.every((i) => !tx.get("DrumPad", i.pad) && !tx.get("Device", i.device));
  if (!fresh || new Set(used.map((i) => i.device)).size !== count || new Set(used.map((i) => i.pad)).size !== count) {
    fail("InvalidArgument", "pad and device ids must be new and distinct");
  }
  tx.upsert("Device", {
    id: rackId,
    track: d.track,
    order: d.order,
    // In the sampler's place: on the track chain or a rack chain.
    ...(d.chain != null ? { chain: d.chain } : {}),
    name: defaultDeviceName("DrumRack"),
    enabled: true,
    kind: { type: "Builtin", device: { type: "DrumRack" } },
    params: defaultParams("DrumRack"),
    sidechain: null,
    pad: null,
  });
  const percent = (t: number) => clamp((t / lengthSeconds) * 100, 0, 100);
  used.forEach((pid, i) => {
    addPad(ctx, pid.pad, rackId, s.base_note + i, `Slice ${i + 1}`);
    const end = s.markers[i + 1] ?? lengthSeconds;
    tx.upsert("Device", {
      id: pid.device,
      track: d.track,
      order: keyForInsert([], null),
      name: d.name,
      enabled: true,
      kind: { type: "Builtin", device: { ...sampler, slices: { ...s, enabled: false } } },
      // The sampler's sound, playing just the slice at its original pitch (pads play C3):
      // Root Key (1) = 60, Start (6) / End (7) in percent.
      params: { ...defaultParams("Sampler"), ...d.params, 1: 60, 6: percent(s.markers[i]!), 7: percent(end) },
      sidechain: null,
      pad: pid.pad,
    });
  });
  // The sampler goes with its automation lanes and MIDI mappings.
  for (const l of tx.all("AutomationLane")) {
    if (targetDevice(l.target) !== d.id) continue;
    for (const pt of tx.all("AutomationPoint")) if (pt.lane === l.id) tx.remove("AutomationPoint", pt.id);
    tx.remove("AutomationLane", l.id);
  }
  onDeviceDeleted(ctx, d.id);
  tx.remove("Device", d.id);
}

/** Mirrors `ether-controller/src/drum_rack` `sample_to_pads`: a sample sliced onto an existing rack. */
function sampleToPads(ctx: ReducerContext, c: Extract<SliceCommand, { type: "SampleToPads" }>, deleteDevice: (id: string) => void): void {
  const { tx } = ctx;
  checkNote(c.base_note);
  const r = rack(ctx, c.rack);
  const first = c.pads[0] ? tx.get("DrumPad", c.pads[0].pad) : undefined;
  if (first && first.rack === r.id) return;
  const media = tx.get("Media", c.media) ?? fail("NotFound", `media ${c.media}`);
  const lengthSeconds = media.frames / media.sample_rate;
  const markers = autoMarkers(ctx, c.mode, lengthSeconds);
  const count = Math.min(markers.length, 128 - c.base_note);
  if (count === 0) fail("InvalidState", "the sample has no slices");
  if (c.pads.length < count) fail("InvalidArgument", `${count} slices need ${count} pad ids (${c.pads.length} given)`);
  const used = c.pads.slice(0, count);
  const fresh = used.every((i) => !tx.get("DrumPad", i.pad) && !tx.get("Device", i.device));
  if (!fresh || new Set(used.map((i) => i.device)).size !== count || new Set(used.map((i) => i.pad)).size !== count) {
    fail("InvalidArgument", "pad and device ids must be new and distinct");
  }
  // The slices' notes are theirs: pads already there go, with their chains.
  for (const p of padsOf(ctx, r.id)) if (p.note >= c.base_note && p.note < c.base_note + count) deletePadCascade(ctx, p.id, deleteDevice);
  const percent = (t: number) => clamp((t / lengthSeconds) * 100, 0, 100);
  const slices: SliceSettings = { enabled: false, base_note: c.base_note, markers };
  used.forEach((pid, i) => {
    addPad(ctx, pid.pad, r.id, c.base_note + i, `Slice ${i + 1}`);
    const end = markers[i + 1] ?? lengthSeconds;
    tx.upsert("Device", {
      id: pid.device,
      track: r.track,
      order: keyForInsert([], null),
      name: defaultDeviceName("Sampler"),
      enabled: true,
      kind: { type: "Builtin", device: { type: "Sampler", sample: c.media, slices } },
      // Root Key (1) = 60, Start (6) / End (7) in percent.
      params: { ...defaultParams("Sampler"), 1: 60, 6: percent(markers[i]!), 7: percent(end) },
      sidechain: null,
      pad: pid.pad,
    });
  });
}
