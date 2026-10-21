/**
 * Device copy / paste. A click on a device card selects it (and focuses it); ⌘ / Ctrl-click
 * adds or removes a card, Shift-click selects the cards between the last clicked one and
 * this one (in its chain); MIDI effects, instruments and audio effects together. ⌘C / Ctrl+C
 * copies the selection (or the focused card when it isn't selected), ⌘V / Ctrl+V pastes it
 * after the selected card, in whatever chain that one is in (a track's, a rack chain, a drum
 * pad's), each copy in its own section: the target's kind right after it, the others at the
 * end of their section. A pasted instrument replaces the chain's instrument, asked first. The
 * header's context menu has "Copy" and "Paste After". A paste is `Device::Duplicate` onto
 * the target's track (params, plugin state, a rack's chains, a drum rack's pads) then a move
 * to its place for each device, in one `Edit::Batch`: one undo step, and all or nothing (a
 * refused move used to leave the copy on the source track).
 *
 * With no device selected (a click on a chain's empty space clears the selection and focuses
 * the chain), ⌘V pastes at the end of each section of that chain, an empty chain included.
 *
 * The keys come in as the document's `copy` / `paste` events (the desktop app's Edit menu
 * sends them), taken only while the focus is in a device card (copy, paste) or a device chain
 * (paste): the arranger takes them while the focus is in the arranger.
 */
import { create } from "zustand";
import type { Command, Device, DeviceCategory, DeviceId, DrumPadId, Project, RackChainId, TrackId } from "@/generated";
import { notify } from "@/features/notifications";
import { useConfirm } from "@/kit/confirmStore";
import { t } from "@/i18n";
import { errorMessage } from "@/features/transport-bar/engine";
import { useProjectStore } from "@/state";
import { cmd, newId, type EngineTransport } from "@/transport";
import { CHAIN_SECTIONS } from "./chainUtils";
import { canHost } from "./libraryInsert";
import { cannotHostMessage } from "./userInsert";

/** How a click changes the selection: plain, ⌘ / Ctrl (toggle), Shift (range). */
export type DeviceSelectMode = "replace" | "toggle" | "range";

interface DeviceClipboardState {
  /** The selected device cards (the paste target is the last one clicked, focused). */
  selected: DeviceId[];
  /** Where a Shift-click range starts (the last plain or ⌘-click). */
  anchor: DeviceId | null;
  /** The copied devices (pasting duplicates them, so they must still exist). */
  copied: DeviceId[];
  select(id: DeviceId | null, mode?: DeviceSelectMode): void;
  copy(ids: DeviceId | readonly DeviceId[]): void;
}

export const useDeviceClipboard = create<DeviceClipboardState>((set, get) => ({
  selected: [],
  anchor: null,
  copied: [],
  select: (id, mode = "replace") => {
    if (id === null) return set({ selected: [], anchor: null });
    const { selected, anchor } = get();
    if (mode === "toggle")
      return set({ selected: selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id], anchor: id });
    if (mode === "range" && anchor) {
      const range = rangeIn(anchor, id);
      if (range) return set({ selected: [...selected.filter((x) => !range.includes(x)), ...range] });
    }
    set({ selected: [id], anchor: id });
  },
  copy: (ids) => set({ copied: typeof ids === "string" ? [ids] : [...ids] }),
}));

/** The selection mode of a click on a card. */
export const selectModeOf = (e: { shiftKey: boolean; metaKey: boolean; ctrlKey: boolean }): DeviceSelectMode =>
  e.shiftKey ? "range" : e.metaKey || e.ctrlKey ? "toggle" : "replace";

/** The devices from `a` to `b` (both included) when they are in the same chain, else null. */
function rangeIn(a: DeviceId, b: DeviceId): DeviceId[] | null {
  const p = useProjectStore.getState().project;
  const from = p?.devices[a];
  const to = p?.devices[b];
  if (!p || !from || !to || pasteChainAttr(chainOf(from)) !== pasteChainAttr(chainOf(to))) return null;
  const ids = chainDevices(p, chainOf(from)).map((d) => d.id);
  const i = ids.indexOf(a);
  const j = ids.indexOf(b);
  return ids.slice(Math.min(i, j), Math.max(i, j) + 1);
}

/** What ⌘C / "Copy" on `id` copies: the selection when `id` is in it, else `id` alone. */
export function copyFrom(id: DeviceId): void {
  const { selected, copy } = useDeviceClipboard.getState();
  copy(selected.includes(id) ? selected : [id]);
}

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
 * Where pasted devices of `category` go in `chain`: right after `target` when they are of
 * the same kind, else (and with no target) at the end of their own section (MIDI effects
 * first, then the instrument, then audio effects). `null`: the end.
 */
export function pastePlace(chain: Device[], target: Device | null, category: DeviceCategory, categoryOf: (d: Device) => DeviceCategory): DeviceId | null {
  if (target && categoryOf(target) === category) return chain[chain.indexOf(target) + 1]?.id ?? null;
  if (category === "AudioEffect") return null;
  const after: DeviceCategory[] = category === "NoteEffect" ? ["Instrument", "AudioEffect"] : ["AudioEffect"];
  return chain.find((d) => after.includes(categoryOf(d)))?.id ?? null;
}

const SECTION_ORDER = CHAIN_SECTIONS.map((s) => s.id);

/**
 * Pastes the copied devices after the device `to`, or at the end of the chain `to` (one undo
 * step), each in its section; says why when it can't. A pasted instrument where the chain has
 * one replaces it, once confirmed. The copies become the selection.
 */
export async function pasteDevice(transport: EngineTransport, to: DeviceId | PasteChain, categoryOf: (d: Device) => DeviceCategory): Promise<void> {
  const { copied } = useDeviceClipboard.getState();
  const project = useProjectStore.getState().project;
  if (!project) return;
  const target = typeof to === "string" ? project.devices[to] : null;
  if (target === undefined) return;
  const chain = target ? chainOf(target) : (to as PasteChain);
  const track = target ? target.track : chainTrack(project, chain);
  if (!track) return;
  const sources = copied
    .map((id) => project.devices[id])
    .filter((d): d is Device => !!d)
    .sort((a, b) => SECTION_ORDER.indexOf(categoryOf(a)) - SECTION_ORDER.indexOf(categoryOf(b)) || byOrder(a, b));
  if (sources.length === 0) {
    notify("Warning", t("devices.clipboard.nothing_to_paste"));
    return;
  }
  // A track's chain takes what its kind hosts (an audio track: no instrument, no MIDI effect).
  const host = chain.type === "Track" ? project.tracks[chain.track] : undefined;
  const refused = host ? sources.filter((d) => !canHost(categoryOf(d), host)) : [];
  for (const d of refused) notify("Warning", cannotHostMessage(d.name, categoryOf(d), host!));
  // One instrument per chain: the first copied one.
  const instrument = sources.find((d) => categoryOf(d) === "Instrument" && !refused.includes(d));
  const pasted = sources.filter((d) => !refused.includes(d) && (categoryOf(d) !== "Instrument" || d === instrument));
  if (pasted.length === 0) return;

  // Placed among the target chain as it is now (the copies aren't in it yet).
  const devices = chainDevices(project, chain);
  const replaced = instrument ? devices.find((d) => categoryOf(d) === "Instrument") : undefined;
  const commands: Command[] = [];
  const ids: DeviceId[] = [];
  for (const source of pasted) {
    const id = newId();
    ids.push(id);
    const before = source === instrument && replaced ? replaced.id : pastePlace(devices, target, categoryOf(source), categoryOf);
    // The copy is made on the target's track (a rack with its chains can't move tracks).
    commands.push(cmd("Device", { type: "Duplicate", id: source.id, new_id: id, ...(track !== source.track ? { track } : {}) }));
    commands.push(
      chain.type === "Pad"
        ? cmd("DrumRack", { type: "MoveDevice", id, pad: chain.pad, before })
        : chain.type === "Chain"
          ? cmd("Rack", { type: "MoveDevice", id, chain: chain.chain, before })
          : cmd("Device", { type: "Move", id, track, before }),
    );
  }
  // The replaced instrument goes last (the moves above are placed against it).
  if (replaced) commands.push(cmd("Device", { type: "Remove", id: replaced.id }));
  const names = pasted.map((d) => d.name).join(", ");
  const run = async () => {
    try {
      // The label is the English for agents; the UI shows the key's message.
      await transport.send(cmd("Edit", { type: "Batch", label: "paste device", key: { id: "undo.device.paste", args: {} }, commands })); // i18n-ignore
      useDeviceClipboard.setState({ selected: ids, anchor: ids.at(-1) ?? null });
    } catch (e) {
      notify("Error", t("devices.clipboard.paste_failed", { name: names, error: errorMessage(e) }));
    }
  };
  if (!replaced || !instrument) return run();
  useConfirm.getState().ask({
    title: t("devices.clipboard.replace_title"),
    body: t("devices.clipboard.replace_body", { name: replaced.name, with: instrument.name }),
    action: t("devices.clipboard.replace_action"),
    danger: true,
    run: () => void run(),
  });
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
    copyFrom(id);
  });
  document.addEventListener("paste", (e) => {
    const to = pasteTarget();
    if (!to || !current.transport || !current.categoryOf) return;
    e.preventDefault();
    void pasteDevice(current.transport, to, current.categoryOf);
  });
}
