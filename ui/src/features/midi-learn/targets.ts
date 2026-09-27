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
 * Explicit opt-in: any element with `data-midi-target='<MidiMapTarget JSON>'` is mappable.
 * Without it, targets are inferred from the stable hooks of the mixer strip (`data-track`,
 * `data-send-to`, `eth-strip__*`), device params (`data-device` + `data-param`) and the
 * transport bar (`eth-tb`).
 */
export const MIDI_TARGET_ATTR = "data-midi-target";

/** Every element that may be a mappable control (resolve each with `resolveControl`). */
export const MAPPABLE_SELECTOR = [
  `[${MIDI_TARGET_ATTR}]`,
  "[data-device] [data-param]",
  "[data-track] .eth-strip__pan",
  "[data-track] .eth-strip__fader-row .eth-fader",
  "[data-track] .eth-strip__mute",
  "[data-track] .eth-strip__solo",
  "[data-track] [data-send-to]",
  ".eth-tb button",
].join(", ");

export interface ResolvedControl {
  /** The element standing for the control (highlighted in MIDI mode). */
  element: Element;
  target: MidiMapTarget;
}

function transportAction(button: Element): TransportAction | null {
  const label = button.getAttribute("aria-label");
  if (button.classList.contains("eth-tb__play")) return "TogglePlay";
  if (button.classList.contains("eth-tb__record")) return "ToggleRecord";
  if (label === "Stop") return "Stop";
  if (label === "Loop") return "ToggleLoop";
  if (label === "Metronome") return "ToggleMetronome";
  if (button.getAttribute("title") === "Tap tempo") return "TapTempo";
  return null;
}

/** The mappable control containing `start` (`null` if none, or if its entity is gone). */
export function resolveControl(start: Element, project: Project | null): ResolvedControl | null {
  if (!project) return null;
  const explicit = start.closest(`[${MIDI_TARGET_ATTR}]`);
  if (explicit) {
    try {
      return { element: explicit, target: JSON.parse(explicit.getAttribute(MIDI_TARGET_ATTR) ?? "") as MidiMapTarget };
    } catch {
      return null;
    }
  }

  const param = start.closest("[data-param]");
  const device = param?.closest("[data-device]");
  if (param && device) {
    const id = device.getAttribute("data-device") ?? "";
    const p = Number(param.getAttribute("data-param"));
    if (!project.devices[id] || !Number.isFinite(p)) return null;
    return { element: param, target: { type: "Param", target: { type: "DeviceParam", device: id, param: p } } };
  }

  const strip = start.closest("[data-track]");
  const track = strip?.getAttribute("data-track");
  if (strip && track && project.tracks[track]) {
    const sendEl = start.closest("[data-send-to]");
    if (sendEl && strip.contains(sendEl)) {
      // The pre/post button is not the level.
      if (start.closest("button")) return null;
      const to = sendEl.getAttribute("data-send-to");
      const send = Object.values(project.sends).find((s) => s.from === track && s.to === to);
      return send ? { element: sendEl, target: { type: "Param", target: { type: "SendLevel", send: send.id } } } : null;
    }
    const pan = start.closest(".eth-strip__pan");
    if (pan) return { element: pan, target: { type: "Param", target: { type: "TrackPan", track } } };
    const fader = start.closest(".eth-fader");
    if (fader && fader.closest(".eth-strip__fader-row")) {
      return { element: fader, target: { type: "Param", target: { type: "TrackVolume", track } } };
    }
    const mute = start.closest(".eth-strip__mute");
    if (mute) return { element: mute, target: { type: "TrackMute", track } };
    const solo = start.closest(".eth-strip__solo");
    if (solo) return { element: solo, target: { type: "TrackSolo", track } };
    return null;
  }

  const button = start.closest(".eth-tb button");
  if (button) {
    const action = transportAction(button);
    return action ? { element: button, target: { type: "Transport", action } } : null;
  }
  return null;
}
