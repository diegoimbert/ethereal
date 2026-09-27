/**
 * Mock of `DrumRack::*` (pads, pad chains) and `Slice::*` (sampler slices), plus the drum
 * rack structure rules the core reducer calls for `Device::{Move, Duplicate, Remove}` and
 * `Track::{Duplicate, Delete}` (mirrors `ether-controller/src/doc/{devices,mod,tracks}.rs`).
 * Owned by `drum-rack`.
 */

import type { Device, DrumPad, DrumRackCommand, SliceCommand, SliceSettings, TrackId } from "@/generated";
import { BEATS_EPSILON } from "@/state/beats";
import { keyForInsert } from "@/state/orderKey";
import { builtinDescriptor, newBuiltinDevice } from "../builtinDevices";
import { defaultParams } from "../demoProject";
import { fail, type ReducerContext } from "../documentReducer";
import { byOrder, clamp } from "./shared";

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

function pad(ctx: ReducerContext, id: string): DrumPad {
  return ctx.tx.get("DrumPad", id) ?? fail("NotFound", `drum pad ${id}`);
}

function checkNote(note: number): void {
  if (!(Number.isInteger(note) && note >= 0 && note <= 127)) fail("InvalidArgument", `invalid note ${note}`);
}

/** Devices of a pad chain in chain order (optionally without `except`). */
export function padChain(ctx: ReducerContext, padId: string, except?: string): Device[] {
  return ctx.tx
    .all("Device")
    .filter((d) => d.pad === padId && d.id !== except)
    .sort(byOrder);
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

/** Siblings a duplicated device is placed among: its pad chain, or its track chain. */
export function duplicateSiblings(ctx: ReducerContext, d: Device, trackChain: Device[]): Device[] {
  return d.pad !== null ? padChain(ctx, d.pad) : trackChain;
}

/**
 * Copy rack `src`'s pads and pad chains onto rack `dst` on `track` (new ids).
 * `deviceIds` receives old → new pad-device ids.
 */
export function copyRackPads(ctx: ReducerContext, src: string, dst: string, track: TrackId, deviceIds?: Map<string, string>): void {
  for (const p of padsOf(ctx, src)) {
    const newPad = ctx.newId();
    ctx.tx.upsert("DrumPad", { ...p, id: newPad, rack: dst });
    for (const d of padChain(ctx, p.id)) {
      const id = ctx.newId();
      deviceIds?.set(d.id, id);
      ctx.tx.upsert("Device", { ...d, id, track, pad: newPad });
    }
  }
}

/** Remove a pad with its chain (devices first, like the Rust cascade). */
export function deletePadCascade(ctx: ReducerContext, padId: string, deleteDevice: (id: string) => void): void {
  for (const d of padChain(ctx, padId)) deleteDevice(d.id);
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
    case "InsertDevice": {
      if (tx.get("Device", c.id)) fail("InvalidArgument", `device ${c.id} already exists`);
      const p = pad(ctx, c.pad);
      const r = rack(ctx, p.rack);
      if (c.device.type === "Plugin") fail("Unsupported", "plugins are not available in the mock engine");
      if (c.device.device.type === "DrumRack") fail("InvalidArgument", "drum racks cannot be nested in pads");
      tx.upsert("Device", {
        id: c.id,
        track: r.track,
        order: keyForInsert(padChain(ctx, p.id), c.before),
        name: builtinDescriptor(c.device.device).name,
        enabled: true,
        kind: { type: "Builtin", device: c.device.device },
        params: defaultParams(c.device.device),
        sidechain: null,
        pad: p.id,
      });
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
        if (isRack(d)) fail("InvalidArgument", "drum racks cannot be nested in pads");
        tx.upsert("Device", { ...d, pad: p.id, order: keyForInsert(padChain(ctx, p.id, d.id), c.before) });
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
        name: "Sampler",
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

export function sliceCommand(ctx: ReducerContext, c: SliceCommand): void {
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
      let step: number;
      if (c.mode.type === "Grid") {
        if (!(c.mode.beats > 0)) fail("InvalidArgument", "grid must be > 0");
        const bpm = ctx.tx.all("TempoPoint").find((p) => p.time < BEATS_EPSILON)?.bpm ?? 120;
        step = (c.mode.beats * 60) / bpm;
      } else {
        // The mock has no onset detection: transients = 8 equal slices.
        const count = c.mode.type === "Equal" ? Math.max(1, c.mode.count) : 8;
        step = lengthSeconds / count;
      }
      const markers: number[] = [];
      for (let t = 0; t < lengthSeconds - 1e-9 && markers.length < 128; t += step) markers.push(t);
      next = { ...s, enabled: true, markers: sorted(markers) };
      break;
    }
    case "ToDrumRack":
      return fail("Unsupported", "slice to drum rack is not available in the mock engine");
  }
  ctx.tx.upsert("Device", { ...d, kind: { type: "Builtin", device: { ...sampler, slices: next } } });
}
