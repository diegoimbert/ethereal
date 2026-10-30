/**
 * "Group into rack" (`Rack::Group`): a device's context menu, and ⌘G / Ctrl+G on the selected
 * device cards (keymap `device.group`, `features/devices/deviceClipboard.ts`). The devices
 * must be next to each other on one track's chain or one rack chain (the rack nests there); the rack is the kind that holds them all
 * (an instrument rack when an instrument is among them, a MIDI or audio effect rack for
 * effects of that kind only). The new rack becomes the selection.
 */

import type { BuiltinDeviceType, Command, Device, DeviceCategory, DeviceDescriptor, DeviceId, Project } from "@/generated";
import { notify } from "@/features/notifications";
import { errorMessage } from "@/features/transport-bar/engine";
import { t, type MessageKey } from "@/i18n";
import type { ContextMenuEntry } from "@/kit";
import { cmd, newId } from "@/transport";
import { isChainRack } from "./model";

/** The rack type that can hold a device of this category. */
export function rackTypeFor(category: DeviceDescriptor["category"]): BuiltinDeviceType {
  return category === "Instrument" ? "InstrumentRack" : category === "NoteEffect" ? "MidiEffectRack" : "AudioEffectRack";
}

/** The rack type that can hold devices of all these categories. */
export function rackTypeForAll(categories: ReadonlyArray<DeviceCategory>): BuiltinDeviceType {
  if (categories.length > 0 && categories.every((c) => c === categories[0])) return rackTypeFor(categories[0]!);
  // Mixed: an instrument rack's chain takes MIDI effects, the instrument and audio effects.
  return "InstrumentRack";
}

const GROUP_INTO: Partial<Record<BuiltinDeviceType, MessageKey>> = {
  InstrumentRack: "racks.group.instrument",
  AudioEffectRack: "racks.group.audio_effect",
  MidiEffectRack: "racks.group.midi_effect",
};

const byOrder = (a: Device, b: Device) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1);

/**
 * `ids` in chain order when they can be grouped (next to each other on one track's chain, or
 * on one rack chain: the new rack nests there), else `null`.
 */
export function groupable(project: Project, ids: ReadonlyArray<DeviceId>): Device[] | null {
  const devices = ids.map((id) => project.devices[id]);
  if (devices.length === 0 || devices.some((d) => !d || d.pad !== null)) return null;
  const track = devices[0]!.track;
  const host = devices[0]!.chain ?? null;
  if (devices.some((d) => d!.track !== track || (d!.chain ?? null) !== host)) return null;
  const chain = Object.values(project.devices)
    .filter((d) => d.track === track && d.pad === null && (d.chain ?? null) === host)
    .sort(byOrder);
  const at = devices.map((d) => chain.indexOf(d!)).sort((a, b) => a - b);
  if (at.some((i, k) => i < 0 || (k > 0 && i !== at[k - 1]! + 1))) return null;
  return at.map((i) => chain[i]!);
}

/** The `Rack::Group` command for `devices` (in chain order) into a new rack of `type`. */
export const groupCommand = (devices: ReadonlyArray<Device>, type: BuiltinDeviceType, rack: DeviceId = newId()): Command =>
  cmd("Rack", { type: "Group", rack, rack_type: type, chain: newId(), devices: devices.map((d) => d.id) });

/**
 * Groups `ids` into a new rack (one undo step); says why it can't. Resolves to the rack's id
 * (`null`: not grouped).
 */
export async function groupDevices(
  send: (c: Command) => Promise<unknown>,
  project: Project,
  ids: ReadonlyArray<DeviceId>,
  categoryOf: (d: Device) => DeviceCategory,
): Promise<DeviceId | null> {
  const devices = groupable(project, ids);
  if (!devices) {
    notify("Warning", t("devices.group.not_consecutive"));
    return null;
  }
  const rack = newId();
  try {
    await send(groupCommand(devices, rackTypeForAll(devices.map(categoryOf)), rack));
    return rack;
  } catch (e) {
    notify("Error", t("devices.group.failed", { error: errorMessage(e) }));
    return null;
  }
}

/**
 * The chain racks "ungroup" flattens for `ids`: each rack among them, or the rack a device is
 * in; none nested in another (flattening the outer one flattens it too).
 */
export function ungroupTargets(project: Project, ids: ReadonlyArray<DeviceId>): DeviceId[] {
  const rackOf = (d: Device): DeviceId | null => {
    if (isChainRack(d)) return d.id;
    const host = d.chain != null ? project.rack_chains[d.chain]?.rack : undefined;
    return host ?? null;
  };
  const racks = new Set<DeviceId>();
  for (const id of ids) {
    const d = project.devices[id];
    const r = d ? rackOf(d) : null;
    if (r) racks.add(r);
  }
  const inside = (id: DeviceId, outer: DeviceId): boolean => {
    for (let d = project.devices[id]; d?.chain != null;) {
      const up = project.rack_chains[d.chain]?.rack;
      if (!up) return false;
      if (up === outer) return true;
      d = project.devices[up];
    }
    return false;
  };
  return [...racks].filter((r) => ![...racks].some((o) => o !== r && inside(r, o)));
}

/** Flattens `racks` (`Rack::Ungroup`, one undo step); says why it can't. */
export async function ungroupRacks(send: (c: Command) => Promise<unknown>, racks: ReadonlyArray<DeviceId>): Promise<void> {
  const commands = racks.map((rack) => cmd("Rack", { type: "Ungroup", rack }));
  try {
    await send(commands.length === 1 ? commands[0]! : cmd("Edit", { type: "Batch", label: "ungroup racks", commands })); // i18n-ignore
  } catch (e) {
    notify("Error", t("devices.ungroup.failed", { error: errorMessage(e) }));
  }
}

/** The "ungroup rack (flatten)" menu entry of a chain rack. */
export function ungroupEntries(device: Device, send: (c: Command) => Promise<unknown>, shortcut?: string): ContextMenuEntry[] {
  if (!isChainRack(device)) return [];
  return [{ label: t("racks.ungroup"), shortcut, onSelect: () => void ungroupRacks(send, [device.id]) }];
}

/**
 * Menu entries wrapping `device` into a new rack (track-chain and rack-chain devices; racks
 * too, not drum racks), or the selection when `device` is part of it (`selection`, more than one: grouped
 * when they're next to each other).
 */
export function groupEntries(
  device: Device,
  descriptor: DeviceDescriptor | null,
  send: (c: Command) => unknown,
  selection?: {
    project: Project;
    ids: ReadonlyArray<DeviceId>;
    categoryOf: (d: Device) => DeviceCategory;
    shortcut?: string;
    onGrouped?(rack: DeviceId): void;
  },
): ContextMenuEntry[] {
  if (!descriptor || device.pad !== null) return [];
  if (device.kind.type === "Builtin" && device.kind.device.type === "DrumRack") return [];
  const several = selection && selection.ids.length > 1 && selection.ids.includes(device.id) ? groupable(selection.project, selection.ids) : null;
  const devices = several ?? [device];
  const type = several ? rackTypeForAll(several.map(selection!.categoryOf)) : rackTypeFor(descriptor.category);
  const label = GROUP_INTO[type];
  if (!label) return [];
  return [
    {
      label: t(label),
      shortcut: selection?.shortcut,
      onSelect: () => {
        const rack = newId();
        void Promise.resolve(send(groupCommand(devices, type, rack))).then(() => selection?.onGrouped?.(rack));
      },
    },
  ];
}
