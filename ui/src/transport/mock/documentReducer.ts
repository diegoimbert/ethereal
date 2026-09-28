/**
 * The MockTransport's document reducer: applies one *document* command (anything that
 * changes the `Project` and is undoable) to a recording `Tx`. The caller turns the
 * transaction into a `Patch` event and an undo step.
 *
 * Validation mirrors the real model where it's cheap (entity existence, track kinds,
 * sends target returns, ...); errors throw `CommandFailedError`
 * and the caller rolls the transaction back (commands are all-or-nothing).
 *
 * Every write replaces the entity object (`{ ...old, field }`); stored objects are never
 * mutated in place, since the same objects are handed to the UI in patches.
 */

import type {
  AutomationCommand,
  AutomationLane,
  AutomationTarget,
  Beats,
  Clip,
  ClipCommand,
  ClipId,
  Command,
  Device,
  DeviceCommand,
  ErrorCode,
  MixerCommand,
  Note,
  NoteCommand,
  ProjectCommand,
  RecordingCommand,
  ReplyValue,
  TimeSignature,
  Track,
  TrackCommand,
  TrackId,
  TransportCommand,
  WarpCommand,
} from "@/generated";
import { BEATS_EPSILON, snapBeats } from "@/state/beats";
import { compareOrderKeys, keyBetween, keyForInsert } from "@/state/orderKey";
import { CommandFailedError } from "../EngineTransport";
import { BUILTIN_DESCRIPTORS, builtinDescriptor, clampParam } from "./builtinDevices";
import { defaultParams, defaultTrackName, makeClip, makeTrack, MOCK_TRACK_COLORS } from "./demoProject";
import { isRoadmapDocumentCommand, reduceRoadmapCommand } from "./roadmap";
import { groupsTrackCommand } from "./roadmap/groupsBuses";
import { setZones } from "./roadmap/multisampler";
import { clipV2Command, isCrossfade } from "./roadmap/clipEditing";
import { onTrackDeletedTakes } from "./roadmap/comping";
import { checkDeviceMove, copyRackPads, duplicateSiblings, onRackDeleted } from "./roadmap/drumRack";
import { swingOffset } from "./roadmap/groove";
import { onDeviceDeleted, onSendDeleted, onTrackDeleted } from "./roadmap/shared";
import { setSidechain } from "./roadmap/sidechain";
import { bpmAt, tempoPointAt, signaturePointAt } from "./tempo";
import type { Tx } from "./tx";

export interface ReducerContext {
  tx: Tx;
  /** Id generator for entities the engine creates itself (children of duplicates, ...). */
  newId: () => string;
  /** Current playhead position (for "tempo at the playhead" commands). */
  position: Beats;
}

export function fail(code: ErrorCode, message: string): never {
  throw new CommandFailedError({ code, message });
}

const EPS = BEATS_EPSILON;
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const SILENCE_DB = -144;
const MAX_DB = 6;

/** `true` if the command changes the document (and so is undoable / allowed in a Batch). */
export function isDocumentCommand(command: Command): boolean {
  const c = command.command;
  switch (command.domain) {
    case "Track":
    case "Clip":
    case "Note":
    case "Automation":
    case "Mixer":
      return true;
    case "Device":
      return c.type !== "ListBuiltin" && c.type !== "GetDescriptor";
    case "Project":
      // Only renaming the *current* project is a document edit (checked in the reducer).
      return c.type === "Rename" || c.type === "SetScale";
    case "Transport":
      return [
        "SetLoopEnabled",
        "SetLoopRegion",
        "SetTempo",
        "SetTimeSignature",
        "SetMetronome",
      ].includes(c.type);
    case "Recording":
      // `Arm` is runtime state (not undoable), handled by the MockTransport itself.
      return ["SetMonitor", "SetInput", "SetCountIn"].includes(c.type);
    case "Warp":
      return c.type !== "DetectTempo";
    default:
      return isRoadmapDocumentCommand(command);
  }
}

/** Apply a document command. Returns the reply value (almost always `Unit`). */
export function reduceDocumentCommand(ctx: ReducerContext, command: Command): ReplyValue {
  switch (command.domain) {
    case "Track":
      trackCommand(ctx, command.command);
      break;
    case "Mixer":
      mixerCommand(ctx, command.command);
      break;
    case "Device":
      return deviceCommand(ctx, command.command);
    case "Clip":
      clipCommand(ctx, command.command);
      break;
    case "Note":
      noteCommand(ctx, command.command);
      break;
    case "Automation":
      automationCommand(ctx, command.command);
      break;
    case "Transport":
      transportSettingsCommand(ctx, command.command);
      break;
    case "Project":
      projectCommand(ctx, command.command);
      break;
    case "Recording":
      recordingCommand(ctx, command.command);
      break;
    case "Warp":
      warpCommand(ctx, command.command);
      break;
    default:
      // Roadmap v2 domains (`./roadmap`).
      if (!reduceRoadmapCommand(ctx, command, (id) => deleteDeviceCascade(ctx, id))) {
        fail("Internal", `not a document command: ${command.domain}`);
      }
  }
  return UNIT;
}

const UNIT: ReplyValue = { type: "Unit" };

// ─── Lookups ────────────────────────────────────────────────────────────────────────────

function track(ctx: ReducerContext, id: TrackId): Track {
  return ctx.tx.get("Track", id) ?? fail("NotFound", `track ${id}`);
}
function clip(ctx: ReducerContext, id: ClipId): Clip {
  return ctx.tx.get("Clip", id) ?? fail("NotFound", `clip ${id}`);
}
function device(ctx: ReducerContext, id: string): Device {
  return ctx.tx.get("Device", id) ?? fail("NotFound", `device ${id}`);
}
function audioContent(c: Clip) {
  if (c.content.type !== "Audio") fail("InvalidArgument", `clip ${c.id} is not an audio clip`);
  return c.content;
}

const byOrder = <T extends { order: string; id: string }>(a: T, b: T) =>
  compareOrderKeys(a.order, b.order) || compareOrderKeys(a.id, b.id);

function siblingsOf(ctx: ReducerContext, parent: TrackId | null, except?: TrackId): Track[] {
  return ctx.tx
    .all("Track")
    .filter((t) => t.parent === parent && t.id !== except)
    .sort(byOrder);
}

function descendants(ctx: ReducerContext, id: TrackId): Track[] {
  const out: Track[] = [];
  for (const t of ctx.tx.all("Track")) {
    if (t.parent === id) out.push(t, ...descendants(ctx, t.id));
  }
  return out;
}

// ─── Cascading deletes / deep copies ────────────────────────────────────────────────────

function deleteLane(ctx: ReducerContext, lane: string): void {
  for (const p of ctx.tx.all("AutomationPoint")) if (p.lane === lane) ctx.tx.remove("AutomationPoint", p.id);
  ctx.tx.remove("AutomationLane", lane);
}

function deleteLanesWhere(ctx: ReducerContext, pred: (l: AutomationLane) => boolean): void {
  for (const l of ctx.tx.all("AutomationLane")) if (pred(l)) deleteLane(ctx, l.id);
}

function deleteClipCascade(ctx: ReducerContext, id: ClipId): void {
  for (const n of ctx.tx.all("Note")) if (n.clip === id) ctx.tx.remove("Note", n.id);
  for (const m of ctx.tx.all("WarpMarker")) if (m.clip === id) ctx.tx.remove("WarpMarker", m.id);
  deleteLanesWhere(ctx, (l) => l.owner.type === "Clip" && l.owner.clip === id);
  ctx.tx.remove("Clip", id);
}

function deleteSendCascade(ctx: ReducerContext, id: string): void {
  deleteLanesWhere(ctx, (l) => l.target.type === "SendLevel" && l.target.send === id);
  onSendDeleted(ctx, id);
  ctx.tx.remove("Send", id);
}

function deleteDeviceCascade(ctx: ReducerContext, id: string): void {
  deleteLanesWhere(ctx, (l) => l.target.type === "DeviceParam" && l.target.device === id);
  onDeviceDeleted(ctx, id);
  // A drum rack takes its pads (and their chains) with it.
  onRackDeleted(ctx, id, (d) => deleteDeviceCascade(ctx, d));
  ctx.tx.remove("Device", id);
}

function deleteTrackCascade(ctx: ReducerContext, id: TrackId): void {
  for (const c of ctx.tx.all("Clip")) if (c.track === id) deleteClipCascade(ctx, c.id);
  // Pad devices go with their rack.
  for (const d of ctx.tx.all("Device")) if (d.track === id && d.pad === null) deleteDeviceCascade(ctx, d.id);
  onTrackDeleted(ctx, id);
  onTrackDeletedTakes(ctx, id);
  for (const s of ctx.tx.all("Send")) if (s.from === id || s.to === id) deleteSendCascade(ctx, s.id);
  deleteLanesWhere(
    ctx,
    (l) =>
      (l.owner.type === "Track" && l.owner.track === id) ||
      ((l.target.type === "TrackVolume" || l.target.type === "TrackPan") && l.target.track === id),
  );
  // Re-route tracks that pointed at the deleted one.
  for (const t of ctx.tx.all("Track")) {
    if (t.id === id) continue;
    const output = t.output.type === "Track" && t.output.track === id ? ({ type: "Default" } as const) : t.output;
    const input = t.input.type === "Track" && t.input.track === id ? ({ type: "None" } as const) : t.input;
    if (output !== t.output || input !== t.input) ctx.tx.upsert("Track", { ...t, output, input });
  }
  ctx.tx.remove("Track", id);
}

/**
 * Deep-copy a clip (notes, clip envelopes, warp markers) under `newId` with `overrides`.
 */
function copyClip(ctx: ReducerContext, src: Clip, newId: ClipId, overrides: Partial<Clip>): Clip {
  if (ctx.tx.get("Clip", newId)) fail("InvalidArgument", `clip ${newId} already exists`);
  const copy: Clip = { ...src, ...overrides, id: newId };
  ctx.tx.upsert("Clip", copy);
  for (const n of ctx.tx.all("Note")) if (n.clip === src.id) ctx.tx.upsert("Note", { ...n, id: ctx.newId(), clip: newId });
  for (const m of ctx.tx.all("WarpMarker")) {
    if (m.clip === src.id) ctx.tx.upsert("WarpMarker", { ...m, id: ctx.newId(), clip: newId });
  }
  for (const l of ctx.tx.all("AutomationLane")) {
    if (l.owner.type === "Clip" && l.owner.clip === src.id) {
      copyLane(ctx, l, { ...l, id: ctx.newId(), owner: { type: "Clip", clip: newId } });
    }
  }
  return copy;
}

function copyLane(ctx: ReducerContext, src: AutomationLane, lane: AutomationLane): void {
  ctx.tx.upsert("AutomationLane", lane);
  for (const p of ctx.tx.all("AutomationPoint")) {
    if (p.lane === src.id) ctx.tx.upsert("AutomationPoint", { ...p, id: ctx.newId(), lane: lane.id });
  }
}

// ─── Tracks ─────────────────────────────────────────────────────────────────────────────

/**
 * Default placement for `before: null`: at the end of the siblings, except that top-level
 * regular tracks go before the first return/master track and returns go before master
 * (Ableton's layout).
 */
function defaultBefore(ctx: ReducerContext, kind: Track["kind"], parent: TrackId | null, except?: TrackId): TrackId | null {
  if (parent !== null) return null;
  const sibs = siblingsOf(ctx, null, except);
  const stop = sibs.find((t) => t.kind === "Master" || (kind !== "Return" && t.kind === "Return"));
  return stop?.id ?? null;
}

function validateParent(ctx: ReducerContext, kind: Track["kind"], parent: TrackId | null, self?: TrackId): void {
  if (parent === null) return;
  if (kind === "Return" || kind === "Master") fail("InvalidArgument", `${kind} tracks must be top-level`);
  const p = track(ctx, parent);
  if (p.kind !== "Group") fail("InvalidArgument", `parent ${parent} is not a group track`);
  if (self !== undefined) {
    // No cycles: the parent may not be the track itself or one of its descendants.
    let cur: TrackId | null = parent;
    while (cur !== null) {
      if (cur === self) fail("InvalidArgument", "a group cannot be moved into itself");
      cur = ctx.tx.get("Track", cur)?.parent ?? null;
    }
  }
}

function trackCommand(ctx: ReducerContext, c: TrackCommand): void {
  // v0.2 (`groups-buses`, `roadmap/groupsBuses.ts`).
  if (groupsTrackCommand(ctx, c, (id) => deleteTrackCascade(ctx, id))) return;
  if (c.type === "SetScale") {
    const t = track(ctx, c.id);
    if (t.kind !== "Midi") fail("InvalidArgument", "track scales are only available on MIDI tracks");
    if (c.scale.type === "Custom" && (!Number.isInteger(c.scale.scale.root) || c.scale.scale.root < 0 || c.scale.scale.root > 11)) fail("InvalidArgument", "invalid scale root");
    ctx.tx.upsert("Track", { ...t, scale: c.scale });
    return;
  }
  const { tx } = ctx;
  switch (c.type) {
    case "Create": {
      if (tx.get("Track", c.id)) fail("InvalidArgument", `track ${c.id} already exists`);
      if (c.kind === "Master") fail("InvalidArgument", "there is exactly one master track");
      validateParent(ctx, c.kind, c.parent);
      const before = c.before ?? defaultBefore(ctx, c.kind, c.parent);
      const all = tx.all("Track");
      const sameKind = all.filter((t) => t.kind === c.kind).length;
      tx.upsert(
        "Track",
        makeTrack({
          id: c.id,
          kind: c.kind,
          name: c.name ?? defaultTrackName(c.kind, sameKind + 1),
          color: c.color ?? MOCK_TRACK_COLORS[all.length % MOCK_TRACK_COLORS.length]!,
          order: keyForInsert(siblingsOf(ctx, c.parent), before),
          parent: c.parent,
        }),
      );
      break;
    }
    case "Delete": {
      const t = track(ctx, c.id);
      if (t.kind === "Master") fail("InvalidArgument", "the master track cannot be deleted");
      for (const d of descendants(ctx, t.id).reverse()) deleteTrackCascade(ctx, d.id);
      deleteTrackCascade(ctx, t.id);
      break;
    }
    case "Duplicate": {
      const t = track(ctx, c.id);
      if (t.kind === "Master") fail("InvalidArgument", "the master track cannot be duplicated");
      if (tx.get("Track", c.new_id)) fail("InvalidArgument", `track ${c.new_id} already exists`);
      const sibs = siblingsOf(ctx, t.parent);
      const next = sibs[sibs.findIndex((s) => s.id === t.id) + 1];
      duplicateTrack(ctx, t, c.new_id, keyBetween(t.order, next?.order ?? null), t.parent);
      break;
    }
    case "Rename":
      tx.upsert("Track", { ...track(ctx, c.id), name: c.name });
      break;
    case "SetColor":
      tx.upsert("Track", { ...track(ctx, c.id), color: c.color });
      break;
    case "Move": {
      const t = track(ctx, c.id);
      validateParent(ctx, t.kind, c.parent, t.id);
      if (t.kind === "Master" && c.parent !== null) fail("InvalidArgument", "master must be top-level");
      if (c.before === t.id) break;
      const before = c.before ?? (t.kind === "Master" ? null : defaultBefore(ctx, t.kind, c.parent, t.id));
      tx.upsert("Track", { ...t, parent: c.parent, order: keyForInsert(siblingsOf(ctx, c.parent, t.id), before) });
      break;
    }
  }
}

/** Deep copy of a track (clips, notes, devices, sends, lanes, and children of groups). */
function duplicateTrack(ctx: ReducerContext, t: Track, newId: TrackId, order: string, parent: TrackId | null): void {
  const { tx } = ctx;
  tx.upsert("Track", { ...t, id: newId, order, parent });
  const deviceIds = new Map<string, string>();
  const sendIds = new Map<string, string>();
  // Track-chain devices; drum racks bring their pads and pad chains.
  for (const d of tx.all("Device")) {
    if (d.track !== t.id || d.pad !== null) continue;
    const id = ctx.newId();
    deviceIds.set(d.id, id);
    tx.upsert("Device", { ...d, id, track: newId });
    copyRackPads(ctx, d.id, id, newId, deviceIds);
  }
  for (const s of tx.all("Send")) {
    if (s.from !== t.id) continue;
    const id = ctx.newId();
    sendIds.set(s.id, id);
    tx.upsert("Send", { ...s, id, from: newId });
  }
  for (const cl of tx.all("Clip")) {
    if (cl.track === t.id) copyClip(ctx, cl, ctx.newId(), { track: newId });
  }
  const remap = (target: AutomationTarget): AutomationTarget | null => {
    switch (target.type) {
      case "TrackVolume":
      case "TrackPan":
        return target.track === t.id ? { ...target, track: newId } : target;
      case "SendLevel": {
        const send = sendIds.get(target.send);
        return send ? { type: "SendLevel", send } : null;
      }
      case "DeviceParam": {
        const dev = deviceIds.get(target.device);
        return dev ? { ...target, device: dev } : null;
      }
    }
  };
  for (const l of tx.all("AutomationLane")) {
    if (l.owner.type !== "Track" || l.owner.track !== t.id) continue;
    const target = remap(l.target);
    if (target) copyLane(ctx, l, { ...l, id: ctx.newId(), owner: { type: "Track", track: newId }, target });
  }
  // Children of a group, in order.
  const kids = siblingsOf(ctx, t.id);
  let prev: string | null = null;
  for (const k of kids) {
    prev = keyBetween(prev, null);
    duplicateTrack(ctx, k, ctx.newId(), prev, newId);
  }
}

// ─── Mixer ──────────────────────────────────────────────────────────────────────────────

function mixerCommand(ctx: ReducerContext, c: MixerCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "SetVolume": {
      const t = track(ctx, c.track);
      tx.upsert("Track", { ...t, mixer: { ...t.mixer, volume: clamp(c.volume, SILENCE_DB, MAX_DB) } });
      break;
    }
    case "SetPan": {
      const t = track(ctx, c.track);
      tx.upsert("Track", { ...t, mixer: { ...t.mixer, pan: clamp(c.pan, -1, 1) } });
      break;
    }
    case "SetMute": {
      const t = track(ctx, c.track);
      tx.upsert("Track", { ...t, mixer: { ...t.mixer, mute: c.mute } });
      break;
    }
    case "SetSolo": {
      const t = track(ctx, c.track);
      if (c.exclusive) {
        for (const o of tx.all("Track")) {
          if (o.id !== t.id && o.mixer.solo) tx.upsert("Track", { ...o, mixer: { ...o.mixer, solo: false } });
        }
      }
      tx.upsert("Track", { ...t, mixer: { ...t.mixer, solo: c.solo } });
      break;
    }
    case "SetOutput": {
      const t = track(ctx, c.track);
      if (c.output.type === "Track") {
        const target = track(ctx, c.output.track);
        if (target.id === t.id) fail("InvalidArgument", "a track cannot output to itself");
        if (target.kind !== "Group" && target.kind !== "Return") {
          fail("InvalidArgument", "outputs can only target group or return tracks");
        }
      }
      tx.upsert("Track", { ...t, output: c.output });
      break;
    }
    case "CreateSend": {
      if (tx.get("Send", c.id)) fail("InvalidArgument", `send ${c.id} already exists`);
      const from = track(ctx, c.from);
      const to = track(ctx, c.to);
      if (to.kind !== "Return") fail("InvalidArgument", "sends must target a return track");
      if (from.id === to.id || from.kind === "Master") fail("InvalidArgument", "invalid send source");
      if (tx.all("Send").some((s) => s.from === from.id && s.to === to.id)) {
        fail("InvalidArgument", "a send between these tracks already exists");
      }
      tx.upsert("Send", {
        id: c.id,
        from: from.id,
        to: to.id,
        level: clamp(c.level, SILENCE_DB, MAX_DB),
        pre_fader: c.pre_fader,
      });
      break;
    }
    case "SetSendLevel": {
      const s = tx.get("Send", c.send) ?? fail("NotFound", `send ${c.send}`);
      tx.upsert("Send", { ...s, level: clamp(c.level, SILENCE_DB, MAX_DB) });
      break;
    }
    case "SetSendPreFader": {
      const s = tx.get("Send", c.send) ?? fail("NotFound", `send ${c.send}`);
      tx.upsert("Send", { ...s, pre_fader: c.pre_fader });
      break;
    }
    case "DeleteSend":
      if (!tx.get("Send", c.send)) fail("NotFound", `send ${c.send}`);
      deleteSendCascade(ctx, c.send);
      break;
  }
}

// ─── Devices ────────────────────────────────────────────────────────────────────────────

function chainOf(ctx: ReducerContext, trackId: TrackId, except?: string): Device[] {
  return ctx.tx
    .all("Device")
    .filter((d) => d.track === trackId && d.pad === null && d.id !== except)
    .sort(byOrder);
}

function checkDeviceFits(t: Track, kind: Device["kind"]): void {
  if (kind.type !== "Builtin") return;
  const { category } = builtinDescriptor(kind.device);
  if (category === "Instrument" && t.kind !== "Midi") {
    fail("InvalidArgument", `instruments can only be inserted on MIDI tracks (track ${t.id} is ${t.kind})`);
  }
}

function deviceCommand(ctx: ReducerContext, c: DeviceCommand): ReplyValue {
  const { tx } = ctx;
  switch (c.type) {
    case "Insert": {
      if (tx.get("Device", c.id)) fail("InvalidArgument", `device ${c.id} already exists`);
      const t = track(ctx, c.track);
      if (c.device.type === "Plugin") fail("Unsupported", "plugins are not available in the mock engine");
      const kind: Device["kind"] = { type: "Builtin", device: c.device.device };
      checkDeviceFits(t, kind);
      tx.upsert("Device", {
        id: c.id,
        track: t.id,
        order: keyForInsert(chainOf(ctx, t.id), c.before),
        name: builtinDescriptor(c.device.device).name,
        enabled: true,
        kind,
        params: defaultParams(c.device.device),
        sidechain: null,
        pad: null,
      });
      break;
    }
    case "Remove":
      device(ctx, c.id);
      deleteDeviceCascade(ctx, c.id);
      break;
    case "Move": {
      const d = device(ctx, c.id);
      checkDeviceMove(ctx, d, c.track);
      const t = track(ctx, c.track);
      checkDeviceFits(t, d.kind);
      if (c.before === d.id) break;
      // Param automation lanes target the device by id, so they follow it across tracks.
      tx.upsert("Device", { ...d, track: t.id, order: keyForInsert(chainOf(ctx, t.id, d.id), c.before) });
      break;
    }
    case "Duplicate": {
      const d = device(ctx, c.id);
      if (tx.get("Device", c.new_id)) fail("InvalidArgument", `device ${c.new_id} already exists`);
      const chain = duplicateSiblings(ctx, d, chainOf(ctx, d.track));
      const next = chain[chain.findIndex((x) => x.id === d.id) + 1];
      tx.upsert("Device", { ...d, id: c.new_id, order: keyBetween(d.order, next?.order ?? null) });
      copyRackPads(ctx, d.id, c.new_id, d.track);
      break;
    }
    case "Rename":
      tx.upsert("Device", { ...device(ctx, c.id), name: c.name });
      break;
    case "SetEnabled":
      tx.upsert("Device", { ...device(ctx, c.id), enabled: c.enabled });
      break;
    case "SetParam": {
      const d = device(ctx, c.device);
      const info = d.kind.type === "Builtin" ? builtinDescriptor(d.kind.device).params.find((p) => p.id === c.param) : undefined;
      if (d.kind.type === "Builtin" && !info) fail("NotFound", `param ${c.param} of device ${d.id}`);
      const value = info ? clampParam(info, c.value) : c.value;
      tx.upsert("Device", { ...d, params: { ...d.params, [c.param]: value } });
      break;
    }
    case "ResetParam": {
      const d = device(ctx, c.device);
      const params = { ...d.params };
      delete params[c.param];
      tx.upsert("Device", { ...d, params });
      break;
    }
    case "SetSample": {
      const d = device(ctx, c.device);
      if (d.kind.type !== "Builtin" || d.kind.device.type !== "Sampler") {
        fail("InvalidArgument", `device ${d.id} is not a sampler`);
      }
      if (c.media !== null && !tx.get("Media", c.media)) fail("NotFound", `media ${c.media}`);
      const slices = d.kind.device.type === "Sampler" ? d.kind.device.slices : { enabled: false, base_note: 36, markers: [] };
      tx.upsert("Device", { ...d, kind: { type: "Builtin", device: { type: "Sampler", sample: c.media, slices } } });
      break;
    }
    case "SetSidechain":
      setSidechain(ctx, c.device, c.source);
      break;
    case "SetZones":
      // v0.2 (`multisampler`, `roadmap/multisampler.ts`).
      setZones(ctx, c.device, c.zones);
      break;
    case "ListBuiltin":
      return { type: "DeviceTypes", devices: Object.values(BUILTIN_DESCRIPTORS) };
    case "GetDescriptor": {
      const d = device(ctx, c.device);
      if (d.kind.type !== "Builtin") fail("Unsupported", "plugins are not available in the mock engine");
      return { type: "Descriptor", descriptor: builtinDescriptor(d.kind.device) };
    }
  }
  return UNIT;
}

// ─── Clips ──────────────────────────────────────────────────────────────────────────────

function checkClipStart(start: Beats): void {
  if (!(start >= 0)) fail("InvalidArgument", "clip start must be >= 0");
}

function checkContentFits(t: Track, content: Clip["content"]): void {
  const want = content.type === "Midi" ? "Midi" : "Audio";
  if (t.kind !== want) fail("InvalidArgument", `${content.type} clips can't go on ${t.kind} track ${t.id}`);
}

/**
 * Make room for arrangement clip `keep` on its track, Ableton-style: clips it fully covers
 * are deleted, partially covered clips are trimmed, and a clip that contains it is split.
 */
function resolveOverlaps(ctx: ReducerContext, keep: Clip, ignore: ReadonlySet<ClipId>): void {
  const s = keep.start;
  const e = s + keep.length;
  for (const o of ctx.tx.all("Clip")) {
    if (o.id === keep.id || ignore.has(o.id) || o.track !== keep.track || (o.lane ?? null) !== (keep.lane ?? null)) continue;
    const os = o.start;
    const oe = os + o.length;
    if (oe <= s + EPS || os >= e - EPS) continue; // no overlap
    if (isCrossfade(keep, o)) continue; // crossfade overlap (clip-editing)
    if (os >= s - EPS && oe <= e + EPS) {
      deleteClipCascade(ctx, o.id); // fully covered
    } else if (os < s && oe > e) {
      // `keep` sits inside `o`: split `o` around it.
      copyClip(ctx, o, ctx.newId(), {
        start: e,
        length: oe - e,
        offset: o.offset + (e - os),
      });
      ctx.tx.upsert("Clip", { ...o, length: s - os });
    } else if (os < s) {
      ctx.tx.upsert("Clip", { ...o, length: s - os }); // trim right edge
    } else {
      ctx.tx.upsert("Clip", { ...o, start: e, length: oe - e, offset: o.offset + (e - os) });
    }
  }
}

function mediaLengthBeats(ctx: ReducerContext, media: { frames: number; sample_rate: number }, at: Beats): Beats {
  const seconds = media.frames / Math.max(1, media.sample_rate);
  return (seconds * bpmAt(ctx.tx.project, at)) / 60;
}

function clipCommand(ctx: ReducerContext, c: ClipCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "SetFadeCurves":
    case "SetReversed":
    case "Crossfade":
      clipV2Command(ctx, c);
      break;
    case "CreateMidi": {
      if (tx.get("Clip", c.id)) fail("InvalidArgument", `clip ${c.id} already exists`);
      const t = track(ctx, c.track);
      checkContentFits(t, { type: "Midi" });
      checkClipStart(c.start);
      if (!(c.length > 0)) fail("InvalidArgument", "clip length must be > 0");
      const created = makeClip({ id: c.id, track: t.id, start: c.start, length: c.length, name: c.name ?? "" });
      tx.upsert("Clip", created);
      resolveOverlaps(ctx, created, new Set());
      break;
    }
    case "CreateAudio": {
      if (tx.get("Clip", c.id)) fail("InvalidArgument", `clip ${c.id} already exists`);
      const t = track(ctx, c.track);
      const media = tx.get("Media", c.media) ?? fail("NotFound", `media ${c.media}`);
      checkContentFits(t, { type: "Audio" } as Clip["content"]);
      checkClipStart(c.start);
      const created = makeClip({
        id: c.id,
        track: t.id,
        start: c.start,
        length: mediaLengthBeats(ctx, media, c.start),
        name: media.name.replace(/\.[^.]+$/, ""),
        content: {
          type: "Audio",
          media: media.id,
          gain: 0,
          transpose: 0,
          fade_in: 0,
          fade_out: 0,
          fade_in_curve: { type: "Linear" },
          fade_out_curve: { type: "Linear" },
          reversed: false,
          // Unwarped (native speed), like the real controller.
          warp: { enabled: false, mode: "Repitch", source_bpm: null },
        },
      });
      tx.upsert("Clip", created);
      resolveOverlaps(ctx, created, new Set());
      break;
    }
    case "Delete":
      for (const id of c.ids) {
        clip(ctx, id);
        deleteClipCascade(ctx, id);
      }
      break;
    case "Move": {
      const moving = new Set(c.moves.map((m) => m.id));
      const moved: Clip[] = [];
      for (const m of c.moves) {
        const cl = clip(ctx, m.id);
        const t = track(ctx, m.track);
        checkContentFits(t, cl.content);
        checkClipStart(m.start);
        const next = { ...cl, track: t.id, start: m.start };
        moved.push(next);
      }
      for (const m of moved) tx.upsert("Clip", m);
      for (const m of moved) if (tx.get("Clip", m.id)) resolveOverlaps(ctx, tx.get("Clip", m.id)!, moving);
      break;
    }
    case "SetBounds": {
      const cl = clip(ctx, c.id);
      if (!(c.length > 0)) fail("InvalidArgument", "clip length must be > 0");
      if (!(c.offset >= 0)) fail("InvalidArgument", "clip offset must be >= 0");
      checkClipStart(c.start);
      const next = { ...cl, start: c.start, length: c.length, offset: c.offset };
      tx.upsert("Clip", next);
      resolveOverlaps(ctx, next, new Set());
      break;
    }
    case "Split": {
      const cl = clip(ctx, c.id);
      const start = cl.start;
      if (!(c.at > start + EPS && c.at < start + cl.length - EPS)) fail("InvalidArgument", "split point outside the clip");
      copyClip(ctx, cl, c.new_id, {
        start: c.at,
        length: start + cl.length - c.at,
        offset: cl.offset + (c.at - start),
      });
      tx.upsert("Clip", { ...cl, length: c.at - start });
      break;
    }
    case "Duplicate": {
      const cl = clip(ctx, c.id);
      const start = c.start ?? cl.start + cl.length;
      checkClipStart(start);
      const copy = copyClip(ctx, cl, c.new_id, { start });
      resolveOverlaps(ctx, copy, new Set());
      break;
    }
    case "Rename":
      tx.upsert("Clip", { ...clip(ctx, c.id), name: c.name });
      break;
    case "SetColor":
      tx.upsert("Clip", { ...clip(ctx, c.id), color: c.color });
      break;
    case "SetMuted":
      for (const id of c.ids) tx.upsert("Clip", { ...clip(ctx, id), muted: c.muted });
      break;
    case "SetLoop": {
      const cl = clip(ctx, c.id);
      if (!(c.looping.end > c.looping.start) || c.looping.start < 0) fail("InvalidArgument", "invalid loop region");
      tx.upsert("Clip", { ...cl, looping: c.looping });
      break;
    }
    case "SetGain": {
      const cl = clip(ctx, c.id);
      tx.upsert("Clip", { ...cl, content: { ...audioContent(cl), gain: clamp(c.gain, SILENCE_DB, 24) } });
      break;
    }
    case "SetTranspose": {
      const cl = clip(ctx, c.id);
      tx.upsert("Clip", { ...cl, content: { ...audioContent(cl), transpose: clamp(c.semitones, -48, 48) } });
      break;
    }
    case "SetFades": {
      const cl = clip(ctx, c.id);
      const max = cl.length;
      tx.upsert("Clip", {
        ...cl,
        content: { ...audioContent(cl), fade_in: clamp(c.fade_in, 0, max), fade_out: clamp(c.fade_out, 0, max) },
      });
      break;
    }
  }
}

// ─── Notes ──────────────────────────────────────────────────────────────────────────────

const pitchOk = (p: number) => Number.isInteger(p) && p >= 0 && p <= 127;

function note(ctx: ReducerContext, id: string): Note {
  return ctx.tx.get("Note", id) ?? fail("NotFound", `note ${id}`);
}

function noteCommand(ctx: ReducerContext, c: NoteCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "Add": {
      const cl = clip(ctx, c.clip);
      if (cl.content.type !== "Midi") fail("InvalidArgument", `clip ${cl.id} is not a MIDI clip`);
      for (const n of c.notes) {
        if (!pitchOk(n.pitch)) fail("InvalidArgument", `invalid pitch ${n.pitch}`);
        if (!(n.duration > 0) || n.start < 0) fail("InvalidArgument", "invalid note timing");
        tx.upsert("Note", {
          id: n.id,
          clip: cl.id,
          pitch: n.pitch,
          velocity: clamp(n.velocity, 0, 1),
          release_velocity: 0.5,
          start: n.start,
          duration: n.duration,
          muted: false,
        });
      }
      break;
    }
    case "Remove":
      for (const id of c.ids) {
        note(ctx, id);
        tx.remove("Note", id);
      }
      break;
    case "Edit":
      for (const e of c.edits) {
        const n = note(ctx, e.id);
        if (e.pitch !== null && !pitchOk(e.pitch)) fail("InvalidArgument", `invalid pitch ${e.pitch}`);
        if (e.duration !== null && !(e.duration > 0)) fail("InvalidArgument", "note duration must be > 0");
        tx.upsert("Note", {
          ...n,
          pitch: e.pitch ?? n.pitch,
          velocity: e.velocity !== null ? clamp(e.velocity, 0, 1) : n.velocity,
          start: e.start !== null ? Math.max(0, e.start) : n.start,
          duration: e.duration ?? n.duration,
          muted: e.muted ?? n.muted,
        });
      }
      break;
    case "Quantize": {
      clip(ctx, c.clip);
      if (!(c.grid > 0)) fail("InvalidArgument", "grid must be > 0");
      const strength = clamp(c.strength, 0, 1);
      const ids = c.notes ? new Set(c.notes) : null;
      const target = (t: number) => snapBeats(t, c.grid) + swingOffset(snapBeats(t, c.grid), c.grid, c.swing);
      const snap = (t: number) => t + (target(t) - t) * strength;
      for (const n of tx.all("Note")) {
        if (n.clip !== c.clip || (ids && !ids.has(n.id))) continue;
        const start = Math.max(0, snap(n.start));
        const end = c.ends ? snap(n.start + n.duration) : start + n.duration;
        tx.upsert("Note", { ...n, start, duration: Math.max(end - start, c.grid * 0.25) });
      }
      break;
    }
    case "Duplicate":
      for (const cp of c.copies) {
        const n = note(ctx, cp.from);
        tx.upsert("Note", {
          ...n,
          id: cp.new_id,
          start: Math.max(0, n.start + c.offset),
          pitch: clamp(n.pitch + c.transpose, 0, 127),
        });
      }
      break;
  }
}

// ─── Automation ─────────────────────────────────────────────────────────────────────────

function checkTarget(ctx: ReducerContext, target: AutomationTarget): void {
  switch (target.type) {
    case "TrackVolume":
    case "TrackPan":
      track(ctx, target.track);
      break;
    case "SendLevel":
      if (!ctx.tx.get("Send", target.send)) fail("NotFound", `send ${target.send}`);
      break;
    case "DeviceParam":
      device(ctx, target.device);
      break;
  }
}

function lane(ctx: ReducerContext, id: string): AutomationLane {
  return ctx.tx.get("AutomationLane", id) ?? fail("NotFound", `automation lane ${id}`);
}

function automationCommand(ctx: ReducerContext, c: AutomationCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "CreateLane": {
      if (tx.get("AutomationLane", c.id)) fail("InvalidArgument", `lane ${c.id} already exists`);
      if (c.owner.type === "Track") track(ctx, c.owner.track);
      else clip(ctx, c.owner.clip);
      checkTarget(ctx, c.target);
      const same = (l: AutomationLane) => JSON.stringify([l.owner, l.target]) === JSON.stringify([c.owner, c.target]);
      if (tx.all("AutomationLane").some(same)) fail("InvalidArgument", "a lane for this owner/target already exists");
      tx.upsert("AutomationLane", { id: c.id, owner: c.owner, target: c.target, enabled: true });
      break;
    }
    case "DeleteLane":
      lane(ctx, c.id);
      deleteLane(ctx, c.id);
      break;
    case "SetLaneEnabled":
      tx.upsert("AutomationLane", { ...lane(ctx, c.id), enabled: c.enabled });
      break;
    case "AddPoints":
      lane(ctx, c.lane);
      for (const p of c.points) {
        if (p.time < 0) fail("InvalidArgument", "point time must be >= 0");
        tx.upsert("AutomationPoint", { id: p.id, lane: c.lane, time: p.time, value: clamp(p.value, 0, 1), curve: p.curve });
      }
      break;
    case "RemovePoints":
      for (const id of c.ids) {
        if (!tx.get("AutomationPoint", id)) fail("NotFound", `automation point ${id}`);
        tx.remove("AutomationPoint", id);
      }
      break;
    case "EditPoints":
      for (const e of c.edits) {
        const p = tx.get("AutomationPoint", e.id) ?? fail("NotFound", `automation point ${e.id}`);
        tx.upsert("AutomationPoint", {
          ...p,
          time: e.time !== null ? Math.max(0, e.time) : p.time,
          value: e.value !== null ? clamp(e.value, 0, 1) : p.value,
          curve: e.curve ?? p.curve,
        });
      }
      break;
    case "ClearRange":
      lane(ctx, c.lane);
      for (const p of tx.all("AutomationPoint")) {
        if (p.lane === c.lane && p.time >= c.start && p.time < c.end) tx.remove("AutomationPoint", p.id);
      }
      break;
  }
}

function validSignature(sig: TimeSignature): boolean {
  return Number.isInteger(sig.numerator) && sig.numerator >= 1 && sig.numerator <= 99 && [1, 2, 4, 8, 16, 32].includes(sig.denominator);
}

// ─── Settings-like commands ─────────────────────────────────────────────────────────────

function transportSettingsCommand(ctx: ReducerContext, c: TransportCommand): void {
  const { tx } = ctx;
  const settings = tx.project.settings;
  switch (c.type) {
    case "SetLoopEnabled":
      tx.setSettings({ ...settings, loop_enabled: c.enabled });
      break;
    case "SetLoopRegion":
      if (!(c.region.start >= 0 && c.region.end > c.region.start)) fail("InvalidArgument", "invalid loop region");
      tx.setSettings({ ...settings, loop_region: c.region });
      break;
    case "SetTempo": {
      const p = tempoPointAt(tx.project, ctx.position) ?? fail("InvalidState", "no tempo point");
      tx.upsert("TempoPoint", { ...p, bpm: clamp(c.bpm, 20, 999) });
      break;
    }
    case "SetTimeSignature": {
      if (!validSignature(c.signature)) fail("InvalidArgument", "invalid time signature");
      const p = signaturePointAt(tx.project, ctx.position) ?? fail("InvalidState", "no time signature point");
      tx.upsert("TimeSignature", { ...p, signature: c.signature });
      break;
    }
    case "SetMetronome":
      tx.setSettings({ ...settings, metronome: c.enabled });
      break;
    default:
      fail("Internal", `not a document transport command: ${c.type}`);
  }
}

function projectCommand(ctx: ReducerContext, c: ProjectCommand): void {
  if (c.type === "SetScale") {
    if (!Number.isInteger(c.scale.root) || c.scale.root < 0 || c.scale.root > 11) fail("InvalidArgument", "invalid scale root");
    ctx.tx.setSettings({ ...ctx.tx.project.settings, scale: c.scale });
    return;
  }
  if (c.type !== "Rename") fail("Internal", `not a document project command: ${c.type}`);
  if (c.id !== ctx.tx.project.id) fail("InvalidArgument", "only the current project can be renamed as a document edit");
  const name = c.name.trim();
  if (!name) fail("InvalidArgument", "project name must not be empty");
  ctx.tx.setSettings({ ...ctx.tx.project.settings, name });
}

function recordingCommand(ctx: ReducerContext, c: RecordingCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "SetMonitor":
      tx.upsert("Track", { ...track(ctx, c.track), monitor: c.monitor });
      break;
    case "SetInput":
      if (c.input.type === "Track") track(ctx, c.input.track);
      tx.upsert("Track", { ...track(ctx, c.track), input: c.input });
      break;
    case "SetCountIn":
      tx.setSettings({ ...tx.project.settings, count_in_bars: Math.max(0, Math.round(c.bars)) });
      break;
    default:
      fail("Internal", `not a document recording command: ${c.type}`);
  }
}

function warpCommand(ctx: ReducerContext, c: WarpCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "SetWarp": {
      const cl = clip(ctx, c.clip);
      tx.upsert("Clip", { ...cl, content: { ...audioContent(cl), warp: c.warp } });
      break;
    }
    case "AddMarker":
      audioContent(clip(ctx, c.clip));
      if (tx.get("WarpMarker", c.id)) fail("InvalidArgument", `warp marker ${c.id} already exists`);
      tx.upsert("WarpMarker", { id: c.id, clip: c.clip, beat: c.beat, source: Math.max(0, c.source) });
      break;
    case "MoveMarker": {
      const m = tx.get("WarpMarker", c.id) ?? fail("NotFound", `warp marker ${c.id}`);
      tx.upsert("WarpMarker", { ...m, beat: c.beat, source: Math.max(0, c.source) });
      break;
    }
    case "RemoveMarker":
      if (!tx.get("WarpMarker", c.id)) fail("NotFound", `warp marker ${c.id}`);
      tx.remove("WarpMarker", c.id);
      break;
    default:
      fail("Internal", `not a document warp command: ${c.type}`);
  }
}

/** Human-readable undo label for a command ("SetVolume" → "Set Volume"). */
export function labelOf(command: Command): string {
  if (command.domain === "Edit" && command.command.type === "Batch") return command.command.label;
  return command.command.type.replace(/([a-z])([A-Z])/g, "$1 $2");
}

