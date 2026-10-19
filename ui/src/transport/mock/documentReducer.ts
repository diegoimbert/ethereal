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
  DeviceCategory,
  DeviceCommand,
  DeviceDescriptor,
  ErrorCode,
  MixerCommand,
  Note,
  NoteCommand,
  ProjectCommand,
  RecordingCommand,
  ReplyValue,
  Track,
  TrackCommand,
  TrackId,
  TransportCommand,
  WarpCommand,
  WarpSettings,
} from "@/generated";
import { BEATS_EPSILON, snapBeats } from "@/state/beats";
import { keyBetween, keyForInsert } from "@/state/orderKey";
import { CommandFailedError } from "../EngineTransport";
import { BUILTIN_DESCRIPTORS, builtinDescriptor, clampParam, missingDescriptor } from "./builtinDevices";
import { mockPluginDescriptor, mockPluginInstance } from "./plugins";
import { defaultParams, defaultTrackName, makeClip, makeTrack, MOCK_TRACK_COLORS } from "./demoProject";
import { isRoadmapDocumentCommand, reduceRoadmapCommand } from "./roadmap";
import { groupsTrackCommand } from "./roadmap/groupsBuses";
import { setZones } from "./roadmap/multisampler";
import { listFactoryIrs, setIr } from "./roadmap/fxSpace";
import { clipV2Command, isCrossfade } from "./roadmap/clipEditing";
import { setTranspose } from "./roadmap/transpose";
import { onTrackDeletedTakes } from "./roadmap/comping";
import { checkDeviceMove, copyRackPads, onRackDeleted } from "./roadmap/drumRack";
import { addFirstChain, copyTrackRackExtras, onDeviceDeletedRacks } from "./roadmap/racksModulation";
import { swingOffset } from "./roadmap/groove";
import {
  chainDevicesOf,
  checkChainOrder,
  childTracksOf,
  clamp,
  detectBpm,
  onDeviceDeleted,
  onSendDeleted,
  onTrackDeleted,
  padChainOf,
  targetDevice,
  trackChainOf,
  validSignature,
} from "./roadmap/shared";
import { setSidechain } from "./roadmap/sidechain";
import { copyClipExpression, copyNoteExpressions, deleteClipExpression, deleteNoteExpressions } from "./roadmap/expression";
import { reverseNotes } from "./roadmap/reverse";
import { bpmAt, tempoPointAt, signaturePointAt } from "./tempo";
import type { Tx } from "./tx";

export interface ReducerContext {
  tx: Tx;
  /** Id generator for entities the engine creates itself (children of duplicates, ...). */
  newId: () => string;
  /** Current playhead position (for "tempo at the playhead" commands). */
  position: Beats;
  /** A warning of the edit (sent as a `Notification` once it commits; `ctx.warnings` in Rust). */
  warn?: (message: string) => void;
}

export function fail(code: ErrorCode, message: string): never {
  throw new CommandFailedError({ code, message });
}

const EPS = BEATS_EPSILON;
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
      return c.type !== "ListBuiltin" && c.type !== "GetDescriptor" && c.type !== "ListFactoryIrs";
    case "Project":
      // Only renaming the *current* project is a document edit (checked in the reducer).
      return c.type === "Rename" || c.type === "SetScale";
    case "Transport":
      return ["SetLoopEnabled", "SetLoopRegion", "SetTempo", "SetTimeSignature", "SetMetronome"].includes(c.type);
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

function descendants(ctx: ReducerContext, id: TrackId): Track[] {
  const out: Track[] = [];
  for (const t of ctx.tx.all("Track")) {
    if (t.parent === id) out.push(t, ...descendants(ctx, t.id));
  }
  return out;
}

// ─── Cascading deletes / deep copies ────────────────────────────────────────────────────

function deleteLane(tx: Tx, lane: string): void {
  for (const p of tx.all("AutomationPoint")) if (p.lane === lane) tx.remove("AutomationPoint", p.id);
  tx.remove("AutomationLane", lane);
}

function deleteLanesWhere(tx: Tx, pred: (l: AutomationLane) => boolean): void {
  for (const l of tx.all("AutomationLane")) if (pred(l)) deleteLane(tx, l.id);
}

export function deleteClipCascade(tx: Tx, id: ClipId): void {
  // v0.3 (midi-expression): lanes and note expressions go with the clip.
  deleteNoteExpressions(
    tx,
    tx
      .all("Note")
      .filter((n) => n.clip === id)
      .map((n) => n.id),
  );
  deleteClipExpression(tx, id);
  for (const n of tx.all("Note")) if (n.clip === id) tx.remove("Note", n.id);
  for (const m of tx.all("WarpMarker")) if (m.clip === id) tx.remove("WarpMarker", m.id);
  deleteLanesWhere(tx, (l) => l.owner.type === "Clip" && l.owner.clip === id);
  tx.remove("Clip", id);
}

function deleteSendCascade(ctx: ReducerContext, id: string): void {
  deleteLanesWhere(ctx.tx, (l) => l.target.type === "SendLevel" && l.target.send === id);
  onSendDeleted(ctx, id);
  ctx.tx.remove("Send", id);
}

function deleteDeviceCascade(ctx: ReducerContext, id: string): void {
  deleteLanesWhere(ctx.tx, (l) => targetDevice(l.target) === id);
  onDeviceDeleted(ctx, id);
  // A drum rack takes its pads (and their chains) with it.
  onRackDeleted(ctx, id, (d) => deleteDeviceCascade(ctx, d));
  onDeviceDeletedRacks(ctx, id, (d) => deleteDeviceCascade(ctx, d));
  ctx.tx.remove("Device", id);
}

function deleteTrackCascade(ctx: ReducerContext, id: TrackId): void {
  for (const c of ctx.tx.all("Clip")) if (c.track === id) deleteClipCascade(ctx.tx, c.id);
  // Pad devices go with their rack.
  for (const d of ctx.tx.all("Device")) if (d.track === id && d.pad === null) deleteDeviceCascade(ctx, d.id);
  onTrackDeleted(ctx, id);
  onTrackDeletedTakes(ctx, id);
  for (const s of ctx.tx.all("Send")) if (s.from === id || s.to === id) deleteSendCascade(ctx, s.id);
  deleteLanesWhere(
    ctx.tx,
    (l) => (l.owner.type === "Track" && l.owner.track === id) || ((l.target.type === "TrackVolume" || l.target.type === "TrackPan") && l.target.track === id),
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
function copyClip(
  ctx: ReducerContext,
  src: Clip,
  newId: ClipId,
  overrides: Partial<Clip>,
  /** Clip envelopes' targets on the copy (a duplicated track: its own devices); null drops one. */
  retarget: (t: AutomationTarget) => AutomationTarget | null = (t) => t,
): Clip {
  if (ctx.tx.get("Clip", newId)) fail("InvalidArgument", `clip ${newId} already exists`);
  const copy: Clip = { ...src, ...overrides, id: newId };
  ctx.tx.upsert("Clip", copy);
  for (const n of ctx.tx.all("Note")) {
    if (n.clip !== src.id) continue;
    const id = ctx.newId();
    ctx.tx.upsert("Note", { ...n, id, clip: newId });
    copyNoteExpressions(ctx, n.id, id);
  }
  copyClipExpression(ctx, src.id, newId);
  for (const m of ctx.tx.all("WarpMarker")) {
    if (m.clip === src.id) ctx.tx.upsert("WarpMarker", { ...m, id: ctx.newId(), clip: newId });
  }
  for (const l of ctx.tx.all("AutomationLane")) {
    if (l.owner.type === "Clip" && l.owner.clip === src.id) {
      const target = retarget(l.target);
      if (target) copyLane(ctx, l, { ...l, id: ctx.newId(), owner: { type: "Clip", clip: newId }, target });
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
  const sibs = childTracksOf(ctx.tx.project, null, except);
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

/** A return's explicit `before` must be another return or master (returns stay together). */
function validateReturnBefore(ctx: ReducerContext, kind: Track["kind"], before: TrackId | null): void {
  if (before === null || kind !== "Return") return;
  const k = track(ctx, before).kind;
  if (k !== "Return" && k !== "Master") fail("InvalidArgument", "a return track can only be placed among the returns");
}

function trackCommand(ctx: ReducerContext, c: TrackCommand): void {
  // v0.2 (`groups-buses`, `roadmap/groupsBuses.ts`).
  if (groupsTrackCommand(ctx, c, (id) => deleteTrackCascade(ctx, id))) return;
  if (c.type === "SetScale") {
    const t = track(ctx, c.id);
    if (t.kind !== "Midi") fail("InvalidArgument", "track scales are only available on MIDI tracks");
    if (c.scale.type === "Custom" && (!Number.isInteger(c.scale.scale.root) || c.scale.scale.root < 0 || c.scale.scale.root > 11))
      fail("InvalidArgument", "invalid scale root");
    ctx.tx.upsert("Track", { ...t, scale: c.scale });
    return;
  }
  const { tx } = ctx;
  switch (c.type) {
    case "Create": {
      if (tx.get("Track", c.id)) break; // already created (a retry): no-op, like the engine
      if (c.kind === "Master") fail("InvalidArgument", "there is exactly one master track");
      validateParent(ctx, c.kind, c.parent);
      validateReturnBefore(ctx, c.kind, c.before);
      const before = c.before ?? defaultBefore(ctx, c.kind, c.parent);
      const all = tx.all("Track");
      const sameKind = all.filter((t) => t.kind === c.kind).length;
      tx.upsert(
        "Track",
        makeTrack({
          id: c.id,
          kind: c.kind,
          name: c.name?.trim() ? c.name : defaultTrackName(c.kind, sameKind + 1),
          color: c.color ?? MOCK_TRACK_COLORS[all.length % MOCK_TRACK_COLORS.length]!,
          order: keyForInsert(childTracksOf(ctx.tx.project, c.parent), before),
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
      if (tx.get("Track", c.new_id)) break; // already duplicated: no-op
      const sibs = childTracksOf(ctx.tx.project, t.parent);
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
      validateReturnBefore(ctx, t.kind, c.before);
      const before = c.before ?? (t.kind === "Master" ? null : defaultBefore(ctx, t.kind, c.parent, t.id));
      tx.upsert("Track", { ...t, parent: c.parent, order: keyForInsert(childTracksOf(ctx.tx.project, c.parent, t.id), before) });
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
    if (d.track !== t.id || d.pad !== null || d.chain != null) continue;
    const id = ctx.newId();
    deviceIds.set(d.id, id);
    tx.upsert("Device", { ...d, id, track: newId });
    copyRackPads(ctx, d.id, id, newId, deviceIds);
  }
  copyTrackRackExtras(ctx, deviceIds);
  for (const s of tx.all("Send")) {
    if (s.from !== t.id) continue;
    const id = ctx.newId();
    sendIds.set(s.id, id);
    tx.upsert("Send", { ...s, id, from: newId });
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
      case "DeviceParam":
      case "DeviceEnabled": {
        const dev = deviceIds.get(target.device);
        return dev ? { ...target, device: dev } : null;
      }
      case "ClipGain":
      case "ClipTranspose":
        return null; // clip envelopes only, never on a track lane
    }
  };
  // Clip envelopes follow their clip onto the copy, retargeted like the track's lanes (clip
  // gain/transpose stay: they are the clip's own).
  const clipRemap = (target: AutomationTarget) => (target.type === "ClipGain" || target.type === "ClipTranspose" ? target : remap(target));
  for (const cl of tx.all("Clip")) {
    if (cl.track === t.id) copyClip(ctx, cl, ctx.newId(), { track: newId }, clipRemap);
  }
  for (const l of tx.all("AutomationLane")) {
    if (l.owner.type !== "Track" || l.owner.track !== t.id) continue;
    const target = remap(l.target);
    if (target) copyLane(ctx, l, { ...l, id: ctx.newId(), owner: { type: "Track", track: newId }, target });
  }
  // Children of a group, in order.
  const kids = childTracksOf(ctx.tx.project, t.id);
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
      tx.upsert("Track", { ...t, mixer: { ...t.mixer, volume: level(c.volume, MAX_DB) } });
      break;
    }
    case "SetPan": {
      const t = track(ctx, c.track);
      if (Number.isNaN(c.pan)) fail("InvalidArgument", "pan must be a number");
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
      if (tx.get("Send", c.id)) break; // already created: no-op
      const from = track(ctx, c.from);
      const to = track(ctx, c.to);
      if (to.kind !== "Return") fail("InvalidArgument", "sends must target a return track");
      if (from.id === to.id || from.kind === "Master") fail("InvalidArgument", "invalid send source");
      if (tx.all("Send").some((s) => s.from === from.id && s.to === to.id)) {
        break; // a second send on the same route (double-sent keystroke): no-op
      }
      tx.upsert("Send", {
        id: c.id,
        from: from.id,
        to: to.id,
        level: level(c.level, MAX_DB),
        pre_fader: c.pre_fader,
      });
      break;
    }
    case "SetSendLevel": {
      const s = tx.get("Send", c.send) ?? fail("NotFound", `send ${c.send}`);
      tx.upsert("Send", { ...s, level: level(c.level, MAX_DB) });
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

/** The descriptor of a device (built-in, or one of the mock plugins); `null` when unknown. */
function descriptorOf(kind: Device["kind"]): DeviceDescriptor | null {
  if (kind.type === "Builtin") return builtinDescriptor(kind.device);
  if (kind.type === "Missing") return missingDescriptor(kind.missing);
  return mockPluginDescriptor(kind.plugin.plugin_id);
}

/** `doc/devices.rs::check_fits`: instruments and note effects only go on MIDI tracks. */
function checkDeviceFits(t: Track, category: DeviceCategory | null): void {
  if (category === "Instrument" && t.kind !== "Midi") {
    fail("InvalidArgument", `instruments can only be inserted on MIDI tracks (track ${t.id} is ${t.kind})`);
  }
  if (category === "NoteEffect" && t.kind !== "Midi") fail("InvalidArgument", "note effects can only be inserted on MIDI tracks");
}

function deviceCommand(ctx: ReducerContext, c: DeviceCommand): ReplyValue {
  const value = deviceCommandInner(ctx, c);
  if (c.type === "Insert" || c.type === "Move") checkChainOrder(ctx.tx.project, c.track);
  return value;
}

function deviceCommandInner(ctx: ReducerContext, c: DeviceCommand): ReplyValue {
  const { tx } = ctx;
  switch (c.type) {
    case "Insert": {
      if (tx.get("Device", c.id)) break; // already created: no-op
      const t = track(ctx, c.track);
      if (c.device.type === "Plugin") {
        // Only the fake plugins of `./plugins.ts` (e.g. the 10,000-param "Mock Mega").
        const desc = mockPluginDescriptor(c.device.plugin_id);
        if (!desc) fail("Unsupported", "plugins are not available in the mock engine");
        checkDeviceFits(t, desc.category);
        tx.upsert("Device", {
          id: c.id,
          track: t.id,
          order: keyForInsert(trackChainOf(ctx.tx.project, t.id), c.before),
          name: desc.name,
          enabled: true,
          kind: { type: "Plugin", plugin: mockPluginInstance(c.device.plugin_id, c.device.format ?? "Clap") },
          params: {},
          sidechain: null,
          pad: null,
        });
        break;
      }
      const kind: Device["kind"] = { type: "Builtin", device: c.device.device };
      checkDeviceFits(t, builtinDescriptor(c.device.device).category);
      tx.upsert("Device", {
        id: c.id,
        track: t.id,
        order: keyForInsert(trackChainOf(ctx.tx.project, t.id), c.before),
        // Default names are lowercase (the engine's too); a plugin keeps its vendor's name.
        name: builtinDescriptor(c.device.device).name.toLowerCase(),
        enabled: true,
        kind,
        params: defaultParams(c.device.device),
        sidechain: null,
        pad: null,
      });
      // A fresh audio / MIDI effect rack starts with "chain 1" (`racks::add_first_chain`).
      addFirstChain(ctx, c.id, c.device.device.type);
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
      checkDeviceFits(t, descriptorOf(d.kind)?.category ?? null);
      if (c.before === d.id) break;
      // Track lanes target the device by id, so they follow it across tracks; its clip
      // envelopes on the previous track can't (the clips stay there).
      tx.upsert("Device", { ...d, track: t.id, order: keyForInsert(trackChainOf(ctx.tx.project, t.id, d.id), c.before) });
      if (d.track !== t.id) {
        const dead = tx
          .all("AutomationLane")
          .filter((l) => targetDevice(l.target) === d.id && l.owner.type === "Clip" && tx.get("Clip", l.owner.clip)?.track !== t.id);
        for (const l of dead) deleteLane(tx, l.id);
        if (dead.length > 0) ctx.warn?.(`"${d.name}": clip envelopes on its previous track were removed`);
      }
      break;
    }
    case "Duplicate": {
      const d = device(ctx, c.id);
      if (tx.get("Device", c.new_id)) break; // already duplicated: no-op
      const p = tx.project;
      if (c.track != null && c.track !== d.track) {
        // Onto another track (#582): at the end of its chain, with its rack chains / pads there.
        const t = track(ctx, c.track);
        checkDeviceFits(t, descriptorOf(d.kind)?.category ?? null);
        const last = trackChainOf(p, t.id).at(-1);
        tx.upsert("Device", { ...d, id: c.new_id, track: t.id, pad: null, chain: undefined, order: keyBetween(last?.order ?? null, null) });
        const ids = new Map([[d.id, c.new_id]]);
        copyRackPads(ctx, d.id, c.new_id, t.id, ids);
        copyTrackRackExtras(ctx, ids);
        break;
      }
      // Placed among its pad chain, its rack chain or its track chain.
      const chain = d.pad !== null ? padChainOf(p, d.pad) : d.chain != null ? chainDevicesOf(p, d.chain) : trackChainOf(p, d.track);
      const next = chain[chain.findIndex((x) => x.id === d.id) + 1];
      tx.upsert("Device", { ...d, id: c.new_id, order: keyBetween(d.order, next?.order ?? null) });
      // A drum rack with its pads; every copied rack (this one, racks on the copied pads)
      // with its chains, modulators and mappings.
      const ids = new Map([[d.id, c.new_id]]);
      copyRackPads(ctx, d.id, c.new_id, d.track, ids);
      copyTrackRackExtras(ctx, ids);
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
      if (!Number.isFinite(c.value)) fail("InvalidArgument", "param values must be finite");
      const desc = descriptorOf(d.kind);
      const info = desc?.params.find((p) => p.id === c.param);
      // A plugin that isn't instantiated (no descriptor) keeps the mirror value as given; a missing
      // device keeps any param it is sent (its source params are only partly known).
      if (desc && !info && d.kind.type !== "Missing") fail("NotFound", `param ${c.param} of device ${d.id}`);
      const value = info ? clampParam(info, c.value) : c.value;
      tx.upsert("Device", { ...d, params: { ...d.params, [c.param]: value } });
      break;
    }
    case "ResetParam": {
      // Reset = store the default explicitly (built-ins) or drop the mirror entry.
      const d = device(ctx, c.device);
      const info = descriptorOf(d.kind)?.params.find((p) => p.id === c.param);
      if (d.kind.type === "Builtin" && !info) fail("NotFound", `param ${c.param}`);
      const params = { ...d.params };
      if (info) params[c.param] = info.default;
      else delete params[c.param];
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
      setSidechain(ctx, c.device, c.source, c.tap ?? null);
      break;
    case "SetZones":
      // v0.2 (`multisampler`, `roadmap/multisampler.ts`).
      setZones(ctx, c.device, c.zones);
      break;
    case "SetIr":
      // v0.3 (`fx-space`, `roadmap/fxSpace.ts`).
      setIr(ctx, c.device, c.ir);
      break;
    case "SetLfoCurve": {
      // `fx-modulation`: the Volume LFO's drawn cycle, sanitized like Rust `LfoCurve::sanitized`.
      const d = device(ctx, c.device);
      if (d.kind.type !== "Builtin" || d.kind.device.type !== "VolumeLfo") {
        fail("InvalidArgument", `device ${d.id} is not a volume lfo`);
      }
      const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
      const points = c.curve.points
        .filter((p) => Number.isFinite(p.x) && Number.isFinite(p.y))
        .map((p) => ({
          x: clamp01(p.x),
          y: clamp01(p.y),
          curve: p.curve.type === "Curve" ? { type: "Curve" as const, tension: Math.min(1, Math.max(-1, p.curve.tension)) } : p.curve,
        }))
        .sort((a, b) => a.x - b.x)
        .slice(0, 64);
      if (points.length === 0) points.push({ x: 0, y: 1, curve: { type: "Linear" } });
      tx.upsert("Device", { ...d, kind: { type: "Builtin", device: { type: "VolumeLfo", curve: { points } } } });
      break;
    }
    case "ListFactoryIrs":
      return listFactoryIrs();
    case "ListBuiltin":
      return { type: "DeviceTypes", devices: Object.values(BUILTIN_DESCRIPTORS) };
    case "GetDescriptor": {
      const d = device(ctx, c.device);
      if (d.kind.type === "Missing") return { type: "Descriptor", descriptor: missingDescriptor(d.kind.missing) };
      if (d.kind.type !== "Builtin") {
        const desc = mockPluginDescriptor(d.kind.plugin.plugin_id);
        if (!desc) fail("Unsupported", "plugins are not available in the mock engine");
        return { type: "Descriptor", descriptor: desc };
      }
      return { type: "Descriptor", descriptor: builtinDescriptor(d.kind.device) };
    }
  }
  return UNIT;
}

// ─── Clips ──────────────────────────────────────────────────────────────────────────────

/** `mixer.rs::db`: NaN is rejected, ±∞ clamps. */
function level(v: number, max: number): number {
  if (Number.isNaN(v)) fail("InvalidArgument", "level must be a number");
  return clamp(v, SILENCE_DB, max);
}

/** Fails unless `v` is finite (the controller's `is_finite` checks). */
function finite(v: number, message: string): number {
  if (!Number.isFinite(v)) fail("InvalidArgument", message);
  return v;
}

const velocity = (v: number): number => clamp(finite(v, "velocity must be finite"), 0, 1);
const pointTime = (t: number): number => Math.max(0, finite(t, "automation times must be finite"));
const pointValue = (v: number): number => clamp(finite(v, "automation values must be finite"), 0, 1);

function checkClipStart(start: Beats): void {
  if (!(Number.isFinite(start) && start >= 0)) fail("InvalidArgument", "clip start must be >= 0");
}

function checkClipLength(length: Beats): void {
  if (!(Number.isFinite(length) && length > 0)) fail("InvalidArgument", "clip length must be > 0");
}

/** `clips.rs::strip_extension`: a leading dot (".wav") is a name, not an extension. */
function stripExtension(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(0, i) : name;
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
      deleteClipCascade(ctx.tx, o.id); // fully covered
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
  return Math.max((seconds * bpmAt(ctx.tx.project, at)) / 60, BEATS_EPSILON * 10);
}

function clipCommand(ctx: ReducerContext, c: ClipCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "SetFadeCurves":
    case "SetReversed":
    case "Crossfade":
    case "SetCrossfade":
      clipV2Command(ctx, c);
      break;
    case "CreateMidi": {
      if (tx.get("Clip", c.id)) break; // already created: no-op
      const t = track(ctx, c.track);
      checkContentFits(t, { type: "Midi" });
      checkClipStart(c.start);
      checkClipLength(c.length);
      const created = makeClip({ id: c.id, track: t.id, start: c.start, length: c.length, name: c.name ?? "" });
      tx.upsert("Clip", created);
      resolveOverlaps(ctx, created, new Set());
      break;
    }
    case "CreateAudio": {
      if (tx.get("Clip", c.id)) break; // already created: no-op
      const t = track(ctx, c.track);
      const media = tx.get("Media", c.media) ?? fail("NotFound", `media ${c.media}`);
      if (media.frames === 0) fail("InvalidState", "the media is still loading (its length is not known yet)");
      checkClipStart(c.start);
      checkContentFits(t, { type: "Audio" } as Clip["content"]);
      const created = makeClip({
        id: c.id,
        track: t.id,
        start: c.start,
        length: mediaLengthBeats(ctx, media, c.start),
        name: stripExtension(media.name),
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
        deleteClipCascade(ctx.tx, id);
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
      for (const m of moved) {
        const from = tx.get("Clip", m.id)!.track;
        tx.upsert("Clip", m);
        if (from !== m.track) retargetClipLanes(ctx, m.id, from, m.track);
      }
      for (const m of moved) if (tx.get("Clip", m.id)) resolveOverlaps(ctx, tx.get("Clip", m.id)!, moving);
      break;
    }
    case "SetBounds": {
      const cl = clip(ctx, c.id);
      if (!(c.length > 0)) fail("InvalidArgument", "clip length must be > 0");
      // Negative = a leading gap before the content (base-142).
      if (!Number.isFinite(c.offset)) fail("InvalidArgument", "clip offset must be finite");
      checkClipStart(c.start);
      const next = { ...cl, start: c.start, length: c.length, offset: c.offset };
      tx.upsert("Clip", next);
      resolveOverlaps(ctx, next, new Set());
      break;
    }
    case "Split": {
      if (tx.get("Clip", c.new_id)) break; // already split: no-op
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
      if (tx.get("Clip", c.new_id)) break; // already duplicated: no-op
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
      tx.upsert("Clip", { ...cl, content: { ...audioContent(cl), gain: clamp(finite(c.gain, "gain must be finite"), SILENCE_DB, 24) } });
      break;
    }
    case "SetTranspose":
      tx.upsert("Clip", setTranspose(ctx, clip(ctx, c.id), c.semitones));
      break;
    case "SetFades": {
      const cl = clip(ctx, c.id);
      const max = cl.length;
      tx.upsert("Clip", {
        ...cl,
        content: {
          ...audioContent(cl),
          fade_in: clamp(finite(c.fade_in, "fade lengths must be finite"), 0, max),
          fade_out: clamp(finite(c.fade_out, "fade lengths must be finite"), 0, max),
        },
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

/** Earliest note start in a clip: 0, or its start when extended left (a leading gap; `ether_model::clip`). */
function noteFloor(ctx: ReducerContext, clipId: string): number {
  const cl = ctx.tx.get("Clip", clipId);
  return cl ? Math.min(0, cl.offset) : 0;
}

function noteCommand(ctx: ReducerContext, c: NoteCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "Add": {
      const cl = clip(ctx, c.clip);
      if (cl.content.type !== "Midi") fail("InvalidArgument", `clip ${cl.id} is not a MIDI clip`);
      for (const n of c.notes) {
        if (tx.get("Note", n.id)) continue; // already added (a retry): kept as is
        if (!pitchOk(n.pitch)) fail("InvalidArgument", `invalid pitch ${n.pitch}`);
        if (!(n.duration > 0) || !(n.start >= Math.min(0, cl.offset) - 1e-6)) fail("InvalidArgument", "invalid note timing");
        tx.upsert("Note", {
          id: n.id,
          clip: cl.id,
          pitch: n.pitch,
          velocity: velocity(n.velocity),
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
        deleteNoteExpressions(tx, [id]);
        tx.remove("Note", id);
      }
      break;
    case "Edit":
      for (const e of c.edits) {
        const n = note(ctx, e.id);
        if (e.pitch !== null && !pitchOk(e.pitch)) fail("InvalidArgument", `invalid pitch ${e.pitch}`);
        if (e.duration !== null && !(Number.isFinite(e.duration) && e.duration > 0)) fail("InvalidArgument", "note duration must be > 0");
        tx.upsert("Note", {
          ...n,
          pitch: e.pitch ?? n.pitch,
          velocity: e.velocity !== null ? velocity(e.velocity) : n.velocity,
          start: e.start !== null ? Math.max(noteFloor(ctx, n.clip), finite(e.start, "note start must be finite")) : n.start,
          duration: e.duration ?? n.duration,
          muted: e.muted ?? n.muted,
        });
      }
      break;
    case "Reverse":
      reverseNotes(ctx, c.clip, c.notes);
      break;
    case "Quantize": {
      clip(ctx, c.clip);
      if (!(Number.isFinite(c.grid) && c.grid > 0)) fail("InvalidArgument", "grid must be > 0");
      const strength = Number.isFinite(c.strength) ? clamp(c.strength, 0, 1) : 1;
      const ids = c.notes ? new Set(c.notes) : null;
      const target = (t: number) => snapBeats(t, c.grid) + swingOffset(snapBeats(t, c.grid), c.grid, c.swing);
      const snap = (t: number) => t + (target(t) - t) * strength;
      for (const n of tx.all("Note")) {
        if (n.clip !== c.clip || (ids && !ids.has(n.id))) continue;
        const start = Math.max(Math.min(noteFloor(ctx, n.clip), n.start), snap(n.start));
        const end = c.ends ? snap(n.start + n.duration) : start + n.duration;
        tx.upsert("Note", { ...n, start, duration: Math.max(end - start, c.grid * 0.25) });
      }
      break;
    }
    case "Duplicate":
      for (const cp of c.copies) {
        if (tx.get("Note", cp.new_id)) continue; // already duplicated: kept as is
        const n = note(ctx, cp.from);
        tx.upsert("Note", {
          ...n,
          id: cp.new_id,
          start: Math.max(noteFloor(ctx, n.clip), n.start + c.offset),
          pitch: clamp(n.pitch + c.transpose, 0, 127),
        });
        copyNoteExpressions(ctx, cp.from, cp.new_id);
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
    case "DeviceEnabled":
      device(ctx, target.device);
      break;
    case "ClipGain":
    case "ClipTranspose":
      break; // checked against the owner by `CreateLane`
  }
}

/** `DocCtx::target_track`: the track a target belongs to (`null` = dangling). */
function targetTrack(tx: Tx, target: AutomationTarget): TrackId | null {
  switch (target.type) {
    case "TrackVolume":
    case "TrackPan":
      return target.track;
    case "SendLevel":
      return tx.get("Send", target.send)?.from ?? null;
    case "DeviceParam":
    case "DeviceEnabled":
      return tx.get("Device", target.device)?.track ?? null;
    case "ClipGain":
    case "ClipTranspose":
      return null; // the owning clip's own property: no track of its own
  }
}

/**
 * `DocCtx::retarget_clip_lanes`: a clip moved from track `from` to `to` takes its envelopes
 * along where the target exists there (volume/pan, the send to the same return, devices
 * already on `to`); the others are removed, with a warning.
 */
function retargetClipLanes(ctx: ReducerContext, clipId: ClipId, from: TrackId, to: TrackId): void {
  const { tx } = ctx;
  for (const l of tx.all("AutomationLane")) {
    if (l.owner.type !== "Clip" || l.owner.clip !== clipId) continue;
    const t = l.target;
    let next: AutomationTarget | null;
    switch (t.type) {
      case "TrackVolume":
      case "TrackPan":
        next = t.track === from ? { ...t, track: to } : t;
        break;
      case "SendLevel": {
        const ret = tx.get("Send", t.send)?.to;
        const send = tx.all("Send").find((s) => s.from === to && s.to === ret);
        next = send ? { type: "SendLevel", send: send.id } : null;
        break;
      }
      case "DeviceParam":
      case "DeviceEnabled":
        next = tx.get("Device", t.device)?.track === to ? t : null;
        break;
      // The clip's own gain / transposition move with it (the controller's `other => Some(other)`).
      case "ClipGain":
      case "ClipTranspose":
        next = t;
        break;
    }
    if (next === null) {
      deleteLane(tx, l.id);
      const name = tx.get("Clip", clipId)?.name ?? "";
      ctx.warn?.(`clip "${name}": an envelope was removed (its target does not exist on the new track)`);
    } else if (JSON.stringify(next) !== JSON.stringify(t)) {
      tx.upsert("AutomationLane", { ...l, target: next });
    }
  }
}

function lane(ctx: ReducerContext, id: string): AutomationLane {
  return ctx.tx.get("AutomationLane", id) ?? fail("NotFound", `automation lane ${id}`);
}

function automationCommand(ctx: ReducerContext, c: AutomationCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "CreateLane": {
      if (tx.get("AutomationLane", c.id)) break; // already created: no-op
      if (c.owner.type === "Track") track(ctx, c.owner.track);
      else clip(ctx, c.owner.clip);
      checkTarget(ctx, c.target);
      if (c.target.type === "ClipGain" || c.target.type === "ClipTranspose") {
        const owner = c.owner.type === "Clip" ? tx.get("Clip", c.owner.clip) : undefined;
        if (owner?.content.type !== "Audio") fail("InvalidArgument", "clip gain and transposition envelopes belong to audio clips");
      } else if (c.owner.type === "Clip" && targetTrack(tx, c.target) !== clip(ctx, c.owner.clip).track) {
        fail("InvalidArgument", "a clip envelope must target its own track (volume, pan, its sends or devices)");
      }
      const same = (l: AutomationLane) => JSON.stringify([l.owner, l.target]) === JSON.stringify([c.owner, c.target]);
      if (tx.all("AutomationLane").some(same)) fail("InvalidArgument", "a lane for this owner/target already exists");
      tx.upsert("AutomationLane", { id: c.id, owner: c.owner, target: c.target, enabled: true });
      break;
    }
    case "DeleteLane":
      lane(ctx, c.id);
      deleteLane(ctx.tx, c.id);
      break;
    case "SetLaneEnabled":
      tx.upsert("AutomationLane", { ...lane(ctx, c.id), enabled: c.enabled });
      break;
    case "AddPoints":
      lane(ctx, c.lane);
      for (const p of c.points) {
        if (tx.get("AutomationPoint", p.id)) continue; // already added: kept as is
        tx.upsert("AutomationPoint", { id: p.id, lane: c.lane, time: pointTime(p.time), value: pointValue(p.value), curve: p.curve });
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
          time: e.time !== null ? pointTime(e.time) : p.time,
          value: e.value !== null ? pointValue(e.value) : p.value,
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

// ─── Settings-like commands ─────────────────────────────────────────────────────────────

function transportSettingsCommand(ctx: ReducerContext, c: TransportCommand): void {
  const { tx } = ctx;
  const settings = tx.project.settings;
  switch (c.type) {
    case "SetLoopEnabled":
      tx.setSettings({ ...settings, loop_enabled: c.enabled });
      break;
    case "SetLoopRegion":
      if (!(Number.isFinite(c.region.start) && Number.isFinite(c.region.end) && c.region.start >= 0 && c.region.end > c.region.start)) {
        fail("InvalidArgument", "invalid loop region");
      }
      tx.setSettings({ ...settings, loop_region: c.region });
      break;
    case "SetTempo": {
      const p = tempoPointAt(tx.project, ctx.position) ?? fail("InvalidState", "no tempo point");
      tx.upsert("TempoPoint", { ...p, bpm: clamp(finite(c.bpm, "bpm must be finite"), 20, 999) });
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
      tx.setSettings({ ...tx.project.settings, count_in_bars: Math.min(16, Math.max(0, Math.round(c.bars))) });
      break;
    default:
      fail("Internal", `not a document recording command: ${c.type}`);
  }
}

function checkMarker(beat: Beats, source: number): void {
  if (!(Number.isFinite(beat) && Number.isFinite(source))) fail("InvalidArgument", "warp marker times must be finite");
}

/**
 * `warp::set_warp`: turning warp on for a clip without markers pins the content start to
 * the media start and the media end to the beat it reaches at the detected tempo (else the
 * given `source_bpm`, else the project tempo at the clip start), recorded as `source_bpm`.
 */
function setWarp(ctx: ReducerContext, clipId: ClipId, warp: WarpSettings): void {
  const { tx } = ctx;
  const cl = clip(ctx, clipId);
  const audio = audioContent(cl);
  if (warp.source_bpm !== null && !(Number.isFinite(warp.source_bpm) && warp.source_bpm > 0)) {
    fail("InvalidArgument", "source_bpm must be positive");
  }
  const next = { ...warp };
  const hasMarkers = tx.all("WarpMarker").some((m) => m.clip === clipId);
  const media = tx.get("Media", audio.media);
  if (warp.enabled && !audio.warp.enabled && !hasMarkers && media && media.frames > 0 && media.sample_rate > 0) {
    const seconds = media.frames / media.sample_rate;
    const bpm = detectBpm(seconds) ?? warp.source_bpm ?? bpmAt(tx.project, cl.start);
    next.source_bpm = bpm;
    for (const [beat, source] of [
      [0, 0],
      [(seconds * bpm) / 60, seconds],
    ] as const) {
      tx.upsert("WarpMarker", { id: ctx.newId(), clip: clipId, beat, source });
    }
  }
  if (JSON.stringify(audio.warp) !== JSON.stringify(next)) tx.upsert("Clip", { ...cl, content: { ...audio, warp: next } });
}

function warpCommand(ctx: ReducerContext, c: WarpCommand): void {
  const { tx } = ctx;
  switch (c.type) {
    case "SetWarp":
      setWarp(ctx, c.clip, c.warp);
      break;
    case "AddMarker":
      if (tx.get("WarpMarker", c.id)) break; // already created: no-op
      audioContent(clip(ctx, c.clip));
      checkMarker(c.beat, c.source);
      tx.upsert("WarpMarker", { id: c.id, clip: c.clip, beat: c.beat, source: Math.max(0, c.source) });
      break;
    case "MoveMarker": {
      const m = tx.get("WarpMarker", c.id) ?? fail("NotFound", `warp marker ${c.id}`);
      checkMarker(c.beat, c.source);
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

/**
 * Undo label for a command, in the engine's lowercase style ("SetVolume" → "set volume").
 * The engine's labels (`doc::label_of`) are richer (counts, device names); the mock only
 * mirrors their casing.
 */
export function labelOf(command: Command): string {
  if (command.domain === "Edit" && command.command.type === "Batch") return command.command.label;
  return command.command.type
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .split(" ")
    .map((w) => w.toLowerCase())
    .join(" ");
}
