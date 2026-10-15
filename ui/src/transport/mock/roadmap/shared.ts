/**
 * Helpers shared by the mock's simulations: the cascades the core reducer runs when a
 * device, send or track is deleted (MIDI mappings and sidechains that point at it), and
 * small rules that mirror the controller (clamping, time signatures, text and file names).
 */

import type {
  AutomationTarget,
  BuiltinDeviceType,
  Device,
  DeviceId,
  DrumPadId,
  MidiMapping,
  Modulator,
  Project,
  RackChain,
  RackChainId,
  TimeSignature,
  Track,
  TrackId,
} from "@/generated";
import { builtinDescriptor } from "../builtinDevices";
import { fail, type ReducerContext } from "../documentReducer";

export const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/**
 * `warp::detect_bpm`: the tempo at which a loop of `seconds` is a power-of-two number of
 * bars of 4/4, in 80..160 BPM (`null` when none is).
 */
export function detectBpm(seconds: number): number | null {
  if (!(Number.isFinite(seconds) && seconds > 0)) return null;
  for (let k = 0; k < 12; k++) {
    const bpm = ((1 << k) * 4 * 60) / seconds;
    if (bpm >= 80 && bpm < 160) return Math.round(bpm * 100) / 100;
  }
  return null;
}

/** A time signature the engine accepts: 1..99 over a power of two up to 32. */
export function validSignature(sig: TimeSignature): boolean {
  return Number.isInteger(sig.numerator) && sig.numerator >= 1 && sig.numerator <= 99 && [1, 2, 4, 8, 16, 32].includes(sig.denominator);
}

/** The first `n` characters (code points, like Rust's `chars().take(n)`; not UTF-16 units). */
export const takeChars = (s: string, n: number): string => [...s].slice(0, n).join("");

/**
 * A file stem for a preset/template name, like the controller's `presets::files::file_stem`
 * and `templates::files::file_stem`: control characters (`char::is_control`) and characters
 * unsafe in file names become `-`, at most 80 characters, surrounding dots and spaces trimmed.
 */
export function fileStem(name: string, fallback: string): string {
  const cleaned = takeChars(name.replace(/[\p{Cc}/\\:*?"<>|]/gu, "-"), 80);
  return cleaned.trim().replace(/^\.+/, "").replace(/\.+$/, "").trim() || fallback;
}

/** Sort by fractional order key (ties by id), like the Rust chain order. */
export function byOrder<T extends { order: string; id: string }>(a: T, b: T): number {
  if (a.order !== b.order) return a.order < b.order ? -1 : 1;
  return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
}

// ─── Ordered lookups (mirrors of `Project::*_of`) ───────────────────────────────────────
// Project-based; inside a transaction pass `tx.project` (it sees the transaction's writes).
// Deliberately not `@/state/selectors`: those cache per table object, which a transaction
// mutates in place.

function ordered<T extends { order: string; id: string }>(rows: Record<string, T>, keep: (r: T) => boolean, except?: string): T[] {
  return Object.values(rows)
    .filter((r) => r.id !== except && keep(r))
    .sort(byOrder);
}

/** Tracks directly under `parent` (`null` = top level). */
export const childTracksOf = (p: Project, parent: TrackId | null, except?: TrackId): Track[] => ordered(p.tracks, (t) => t.parent === parent, except);

/** A track's own device chain (not its pad or rack-chain devices). */
export const trackChainOf = (p: Project, track: TrackId, except?: DeviceId): Device[] =>
  ordered(p.devices, (d) => d.track === track && d.pad === null && d.chain == null, except);

/** A drum pad's device chain. */
export const padChainOf = (p: Project, pad: DrumPadId, except?: DeviceId): Device[] => ordered(p.devices, (d) => d.pad === pad, except);

/** The chains of a rack. */
export const rackChainsOf = (p: Project, rack: DeviceId, except?: RackChainId): RackChain[] => ordered(p.rack_chains, (c) => c.rack === rack, except);

/** The devices of a rack chain. */
export const chainDevicesOf = (p: Project, chain: RackChainId, except?: DeviceId): Device[] => ordered(p.devices, (d) => d.chain === chain, except);

/** The modulators hosted by a device. */
export const modulatorsOf = (p: Project, host: DeviceId): Modulator[] => ordered(p.modulators, (m) => m.device === host);

/** Racks that hold parallel chains (not the drum rack, whose chains are its pads). */
export const CHAIN_RACK_TYPES: ReadonlyArray<BuiltinDeviceType> = ["InstrumentRack", "AudioEffectRack", "MidiEffectRack"];

export const isChainRack = (d: Device): boolean => d.kind.type === "Builtin" && CHAIN_RACK_TYPES.includes(d.kind.device.type);

export const isDrumRack = (d: Device): boolean => d.kind.type === "Builtin" && d.kind.device.type === "DrumRack";

/** A chain rack or a drum rack: a nesting level (`ether_model::rack`). */
export const isAnyRack = (d: Device): boolean => isChainRack(d) || isDrumRack(d);

/** Levels of racks inside racks (`ether_model::rack::MAX_RACK_DEPTH`). */
export const MAX_RACK_DEPTH = 6;

/**
 * `Project::device_path`: the devices from the track chain down to `id` (inclusive,
 * outermost first), each a rack whose chain or pad holds the next (`null`: dangling,
 * cyclic or deeper than `MAX_RACK_DEPTH` + 1).
 */
export function devicePath(p: Project, id: DeviceId): DeviceId[] | null {
  const out = [id];
  let d = p.devices[id];
  while (d) {
    const parent = d.chain != null ? p.rack_chains[d.chain]?.rack : d.pad !== null ? p.drum_pads[d.pad]?.rack : null;
    if (parent === null) return out.reverse();
    if (parent === undefined || out.includes(parent) || out.length > MAX_RACK_DEPTH + 1) return null;
    out.push(parent);
    d = p.devices[parent];
  }
  return null;
}

/** The devices directly inside `rack`: on its chains or on its pads. */
function rackChildren(p: Project, rack: DeviceId): Device[] {
  return Object.values(p.devices).filter(
    (d) => (d.chain != null && p.rack_chains[d.chain]?.rack === rack) || (d.pad !== null && p.drum_pads[d.pad]?.rack === rack),
  );
}

/** `Project::nested_rack_height`: levels of racks (drum racks included) below `rack`. */
export function nestedRackHeight(p: Project, rack: DeviceId, budget = MAX_RACK_DEPTH + 1): number {
  if (budget === 0) return 0;
  let h = 0;
  for (const d of rackChildren(p, rack)) if (isAnyRack(d)) h = Math.max(h, 1 + nestedRackHeight(p, d.id, budget - 1));
  return h;
}

/** `Project::has_nested_drum_rack`: a drum rack anywhere inside `rack`. */
export function hasNestedDrumRack(p: Project, rack: DeviceId, budget = MAX_RACK_DEPTH + 1): boolean {
  return budget > 0 && rackChildren(p, rack).some((d) => isDrumRack(d) || (isAnyRack(d) && hasNestedDrumRack(p, d.id, budget - 1)));
}

/** `Project::is_under_drum_pad`: a drum rack encloses `id` (it sits under a drum pad). */
export function isUnderDrumPad(p: Project, id: DeviceId): boolean {
  const path = devicePath(p, id);
  return path !== null && path.slice(0, -1).some((r) => p.devices[r] !== undefined && isDrumRack(p.devices[r]!));
}

/** A fresh built-in device's name: its descriptor's, lowercase (every insert path). */
export const defaultDeviceName = (device: Parameters<typeof builtinDescriptor>[0]): string => builtinDescriptor(device).name.toLowerCase();

/** `true` if a mapping target references `track` directly. */
export function mappingRefsTrack(m: MidiMapping, track: TrackId): boolean {
  const t = m.target;
  if (t.type === "Param") return (t.target.type === "TrackVolume" || t.target.type === "TrackPan") && t.target.track === track;
  return (t.type === "TrackMute" || t.type === "TrackSolo" || t.type === "TrackArm") && t.track === track;
}

export function deleteMappingsWhere(ctx: ReducerContext, pred: (m: MidiMapping) => boolean): void {
  for (const m of ctx.tx.all("MidiMapping")) if (pred(m)) ctx.tx.remove("MidiMapping", m.id);
}

/** The device an automation target belongs to (a param or its on/off switch). */
export function targetDevice(t: AutomationTarget): string | null {
  return t.type === "DeviceParam" || t.type === "DeviceEnabled" ? t.device : null;
}

/** Device deleted: its MIDI mappings go. */
export function onDeviceDeleted(ctx: ReducerContext, device: string): void {
  deleteMappingsWhere(ctx, (m) => m.target.type === "Param" && targetDevice(m.target.target) === device);
}

/** Send deleted: its MIDI mappings go. */
export function onSendDeleted(ctx: ReducerContext, send: string): void {
  deleteMappingsWhere(ctx, (m) => m.target.type === "Param" && m.target.target.type === "SendLevel" && m.target.target.send === send);
}

/** Track deleted: its MIDI mappings go and sidechains listening to it are cut. */
export function onTrackDeleted(ctx: ReducerContext, track: TrackId): void {
  deleteMappingsWhere(ctx, (m) => mappingRefsTrack(m, track));
  cutSidechainsWhere(ctx, (d) => d.sidechain === track);
}

/** Clears the sidechain (source and tap) of every device matching `pred`. */
export function cutSidechainsWhere(ctx: ReducerContext, pred: (d: Device) => boolean): void {
  for (const d of ctx.tx.all("Device")) {
    if (!pred(d)) continue;
    const next: Device = { ...d, sidechain: null };
    delete next.sidechain_tap;
    ctx.tx.upsert("Device", next);
  }
}

/** A rack chain / drum pad goes: sidechains tapping it are cut (v0.4). */
export function onSidechainChainDeleted(ctx: ReducerContext, chain: { type: "Rack"; id: string } | { type: "Pad"; id: string }): void {
  cutSidechainsWhere(ctx, (d) => {
    const c = d.sidechain_tap?.chain;
    if (!c) return false;
    return c.type === "Rack" ? chain.type === "Rack" && c.chain === chain.id : chain.type === "Pad" && c.pad === chain.id;
  });
}

/**
 * `midi_fx::check_chain_order`: MIDI effects precede the first instrument of the track's
 * chain and of each of its rack chains (built-ins only; plugins are never constrained).
 */
export function checkChainOrder(p: Project, track: TrackId): void {
  const check = (chain: Device[]) => {
    let instrument: Device | null = null;
    for (const d of chain) {
      const category = d.kind.type === "Builtin" ? builtinDescriptor(d.kind.device).category : null;
      if (category === "Instrument" && !instrument) instrument = d;
      else if (category === "NoteEffect" && instrument) {
        fail("InvalidArgument", `MIDI effect "${d.name}" must come before the instrument "${instrument.name}"`);
      }
    }
  };
  const top = trackChainOf(p, track);
  check(top);
  for (const rack of top) for (const chain of rackChainsOf(p, rack.id)) check(chainDevicesOf(p, chain.id));
}
