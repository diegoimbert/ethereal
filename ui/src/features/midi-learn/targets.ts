// Pure helpers for MIDI learn: target identity and labels, source labels, mode encoding,
// and resolving a clicked control (DOM element) to the `MidiMapTarget` it edits.
import type { MidiControl, MidiMapMode, MidiMapping, MidiMapTarget, MidiSource, Project, RelativeEncoding, TransportAction } from "@/generated";
import { t as tr, type MessageKey } from "@/i18n";

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
        case "DeviceEnabled":
          return `on:${p.device}`;
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

export const TRANSPORT_ACTIONS: ReadonlyArray<{ action: TransportAction; label: MessageKey }> = [
  { action: "TogglePlay", label: "midi_learn.transport.toggle_play" },
  { action: "Play", label: "midi_learn.transport.play" },
  { action: "Stop", label: "midi_learn.transport.stop" },
  { action: "ToggleRecord", label: "midi_learn.transport.record" },
  { action: "ToggleLoop", label: "midi_learn.transport.loop" },
  { action: "ToggleMetronome", label: "midi_learn.transport.metronome" },
  { action: "TapTempo", label: "midi_learn.transport.tap_tempo" },
  { action: "PreviousMarker", label: "midi_learn.transport.previous_marker" },
  { action: "NextMarker", label: "midi_learn.transport.next_marker" },
];

/** A transport action's target label ("transport · loop"). */
export function transportTargetLabel(action: TransportAction): string {
  const key = TRANSPORT_ACTIONS.find((a) => a.action === action)?.label;
  return tr("midi_learn.target.transport", { action: key ? tr(key) : action });
}

/** Param names by device (from descriptors), for labels. */
export type ParamNames = Readonly<Record<string, Readonly<Record<number, string>>>>;

/** Human-readable target, e.g. "Bass · volume", "Delay · Feedback", "transport · loop". */
export function describeTarget(t: MidiMapTarget, project: Project | null, params: ParamNames = {}): string {
  const track = (id: string) => project?.tracks[id]?.name ?? tr("midi_learn.target.missing_track");
  const missingDevice = () => tr("midi_learn.target.missing_device");
  switch (t.type) {
    case "Param": {
      const p = t.target;
      switch (p.type) {
        case "TrackVolume":
          return tr("midi_learn.target.volume", { name: track(p.track) });
        case "TrackPan":
          return tr("midi_learn.target.pan", { name: track(p.track) });
        case "SendLevel": {
          const send = project?.sends[p.send];
          return send ? tr("midi_learn.target.send", { from: track(send.from), to: track(send.to) }) : tr("midi_learn.target.missing_send");
        }
        case "DeviceParam": {
          const device = project?.devices[p.device];
          const name = params[p.device]?.[p.param] ?? tr("midi_learn.target.param", { n: p.param });
          return `${device?.name ?? missingDevice()} · ${name}`;
        }
        case "DeviceEnabled":
          return tr("midi_learn.target.device_on", { device: project?.devices[p.device]?.name ?? missingDevice() });
      }
      break;
    }
    case "TrackMute":
      return tr("midi_learn.target.mute", { name: track(t.track) });
    case "TrackSolo":
      return tr("midi_learn.target.solo", { name: track(t.track) });
    case "TrackArm":
      return tr("midi_learn.target.arm", { name: track(t.track) });
    case "Transport":
      return transportTargetLabel(t.action);
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
      return `cc ${c.number}`; // i18n-ignore
    case "Note":
      return tr("midi_learn.control.note", { note: noteName(c.key) });
    case "PitchBend":
      return tr("midi_learn.control.pitch_bend");
  }
}

/** e.g. "CC 21 · ch 1 · Knobs" (wildcards: "any ch", "any port"). */
export function describeSource(s: MidiSource): string {
  const ch = s.channel === null ? tr("midi_learn.source.any_channel") : tr("midi_learn.source.channel", { n: s.channel + 1 });
  return `${describeControl(s.control)} · ${ch} · ${s.port ?? tr("midi_learn.source.any_port")}`;
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

export const MODE_OPTIONS: ReadonlyArray<{ value: ModeValue; label: MessageKey }> = [
  { value: "Absolute", label: "midi_learn.mode.absolute" },
  { value: "Relative:TwosComplement", label: "midi_learn.mode.relative_twos" },
  { value: "Relative:BinaryOffset", label: "midi_learn.mode.relative_offset" },
  { value: "Relative:SignMagnitude", label: "midi_learn.mode.relative_sign" },
  { value: "Toggle", label: "midi_learn.mode.toggle" },
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
        case "DeviceEnabled":
          return !!project.devices[p.device];
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
