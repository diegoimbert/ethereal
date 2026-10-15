import type { MessageKey } from "@/i18n";
import type { BuiltinDeviceType, Device, DeviceCategory, DeviceDescriptor, DeviceId, ParamInfo, PluginDescriptor, Track } from "@/generated";

/** MIME type used to drag a device within/between chains. */
export const DEVICE_DRAG_TYPE = "application/x-ethereal-device";

/**
 * The device being dragged (set on `dragstart`, cleared on `dragend`): drop targets read it
 * during `dragover`, when the drag's data isn't readable yet, to show where it may land.
 */
let dragging: string | null = null;
export const setDraggingDevice = (id: string | null) => void (dragging = id);
export const draggingDevice = () => dragging;

/** The chain's sections, in signal order (MIDI effects feed the instrument, then audio effects). */
export const CHAIN_SECTIONS: ReadonlyArray<{ id: DeviceCategory; label: MessageKey }> = [
  { id: "NoteEffect", label: "devices.chain.midi_effects" },
  { id: "Instrument", label: "devices.chain.instrument" },
  { id: "AudioEffect", label: "devices.chain.audio_effects" },
];

/**
 * A device's category: its built-in type's descriptor, or its scanned plugin's; a plugin
 * missing from the scan (another machine) counts as an audio effect.
 */
export function deviceCategory(d: Device, builtins: ReadonlyArray<DeviceDescriptor>, plugins: ReadonlyArray<PluginDescriptor> | null): DeviceCategory {
  if (d.kind.type === "Builtin") {
    const type = d.kind.device.type;
    return builtins.find((t) => t.device_type.type === "Builtin" && t.device_type.device === type)?.category ?? "AudioEffect";
  }
  // A missing device (placeholder): what it was.
  if (d.kind.type === "Missing") return d.kind.missing.role;
  const { plugin_id, format } = d.kind.plugin;
  return plugins?.find((p) => p.id === plugin_id && p.format === format)?.category ?? "AudioEffect";
}

/**
 * Whether a built-in type is offered (device browser, insert menus, command palette, import
 * resolver): `implemented: false` marks a type whose engine node is still a contract
 * placeholder (#438). `ListBuiltin` still returns those (flagged) so a document holding one
 * shows it; only the places that offer devices filter here.
 */
export function isOffered(d: DeviceDescriptor): boolean {
  return d.implemented !== false;
}

/** Whether built-in type `d` can go on `track` (instruments only on MIDI tracks; never an unimplemented type). */
export function canInsert(d: DeviceDescriptor, track: Track | undefined): boolean {
  return !!track && d.device_type.type === "Builtin" && isOffered(d) && (d.category !== "Instrument" || track.kind === "Midi");
}

/** Built-in types that can go on `track` (instruments only on MIDI tracks). */
export function insertableTypes(types: ReadonlyArray<DeviceDescriptor>, track: Track): DeviceDescriptor[] {
  return types.filter((d) => canInsert(d, track));
}

/** Built-in MIDI effects (category `NoteEffect`). Plugins aren't constrained by the chain rule. */
const NOTE_EFFECTS: ReadonlySet<BuiltinDeviceType> = new Set<BuiltinDeviceType>([
  "Arpeggiator",
  "Chord",
  "ScaleQuantize",
  "NoteLength",
  "Velocity",
  "Randomizer",
  "Pitch",
  "MidiEffectRack",
]);

const isNoteEffect = (d: Device): boolean => d.kind.type === "Builtin" && NOTE_EFFECTS.has(d.kind.device.type);

/**
 * Where a new device of `category` goes in a chain (`before`; `null` = append), per the chain
 * rule (CONTRACTS.md §12.4.4: MIDI effects precede the instrument). A MIDI effect, or an
 * instrument on a track without one, goes before the first device that isn't a MIDI effect;
 * audio effects append.
 */
export function chainInsertBefore(devices: readonly Device[], category: DeviceCategory): DeviceId | null {
  if (category === "AudioEffect") return null;
  return devices.find((d) => !isNoteEffect(d))?.id ?? null;
}

/** Visible params grouped by `ParamInfo.group`, in first-appearance order. */
export function groupParams(params: ReadonlyArray<ParamInfo>): Array<{ group: string | null; params: ParamInfo[] }> {
  const out: Array<{ group: string | null; params: ParamInfo[] }> = [];
  for (const p of params) {
    if (p.hidden) continue;
    let g = out.find((x) => x.group === p.group);
    if (!g) out.push((g = { group: p.group, params: [] }));
    g.params.push(p);
  }
  return out;
}

export type ParamGroup = ReturnType<typeof groupParams>[number];

/** A device shows all its params up to this many; above it, the rest folds under "More". */
export const FOLD_OVER = 6;
/** Params in the always-visible main section (whole leading groups, at least one group). */
export const MAIN_MAX = 4;

/**
 * Split grouped params into the main section (shown large, always visible) and the rest
 * (behind "More"): small devices show everything; bigger ones keep leading whole groups
 * up to `MAIN_MAX` params.
 */
export function splitMainParams(groups: ReadonlyArray<ParamGroup>): { main: ParamGroup[]; more: ParamGroup[] } {
  const total = groups.reduce((n, g) => n + g.params.length, 0);
  if (total <= FOLD_OVER) return { main: [...groups], more: [] };
  let count = 0;
  let i = 0;
  while (i < groups.length && (i === 0 || count + groups[i]!.params.length <= MAIN_MAX)) {
    count += groups[i]!.params.length;
    i++;
  }
  return { main: groups.slice(0, i), more: groups.slice(i) };
}
