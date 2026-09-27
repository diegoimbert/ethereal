/**
 * MockTransport simulation of the roadmap v2 *document* commands (contracts-2): tempo map
 * CRUD + metronome settings, markers, clip fade curves / reverse / crossfade, sidechain,
 * groove (humanize, project swing), drum racks, sampler slices and MIDI mappings.
 *
 * Same rules as `documentReducer.ts`: every write goes through the recording `Tx`, errors
 * throw `CommandFailedError` (the caller rolls back), validation mirrors the Rust model
 * where cheap. Feature nodes may refine these simulations; the Rust controller is the
 * reference.
 */

import type {
  Clip,
  ClipCommand,
  Device,
  DrumPad,
  DrumRackCommand,
  FadeCurve,
  GrooveCommand,
  MarkerCommand,
  MidiMapCommand,
  MidiMapping,
  SliceCommand,
  SliceSettings,
  TempoCommand,
  TimeSignature,
  TrackId,
} from "@/generated";
import { BEATS_EPSILON } from "@/state/beats";
import { keyForInsert } from "@/state/orderKey";
import { builtinDescriptor, newBuiltinDevice } from "./builtinDevices";
import { defaultParams } from "./demoProject";
import { fail, type ReducerContext } from "./documentReducer";
import { mulberry32 } from "./random";

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/** MIDI note name with C3 = 60 (Ableton convention). */
export function noteName(note: number): string {
  return `${NOTE_NAMES[note % 12]}${Math.floor(note / 12) - 2}`;
}

function validSignature(sig: TimeSignature): boolean {
  return Number.isInteger(sig.numerator) && sig.numerator >= 1 && sig.numerator <= 99 && [1, 2, 4, 8, 16, 32].includes(sig.denominator);
}

function checkCurve(c: FadeCurve | null): void {
  if (c?.type === "Curve" && !(c.tension >= -1 && c.tension <= 1)) fail("InvalidArgument", "curve tension must be in -1..=1");
}

// ─── Tempo ──────────────────────────────────────────────────────────────────────────────

export function tempoCommand(ctx: ReducerContext, c: TempoCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "AddTempoPoint":
      if (tx.get("TempoPoint", c.id)) return;
      if (!(c.time >= 0)) fail("InvalidArgument", "tempo point time must be >= 0");
      tx.upsert("TempoPoint", { id: c.id, time: c.time, bpm: clamp(c.bpm, 20, 999), curve: c.curve });
      break;
    case "EditTempoPoint": {
      const p = tx.get("TempoPoint", c.id) ?? fail("NotFound", `tempo point ${c.id}`);
      if (c.time !== null && p.time < BEATS_EPSILON && c.time > BEATS_EPSILON) fail("InvalidArgument", "the tempo point at beat 0 cannot move");
      tx.upsert("TempoPoint", {
        ...p,
        time: c.time !== null ? Math.max(0, c.time) : p.time,
        bpm: c.bpm !== null ? clamp(c.bpm, 20, 999) : p.bpm,
        curve: c.curve ?? p.curve,
      });
      break;
    }
    case "RemoveTempoPoints":
      for (const id of c.ids) {
        const p = tx.get("TempoPoint", id) ?? fail("NotFound", `tempo point ${id}`);
        if (p.time < BEATS_EPSILON) fail("InvalidArgument", "the tempo point at beat 0 cannot be removed");
        tx.remove("TempoPoint", id);
      }
      break;
    case "AddTimeSignature":
      if (tx.get("TimeSignature", c.id)) return;
      if (!validSignature(c.signature)) fail("InvalidArgument", "invalid time signature");
      if (!(c.time >= 0)) fail("InvalidArgument", "time signature time must be >= 0");
      tx.upsert("TimeSignature", { id: c.id, time: c.time, signature: c.signature });
      break;
    case "EditTimeSignature": {
      const p = tx.get("TimeSignature", c.id) ?? fail("NotFound", `time signature ${c.id}`);
      if (c.signature !== null && !validSignature(c.signature)) fail("InvalidArgument", "invalid time signature");
      if (c.time !== null && p.time < BEATS_EPSILON && c.time > BEATS_EPSILON) fail("InvalidArgument", "the time signature at beat 0 cannot move");
      tx.upsert("TimeSignature", { ...p, time: c.time !== null ? Math.max(0, c.time) : p.time, signature: c.signature ?? p.signature });
      break;
    }
    case "RemoveTimeSignatures":
      for (const id of c.ids) {
        const p = tx.get("TimeSignature", id) ?? fail("NotFound", `time signature ${id}`);
        if (p.time < BEATS_EPSILON) fail("InvalidArgument", "the time signature at beat 0 cannot be removed");
        tx.remove("TimeSignature", id);
      }
      break;
    case "SetMetronomeSettings": {
      const s = tx.project.settings;
      tx.setSettings({
        ...s,
        metronome_volume: c.volume !== null ? clamp(c.volume, -144, 6) : s.metronome_volume,
        metronome_accent: c.accent ?? s.metronome_accent,
        metronome_sound: c.sound ?? s.metronome_sound,
      });
      break;
    }
  }
}

// ─── Markers ────────────────────────────────────────────────────────────────────────────

export function markerCommand(ctx: ReducerContext, c: MarkerCommand): void {
  const { tx } = ctx;
  const marker = (id: string) => tx.get("Marker", id) ?? fail("NotFound", `marker ${id}`);
  switch (c.type) {
    case "Add":
      if (tx.get("Marker", c.id)) return;
      if (!(c.position >= 0)) fail("InvalidArgument", "marker position must be >= 0");
      tx.upsert("Marker", { id: c.id, position: c.position, name: c.name ?? `Marker ${tx.all("Marker").length + 1}`, color: c.color });
      break;
    case "Move":
      if (!(c.position >= 0)) fail("InvalidArgument", "marker position must be >= 0");
      tx.upsert("Marker", { ...marker(c.id), position: c.position });
      break;
    case "Rename":
      tx.upsert("Marker", { ...marker(c.id), name: c.name });
      break;
    case "SetColor":
      tx.upsert("Marker", { ...marker(c.id), color: c.color });
      break;
    case "Remove":
      for (const id of c.ids) {
        marker(id);
        tx.remove("Marker", id);
      }
      break;
  }
}

// ─── Clips (v2 variants) ────────────────────────────────────────────────────────────────

function audioClip(ctx: ReducerContext, id: string) {
  const cl = ctx.tx.get("Clip", id) ?? fail("NotFound", `clip ${id}`);
  if (cl.content.type !== "Audio") fail("InvalidArgument", `clip ${id} is not an audio clip`);
  return { clip: cl, content: cl.content };
}

export function clipV2Command(ctx: ReducerContext, c: Extract<ClipCommand, { type: "SetFadeCurves" | "SetReversed" | "Crossfade" }>): void {
  const { tx } = ctx;
  switch (c.type) {
    case "SetFadeCurves": {
      checkCurve(c.fade_in);
      checkCurve(c.fade_out);
      const { clip, content } = audioClip(ctx, c.id);
      tx.upsert("Clip", {
        ...clip,
        content: { ...content, fade_in_curve: c.fade_in ?? content.fade_in_curve, fade_out_curve: c.fade_out ?? content.fade_out_curve },
      });
      break;
    }
    case "SetReversed": {
      const { clip, content } = audioClip(ctx, c.id);
      tx.upsert("Clip", { ...clip, content: { ...content, reversed: c.reversed } });
      break;
    }
    case "Crossfade": {
      checkCurve(c.curve);
      if (!(c.length > 0)) fail("InvalidArgument", "crossfade length must be > 0");
      const a = audioClip(ctx, c.first);
      const b = audioClip(ctx, c.second);
      if (a.clip.track !== b.clip.track) fail("InvalidArgument", "crossfaded clips must be on the same track");
      // The mock simply extends the first clip so the overlap is `length` (no source checks).
      const end = Math.max(a.clip.start + a.clip.length, b.clip.start + c.length);
      const first: Clip = {
        ...a.clip,
        length: end - a.clip.start,
        content: { ...a.content, fade_out: c.length, fade_out_curve: c.curve },
      };
      tx.upsert("Clip", first);
      tx.upsert("Clip", { ...b.clip, content: { ...b.content, fade_in: c.length, fade_in_curve: c.curve } });
      break;
    }
  }
}

// ─── Sidechain ──────────────────────────────────────────────────────────────────────────

export function setSidechain(ctx: ReducerContext, deviceId: string, source: TrackId | null): void {
  const d = ctx.tx.get("Device", deviceId) ?? fail("NotFound", `device ${deviceId}`);
  if (source !== null) {
    if (!ctx.tx.get("Track", source)) fail("NotFound", `track ${source}`);
    if (source === d.track) fail("InvalidArgument", "a device cannot sidechain its own track");
  }
  ctx.tx.upsert("Device", { ...d, sidechain: source });
}

// ─── Groove ─────────────────────────────────────────────────────────────────────────────

export function grooveCommand(ctx: ReducerContext, c: GrooveCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "Humanize": {
      if (!tx.get("Clip", c.clip)) fail("NotFound", `clip ${c.clip}`);
      const ids = c.notes ? new Set(c.notes) : null;
      const rand = mulberry32(c.seed);
      const notes = tx
        .all("Note")
        .filter((n) => n.clip === c.clip && (!ids || ids.has(n.id)))
        .sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
      for (const n of notes) {
        const dt = (rand() * 2 - 1) * Math.max(0, c.timing);
        const dv = (rand() * 2 - 1) * Math.max(0, c.velocity);
        tx.upsert("Note", { ...n, start: Math.max(0, n.start + dt), velocity: clamp(n.velocity + dv, 0, 1) });
      }
      break;
    }
    case "SetSwing":
      if (!(c.grid > 0)) fail("InvalidArgument", "swing grid must be > 0");
      tx.setSettings({ ...tx.project.settings, swing: clamp(c.amount, 0, 1), swing_grid: c.grid });
      break;
  }
}

/** Swing delay for a quantize target `t` on `grid`: odd grid positions move by `swing·grid/3`. */
export function swingOffset(t: number, grid: number, swing: number): number {
  const index = Math.round(t / grid);
  return Math.abs(index % 2) === 1 ? (clamp(swing, 0, 1) * grid) / 3 : 0;
}

// ─── Drum racks ─────────────────────────────────────────────────────────────────────────

function rack(ctx: ReducerContext, id: string): Device {
  const d = ctx.tx.get("Device", id) ?? fail("NotFound", `device ${id}`);
  if (d.kind.type !== "Builtin" || d.kind.device.type !== "DrumRack" || d.pad !== null) {
    fail("InvalidArgument", `device ${id} is not a drum rack`);
  }
  return d;
}

function pad(ctx: ReducerContext, id: string): DrumPad {
  return ctx.tx.get("DrumPad", id) ?? fail("NotFound", `drum pad ${id}`);
}

function checkNote(note: number): void {
  if (!(Number.isInteger(note) && note >= 0 && note <= 127)) fail("InvalidArgument", `invalid note ${note}`);
}

function padChain(ctx: ReducerContext, padId: string, except?: string): Device[] {
  return ctx.tx
    .all("Device")
    .filter((d) => d.pad === padId && d.id !== except)
    .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : 0));
}

/** Remove a pad with its chain (devices first, like the Rust cascade). */
export function deletePadCascade(ctx: ReducerContext, padId: string, deleteDevice: (id: string) => void): void {
  for (const d of padChain(ctx, padId)) deleteDevice(d.id);
  ctx.tx.remove("DrumPad", padId);
}

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
          .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : 0));
        tx.upsert("Device", { ...d, pad: null, order: keyForInsert(chain, c.before) });
      } else {
        const p = pad(ctx, c.pad);
        const r = rack(ctx, p.rack);
        if (r.track !== d.track) fail("InvalidArgument", "a pad device must be on the rack's track");
        if (d.kind.type === "Builtin" && d.kind.device.type === "DrumRack") fail("InvalidArgument", "drum racks cannot be nested in pads");
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

// ─── Slices ─────────────────────────────────────────────────────────────────────────────

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

// ─── MIDI mappings (document part) ──────────────────────────────────────────────────────

export function checkMapping(ctx: ReducerContext, m: MidiMapping): void {
  const { tx } = ctx;
  if (m.source.channel !== null && !(m.source.channel >= 0 && m.source.channel <= 15)) fail("InvalidArgument", "MIDI channel must be 0..=15");
  if (!(m.min >= 0 && m.min <= 1 && m.max >= 0 && m.max <= 1)) fail("InvalidArgument", "mapping range must be within 0..=1");
  const t = m.target;
  switch (t.type) {
    case "Param":
      switch (t.target.type) {
        case "TrackVolume":
        case "TrackPan":
          if (!tx.get("Track", t.target.track)) fail("NotFound", `track ${t.target.track}`);
          break;
        case "SendLevel":
          if (!tx.get("Send", t.target.send)) fail("NotFound", `send ${t.target.send}`);
          break;
        case "DeviceParam":
          if (!tx.get("Device", t.target.device)) fail("NotFound", `device ${t.target.device}`);
          break;
      }
      break;
    case "TrackMute":
    case "TrackSolo":
    case "TrackArm":
      if (!tx.get("Track", t.track)) fail("NotFound", `track ${t.track}`);
      break;
    case "Transport":
      break;
  }
}

/** Insert `m`, replacing a mapping with the same source (one undo step). */
export function putMapping(ctx: ReducerContext, m: MidiMapping): void {
  checkMapping(ctx, m);
  const same = JSON.stringify(m.source);
  for (const o of ctx.tx.all("MidiMapping")) {
    if (o.id !== m.id && JSON.stringify(o.source) === same) ctx.tx.remove("MidiMapping", o.id);
  }
  ctx.tx.upsert("MidiMapping", m);
}

export function midiMapCommand(ctx: ReducerContext, c: MidiMapCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "Map":
      putMapping(ctx, c.mapping);
      break;
    case "Edit": {
      const m = tx.get("MidiMapping", c.id) ?? fail("NotFound", `MIDI mapping ${c.id}`);
      const next = { ...m, min: c.min ?? m.min, max: c.max ?? m.max, mode: c.mode ?? m.mode };
      checkMapping(ctx, next);
      tx.upsert("MidiMapping", next);
      break;
    }
    case "Unmap":
      for (const id of c.ids) {
        if (!tx.get("MidiMapping", id)) fail("NotFound", `MIDI mapping ${id}`);
        tx.remove("MidiMapping", id);
      }
      break;
    default:
      fail("Internal", `not a document MIDI map command: ${c.type}`);
  }
}

/** `true` if a mapping target references `track` directly. */
export function mappingRefsTrack(m: MidiMapping, track: TrackId): boolean {
  const t = m.target;
  if (t.type === "Param") return (t.target.type === "TrackVolume" || t.target.type === "TrackPan") && t.target.track === track;
  return (t.type === "TrackMute" || t.type === "TrackSolo" || t.type === "TrackArm") && t.track === track;
}
