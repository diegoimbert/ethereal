import type { MessageKey } from "@/i18n";
import type { LibraryDevicePayload } from "./libraryDevice";
import type { BuiltinDeviceType, Device, DeviceCategory, DeviceDescriptor, DeviceId, ParamInfo, PluginDescriptor, Track } from "@/generated";

/** MIME type used to drag a device within/between chains. */
export const DEVICE_DRAG_TYPE = "application/x-ethereal-device";
/** MIME type of a device dragged out of the device library (its `LibraryDevicePayload` as JSON). */
export const LIBRARY_DEVICE_DRAG_TYPE = "application/x-ethereal-library-device";

/**
 * The device being dragged (set on `dragstart`, cleared on `dragend`): drop targets read it
 * during `dragover`, when the drag's data isn't readable yet, to show where it may land.
 */
let dragging: string | null = null;
export const setDraggingDevice = (id: string | null) => void (dragging = id);
export const draggingDevice = () => dragging;

/** The library device being dragged (like `draggingDevice`). */
let draggingFromLibrary: LibraryDevicePayload | null = null;
export const setDraggingLibraryDevice = (p: LibraryDevicePayload | null) => void (draggingFromLibrary = p);

/** What is dragged over a chain: one of its devices (or another chain's), or a device of the library. */
export type ChainDrag = { type: "device"; id: DeviceId } | { type: "library"; payload: LibraryDevicePayload };

/** `true` if a drag (its `DataTransfer` types) may be a device or a library device. */
export const isChainDrag = (types: ReadonlyArray<string>) => types.includes(DEVICE_DRAG_TYPE) || types.includes(LIBRARY_DEVICE_DRAG_TYPE);

/** The drag in progress, during `dragover` (`null`: unknown, e.g. from another window). */
export function currentChainDrag(): ChainDrag | null {
  if (dragging) return { type: "device", id: dragging };
  if (draggingFromLibrary) return { type: "library", payload: draggingFromLibrary };
  return null;
}

/** The dropped drag, on `drop` (its data is readable then). */
export function droppedChainDrag(dt: Pick<DataTransfer, "getData">): ChainDrag | null {
  const id = dt.getData(DEVICE_DRAG_TYPE);
  if (id) return { type: "device", id };
  const json = dt.getData(LIBRARY_DEVICE_DRAG_TYPE);
  if (!json) return null;
  try {
    return { type: "library", payload: JSON.parse(json) as LibraryDevicePayload };
  } catch {
    return null;
  }
}

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
 * placeholder. `ListBuiltin` still returns those (flagged) so a document holding one
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
  "NoteBlocker",
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
