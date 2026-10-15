/**
 * Device copy / paste. A click on a device card selects it (and focuses it); ⌘C / Ctrl+C
 * copies the selected device, ⌘V / Ctrl+V pastes it after the selected one, in whatever chain
 * that one is in (a track's, a rack chain, a drum pad's). The header's context menu has
 * "Copy" and "Paste After". A paste is `Device::Duplicate` onto the target's track (params,
 * plugin state, a rack's chains, a drum rack's pads) then a move to its place, in one
 * `Edit::Batch`: one undo step, and all or nothing (#582: a refused move used to leave the
 * copy on the source track).
 *
 * The keys come in as the document's `copy` / `paste` events (the desktop app's Edit menu
 * sends them), taken only while the focus is in a device card: the arranger takes them while
 * the focus is in the arranger.
 */
import { create } from "zustand";
import type { Command, Device, DeviceCategory, DeviceId, Project } from "@/generated";
import { notify } from "@/features/notifications";
import { t } from "@/i18n";
import { errorMessage } from "@/features/transport-bar/engine";
import { useProjectStore } from "@/state";
import { cmd, newId, type EngineTransport } from "@/transport";

interface DeviceClipboardState {
  /** The selected device card (the paste target). */
  selected: DeviceId | null;
  /** The copied device (pasting duplicates it, so it must still exist). */
  copied: DeviceId | null;
  select(id: DeviceId | null): void;
  copy(id: DeviceId): void;
}

export const useDeviceClipboard = create<DeviceClipboardState>((set) => ({
  selected: null,
  copied: null,
  select: (selected) => set({ selected }),
  copy: (copied) => set({ copied }),
}));

const byOrder = (a: Device, b: Device) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1);

/** The devices of `d`'s chain (its pad's, its rack chain's or its track's), in order. */
function siblings(p: Project, d: Device): Device[] {
  return Object.values(p.devices)
    .filter((x) => (d.pad !== null ? x.pad === d.pad : d.chain != null ? x.chain === d.chain : x.track === d.track && x.pad === null && x.chain == null))
    .sort(byOrder);
}

/**
 * Where a pasted device of `category` goes in `target`'s chain: right after `target` when they
 * are of the same kind, else at its own section's place (MIDI effects first, then the
 * instrument, then audio effects). `null`: the end.
 */
function pastePlace(chain: Device[], target: Device, category: DeviceCategory, categoryOf: (d: Device) => DeviceCategory, skip: DeviceId): DeviceId | null {
  const list = chain.filter((d) => d.id !== skip);
  if (categoryOf(target) === category) return list[list.indexOf(target) + 1]?.id ?? null;
  if (category === "AudioEffect") return null;
  const after: DeviceCategory[] = category === "NoteEffect" ? ["Instrument", "AudioEffect"] : ["AudioEffect"];
  return list.find((d) => after.includes(categoryOf(d)))?.id ?? null;
}

/** Pastes the copied device after `targetId` (one undo step); says why when it can't. */
export async function pasteDevice(transport: EngineTransport, targetId: DeviceId, categoryOf: (d: Device) => DeviceCategory): Promise<void> {
  const { copied } = useDeviceClipboard.getState();
  const project = useProjectStore.getState().project;
  const source = copied ? project?.devices[copied] : undefined;
  const target = project?.devices[targetId];
  if (!project || !target) return;
  if (!source) {
    notify("Warning", t("devices.clipboard.nothing_to_paste"));
    return;
  }
  const id = newId();
  // Placed among the target chain as it is now (the copy isn't in it yet).
  const before = pastePlace(siblings(project, target), target, categoryOf(source), categoryOf, id);
  const move: Command =
    target.pad !== null
      ? cmd("DrumRack", { type: "MoveDevice", id, pad: target.pad, before })
      : target.chain != null
        ? cmd("Rack", { type: "MoveDevice", id, chain: target.chain, before })
        : cmd("Device", { type: "Move", id, track: target.track, before });
  // The copy is made on the target's track (a rack with its chains can't move tracks).
  const duplicate = cmd("Device", { type: "Duplicate", id: source.id, new_id: id, ...(target.track !== source.track ? { track: target.track } : {}) });
  try {
    // The label is the English for agents; the UI shows the key's message.
    await transport.send(cmd("Edit", { type: "Batch", label: "paste device", key: { id: "undo.device.paste", args: {} }, commands: [duplicate, move] })); // i18n-ignore
    useDeviceClipboard.getState().select(id);
  } catch (e) {
    notify("Error", t("devices.clipboard.paste_failed", { name: source.name, error: errorMessage(e) }));
  }
}

/** The device card holding the focus (not while typing in a field), if any. */
export function focusedDevice(): DeviceId | null {
  const el = document.activeElement;
  if (!(el instanceof HTMLElement) || el.closest("input, textarea, [contenteditable='true']")) return null;
  return el.closest<HTMLElement>(".eth-device[data-device]")?.dataset.device ?? null;
}

/** What the document listeners paste with (the latest mounted card's transport and categories). */
const current: { transport: EngineTransport | null; categoryOf: ((d: Device) => DeviceCategory) | null } = { transport: null, categoryOf: null };
let bound = false;

/**
 * Called by every device card: binds the document's copy / paste events once (for the app's
 * lifetime; they act while a device card has the focus) and keeps what they paste with.
 */
export function useDeviceClipboardKeys(transport: EngineTransport, categoryOf: (d: Device) => DeviceCategory): void {
  current.transport = transport;
  current.categoryOf = categoryOf;
  if (bound || typeof document === "undefined") return;
  bound = true;
  document.addEventListener("copy", (e) => {
    const id = focusedDevice();
    if (!id) return;
    e.preventDefault();
    useDeviceClipboard.getState().copy(id);
  });
  document.addEventListener("paste", (e) => {
    const id = focusedDevice();
    if (!id || !current.transport || !current.categoryOf) return;
    e.preventDefault();
    void pasteDevice(current.transport, id, current.categoryOf);
  });
}
