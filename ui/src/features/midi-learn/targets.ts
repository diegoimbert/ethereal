// Pure helpers for MIDI learn: target identity and labels, source labels, mode encoding,
// and resolving a clicked control (DOM element) to the `MidiMapTarget` it edits.
import type {
  MidiControl,
  MidiMapMode,
  MidiMapping,
  MidiMapTarget,
  MidiSource,
  Project,
  RelativeEncoding,
  TransportAction,
} from "@/generated";

/** Stable identity of a target (for comparisons and React keys). */
export function targetKey(t: MidiMapTarget): string {
  switch (t.type) {
    case "Param": {
      const p = t.target;
      switch (p.type) {
        case "TrackVolume":
          return `vol:${p.track}`;
        case "TrackPan":
          return `pan:${p.track}`;
        case "SendLevel":
          return `send:${p.send}`;
        case "DeviceParam":
          return `param:${p.device}:${p.param}`;
      }
      break;
    }
    case "TrackMute":
      return `mute:${t.track}`;
    case "TrackSolo":
      return `solo:${t.track}`;
    case "TrackArm":
      return `arm:${t.track}`;
    case "Transport":
      return `transport:${t.action}`;
  }
  return "";
}

export function sameTarget(a: MidiMapTarget | null | undefined, b: MidiMapTarget | null | undefined): boolean {
  return !!a && !!b && targetKey(a) === targetKey(b);
}

/** The mapping bound to `target`, if any. */
export function mappingOf(mappings: Record<string, MidiMapping> | undefined, target: MidiMapTarget): MidiMapping | undefined {
  const key = targetKey(target);
  return Object.values(mappings ?? {}).find((m) => targetKey(m.target) === key);
}

export const TRANSPORT_ACTIONS: ReadonlyArray<{ action: TransportAction; label: string }> = [
  { action: "TogglePlay", label: "Play/Stop" },
  { action: "Play", label: "Play" },
  { action: "Stop", label: "Stop" },
  { action: "ToggleRecord", label: "Record" },
  { action: "ToggleLoop", label: "Loop" },
  { action: "ToggleMetronome", label: "Metronome" },
  { action: "TapTempo", label: "Tap tempo" },
  { action: "PreviousMarker", label: "Previous marker" },
  { action: "NextMarker", label: "Next marker" },
];

/** Param names by device (from descriptors), for labels. */
export type ParamNames = Readonly<Record<string, Readonly<Record<number, string>>>>;

/** Human-readable target, e.g. "Bass · Volume", "Delay · Feedback", "Transport · Loop". */
export function describeTarget(t: MidiMapTarget, project: Project | null, params: ParamNames = {}): string {
  const track = (id: string) => project?.tracks[id]?.name ?? "Missing track";
  switch (t.type) {
    case "Param": {
      const p = t.target;
      switch (p.type) {
        case "TrackVolume":
          return `${track(p.track)} · Volume`;
        case "TrackPan":
          return `${track(p.track)} · Pan`;
        case "SendLevel": {
          const send = project?.sends[p.send];
          return send ? `${track(send.from)} · Send ${track(send.to)}` : "Missing send";
        }
        case "DeviceParam": {
          const device = project?.devices[p.device];
          const name = params[p.device]?.[p.param] ?? `Param ${p.param}`;
          return `${device?.name ?? "Missing device"} · ${name}`;
        }
      }
      break;
    }
    case "TrackMute":
      return `${track(t.track)} · Mute`;
    case "TrackSolo":
      return `${track(t.track)} · Solo`;
    case "TrackArm":
      return `${track(t.track)} · Arm`;
    case "Transport":
      return `Transport · ${TRANSPORT_ACTIONS.find((a) => a.action === t.action)?.label ?? t.action}`;
  }
  return "";
}

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/** Note name with C3 = 60 (Ableton convention). */
export function noteName(key: number): string {
  return `${NOTE_NAMES[key % 12]}${Math.floor(key / 12) - 2}`;
}

export function describeControl(c: MidiControl): string {
  switch (c.type) {
    case "Cc":
      return `CC ${c.number}`;
    case "Note":
      return `Note ${noteName(c.key)}`;
    case "PitchBend":
      return "Pitch bend";
  }
}

/** e.g. "CC 21 · Ch 1 · Knobs" (wildcards: "Any ch", "Any port"). */
export function describeSource(s: MidiSource): string {
  const ch = s.channel === null ? "Any ch" : `Ch ${s.channel + 1}`;
  return `${describeControl(s.control)} · ${ch} · ${s.port ?? "Any port"}`;
}

/** Whether a mapping's source listens to the concrete `seen` source (wildcards match). */
export function sourceMatches(source: MidiSource, seen: MidiSource): boolean {
  return (
    JSON.stringify(source.control) === JSON.stringify(seen.control) &&
    (source.port === null || source.port === seen.port) &&
    (source.channel === null || source.channel === seen.channel)
  );
}

export type ModeValue = "Absolute" | "Toggle" | `Relative:${RelativeEncoding}`;

export const MODE_OPTIONS: ReadonlyArray<{ value: ModeValue; label: string }> = [
  { value: "Absolute", label: "Absolute" },
  { value: "Relative:TwosComplement", label: "Relative (2's comp.)" },
  { value: "Relative:BinaryOffset", label: "Relative (offset 64)" },
  { value: "Relative:SignMagnitude", label: "Relative (sign bit)" },
  { value: "Toggle", label: "Toggle" },
];

export function modeValue(m: MidiMapMode): ModeValue {
  return m.type === "Relative" ? `Relative:${m.encoding}` : m.type;
}

export function parseMode(v: ModeValue): MidiMapMode {
  if (v.startsWith("Relative:")) return { type: "Relative", encoding: v.slice("Relative:".length) as RelativeEncoding };
  return { type: v as "Absolute" | "Toggle" };
}

// ─── Resolving controls in the DOM ───────────────────────────────────────────────────

/**
 * Controls opt in to MIDI learn with `data-midi-target='<MidiMapTarget JSON>'` on the
 * element standing for the control (spread `midiTarget(target)` on it). Detection reads only
 * this attribute, never class names, so restyling or restructuring a control keeps it
 * mappable.
 */
export const MIDI_TARGET_ATTR = "data-midi-target";

/** Every element that may be a mappable control (resolve each with `resolveControl`). */
export const MAPPABLE_SELECTOR = `[${MIDI_TARGET_ATTR}]`;

/** Props marking an element as the control for `target`: `<div {...midiTarget(t)}>`. */
export function midiTarget(target: MidiMapTarget): { [MIDI_TARGET_ATTR]: string } {
  return { [MIDI_TARGET_ATTR]: JSON.stringify(target) };
}

export interface ResolvedControl {
  /** The element standing for the control (highlighted in MIDI mode). */
  element: Element;
  target: MidiMapTarget;
}

/** Whether the entity `target` points at is in the project (a stale control maps nothing). */
export function targetExists(target: MidiMapTarget, project: Project): boolean {
  switch (target.type) {
    case "Param": {
      const p = target.target;
      switch (p.type) {
        case "TrackVolume":
        case "TrackPan":
          return !!project.tracks[p.track];
        case "SendLevel":
          return !!project.sends[p.send];
        case "DeviceParam":
          return !!project.devices[p.device] && Number.isFinite(p.param);
      }
      return false;
    }
    case "TrackMute":
    case "TrackSolo":
    case "TrackArm":
      return !!project.tracks[target.track];
    case "Transport":
      return true;
  }
  return false;
}

/** The mappable control containing `start` (`null` if none, or if its entity is gone). */
export function resolveControl(start: Element, project: Project | null): ResolvedControl | null {
  if (!project) return null;
  const element = start.closest(`[${MIDI_TARGET_ATTR}]`);
  if (!element) return null;
  let target: MidiMapTarget;
  try {
    target = JSON.parse(element.getAttribute(MIDI_TARGET_ATTR) ?? "") as MidiMapTarget;
  } catch {
    return null;
  }
  return target && typeof target === "object" && targetExists(target, project) ? { element, target } : null;
}
