/**
 * Device copy / paste. A click on a device card selects it (and focuses it); ⌘C / Ctrl+C
 * copies the selected device, ⌘V / Ctrl+V pastes it after the selected one, in whatever chain
 * that one is in (a track's, a rack chain, a drum pad's). The header's context menu has
 * "Copy" and "Paste After". A paste is `Device::Duplicate` onto the target's track (params,
 * plugin state, a rack's chains, a drum rack's pads) then a move to its place, in one
 * `Edit::Batch`: one undo step, and all or nothing (a refused move used to leave the
 * copy on the source track).
 *
 * With no device selected (a click on a chain's empty space clears the selection and focuses
 * the chain), ⌘V appends the copy at the end of that chain (the end of its section: an
 * instrument still goes before the audio effects), an empty chain included.
 *
 * The keys come in as the document's `copy` / `paste` events (the desktop app's Edit menu
 * sends them), taken only while the focus is in a device card (copy, paste) or a device chain
 * (paste): the arranger takes them while the focus is in the arranger.
 */
import { create } from "zustand";
import type { Command, Device, DeviceCategory, DeviceId, DrumPadId, Project, RackChainId, TrackId } from "@/generated";
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

/**
 * A device chain, as a paste target: a track's, a rack chain's or a drum pad's. The chain
 * views carry it on their element (`data-paste-chain`, `pasteChainAttr`).
 */
export type PasteChain = { type: "Track"; track: TrackId } | { type: "Chain"; chain: RackChainId } | { type: "Pad"; pad: DrumPadId };

/** The chain `d` is in. */
function chainOf(d: Device): PasteChain {
  return d.pad !== null ? { type: "Pad", pad: d.pad } : d.chain != null ? { type: "Chain", chain: d.chain } : { type: "Track", track: d.track };
}

/** The devices of `chain`, in order. */
function chainDevices(p: Project, chain: PasteChain): Device[] {
  return Object.values(p.devices)
    .filter((x) =>
      chain.type === "Pad"
        ? x.pad === chain.pad
        : chain.type === "Chain"
          ? x.chain === chain.chain
          : x.track === chain.track && x.pad === null && x.chain == null,
    )
    .sort(byOrder);
}

/** The track `chain`'s devices are on (its rack's or drum rack's for a rack chain or a pad). */
function chainTrack(p: Project, chain: PasteChain): TrackId | undefined {
  if (chain.type === "Track") return p.tracks[chain.track] ? chain.track : undefined;
  const rack = chain.type === "Chain" ? p.rack_chains[chain.chain]?.rack : p.drum_pads[chain.pad]?.rack;
  return rack ? p.devices[rack]?.track : undefined;
}

/**
 * Where a pasted device of `category` goes in `chain`: right after `target` when they are of
 * the same kind, else (and with no target) at the end of its own section (MIDI effects first,
 * then the instrument, then audio effects). `null`: the end.
 */
export function pastePlace(
  chain: Device[],
  target: Device | null,
  category: DeviceCategory,
  categoryOf: (d: Device) => DeviceCategory,
  skip: DeviceId,
): DeviceId | null {
  const list = chain.filter((d) => d.id !== skip);
  if (target && categoryOf(target) === category) return list[list.indexOf(target) + 1]?.id ?? null;
  if (category === "AudioEffect") return null;
  const after: DeviceCategory[] = category === "NoteEffect" ? ["Instrument", "AudioEffect"] : ["AudioEffect"];
  return list.find((d) => after.includes(categoryOf(d)))?.id ?? null;
}

/**
 * Pastes the copied device after the device `to`, or at the end of the chain `to` (one undo
 * step); says why when it can't. The copy becomes the selection.
 */
export async function pasteDevice(transport: EngineTransport, to: DeviceId | PasteChain, categoryOf: (d: Device) => DeviceCategory): Promise<void> {
  const { copied } = useDeviceClipboard.getState();
  const project = useProjectStore.getState().project;
  if (!project) return;
  const source = copied ? project.devices[copied] : undefined;
  const target = typeof to === "string" ? project.devices[to] : null;
  if (target === undefined) return;
  const chain = target ? chainOf(target) : (to as PasteChain);
  const track = target ? target.track : chainTrack(project, chain);
  if (!track) return;
  if (!source) {
    notify("Warning", t("devices.clipboard.nothing_to_paste"));
    return;
  }
  const id = newId();
  // Placed among the target chain as it is now (the copy isn't in it yet).
  const before = pastePlace(chainDevices(project, chain), target, categoryOf(source), categoryOf, id);
  const move: Command =
    chain.type === "Pad"
      ? cmd("DrumRack", { type: "MoveDevice", id, pad: chain.pad, before })
      : chain.type === "Chain"
        ? cmd("Rack", { type: "MoveDevice", id, chain: chain.chain, before })
        : cmd("Device", { type: "Move", id, track, before });
  // The copy is made on the target's track (a rack with its chains can't move tracks).
  const duplicate = cmd("Device", { type: "Duplicate", id: source.id, new_id: id, ...(track !== source.track ? { track } : {}) });
  try {
    // The label is the English for agents; the UI shows the key's message.
    await transport.send(cmd("Edit", { type: "Batch", label: "paste device", key: { id: "undo.device.paste", args: {} }, commands: [duplicate, move] })); // i18n-ignore
    useDeviceClipboard.getState().select(id);
  } catch (e) {
    notify("Error", t("devices.clipboard.paste_failed", { name: source.name, error: errorMessage(e) }));
  }
}

/** The `data-paste-chain` attribute of a chain view's element. */
export const pasteChainAttr = (chain: PasteChain): string => JSON.stringify(chain);

/**
 * Where ⌘V pastes from the focus (not while typing in a field): after the focused device card,
 * else at the end of the focused chain (the innermost: a rack chain inside its rack's card).
 */
export function pasteTarget(): DeviceId | PasteChain | null {
  const el = document.activeElement;
  if (!(el instanceof HTMLElement) || el.closest("input, textarea, [contenteditable='true']")) return null;
  const at = el.closest<HTMLElement>(".eth-device[data-device], [data-paste-chain]");
  if (!at) return null;
  if (at.dataset.device) return at.dataset.device;
  try {
    return JSON.parse(at.dataset.pasteChain!) as PasteChain;
  } catch {
    return null;
  }
}

/** The device card holding the focus (not while typing in a field), if any. */
export function focusedDevice(): DeviceId | null {
  const el = document.activeElement;
  // A rack's chain row has its own clipboard (racks/chainClipboard.ts).
  if (!(el instanceof HTMLElement) || el.closest("input, textarea, [contenteditable='true'], .eth-rack__row")) return null;
  return el.closest<HTMLElement>(".eth-device[data-device]")?.dataset.device ?? null;
}

/** What the document listeners paste with (the latest mounted card's transport and categories). */
const current: { transport: EngineTransport | null; categoryOf: ((d: Device) => DeviceCategory) | null } = { transport: null, categoryOf: null };
let bound = false;

/**
 * Called by every device card and chain: binds the document's copy / paste events once (for
 * the app's lifetime; they act while a device card or chain has the focus) and keeps what
 * they paste with.
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
    const to = pasteTarget();
    if (!to || !current.transport || !current.categoryOf) return;
    e.preventDefault();
    void pasteDevice(current.transport, to, current.categoryOf);
  });
}
