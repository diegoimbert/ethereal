/**
 * Device copy / cut / paste. A click on a device card selects it (and focuses it); ⌘ / Ctrl-click
 * adds or removes a card, Shift-click selects the cards between the last clicked one and
 * this one (in its chain); MIDI effects, instruments and audio effects together. ⌘C / Ctrl+C
 * copies the selection (or the focused card when it isn't selected), ⌘V / Ctrl+V pastes it
 * after the selected card, in whatever chain that one is in (a track's, a rack chain, a drum
 * pad's), each copy in its own section: the target's kind right after it, the others at the
 * end of their section. A pasted instrument replaces the chain's instrument, asked first. The
 * header's context menu has "Copy", "Cut" and "Paste After". ⌘X / Ctrl+X cuts: the cards
 * dim and stay until pasted, then the paste moves them (copies placed, originals removed, one
 * undo step), like a cut rack chain. A paste is `Device::Duplicate` onto
 * the target's track (params, plugin state, a rack's chains, a drum rack's pads) then a move
 * to its place for each device, in one `Edit::Batch`: one undo step, and all or nothing (a
 * refused move used to leave the copy on the source track).
 *
 * With no device selected (a click on a chain's empty space clears the selection and focuses
 * the chain), ⌘V pastes at the end of each section of that chain, an empty chain included;
 * with the focus on no chain at all (and not in an editor), at the end of the selected
 * track's chain.
 *
 * ⌘A / Ctrl+A selects every device of the focused chain; ⌘G / Ctrl+G groups the selection
 * into a rack (`@/features/racks/group`); Delete / Backspace removes it.
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
import { errorMessage, isTextEntry } from "@/features/transport-bar/engine";
import { useProjectStore, useSelectionStore } from "@/state";
import { cmd, newId, type EngineTransport } from "@/transport";
import { CHAIN_SECTIONS } from "./chainUtils";
import { canHost } from "./libraryInsert";
import { cannotHostMessage } from "./userInsert";
import { matchesAction } from "@/features/keymap";
import { groupDevices, ungroupRacks, ungroupTargets } from "@/features/racks/group";

/** How a click changes the selection: plain, ⌘ / Ctrl (toggle), Shift (range). */
export type DeviceSelectMode = "replace" | "toggle" | "range";

interface DeviceClipboardState {
  /** The selected device cards (the paste target is the last one clicked, focused). */
  selected: DeviceId[];
  /** Where a Shift-click range starts (the last plain or ⌘-click). */
  anchor: DeviceId | null;
  /** The copied devices (pasting duplicates them, so they must still exist). */
  copied: DeviceId[];
  /** The project when they were copied: a paste restores their settings as they were then. */
  snapshot: Project | null;
  /** They were cut: the next paste moves them (the copies land, the originals go). */
  cut: boolean;
  select(id: DeviceId | null, mode?: DeviceSelectMode): void;
  copy(ids: DeviceId | readonly DeviceId[], cut?: boolean): void;
}

export const useDeviceClipboard = create<DeviceClipboardState>((set, get) => ({
  selected: [],
  anchor: null,
  copied: [],
  snapshot: null,
  cut: false,
  select: (id, mode = "replace") => {
    if (id === null) return set({ selected: [], anchor: null });
    const { selected, anchor } = get();
    if (mode === "toggle") return set({ selected: selected.includes(id) ? selected.filter((x) => x !== id) : [...selected, id], anchor: id });
    if (mode === "range" && anchor) {
      const range = rangeIn(anchor, id);
      if (range) return set({ selected: [...selected.filter((x) => !range.includes(x)), ...range] });
    }
    set({ selected: [id], anchor: id });
  },
  copy: (ids, cut = false) => set({ copied: typeof ids === "string" ? [ids] : [...ids], cut, snapshot: useProjectStore.getState().project }),
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

/**
 * What ⌘C / "Copy" (or ⌘X / "Cut": `cut`) on `id` takes: the selection when `id` is in it,
 * else `id` alone. Cut devices stay until pasted (like a cut rack chain): the paste moves them.
 */
export function copyFrom(id: DeviceId, cut = false): void {
  const { selected, copy } = useDeviceClipboard.getState();
  copy(selected.includes(id) ? selected : [id], cut);
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
 * one replaces it, once confirmed. Cut devices move (the next paste copies them again). The
 * copies become the selection.
 */
export async function pasteDevice(transport: EngineTransport, to: DeviceId | PasteChain, categoryOf: (d: Device) => DeviceCategory): Promise<void> {
  const { copied, cut, snapshot } = useDeviceClipboard.getState();
  // Moved: the next paste copies what landed.
  await placeDevices(
    transport,
    copied,
    to,
    categoryOf,
    cut,
    (ids) => cut && useDeviceClipboard.setState({ copied: ids, cut: false, snapshot: useProjectStore.getState().project }),
    snapshot,
  );
}

/**
 * Copies (or moves: `move`) devices `ids` after the device `to`, or to the end of the chain
 * `to`, each in its own section (one undo step): the paste, and devices dropped on a track in
 * the arranger. Says why when it can't; a placed instrument where the chain has one replaces
 * it, once confirmed (it resolves once asked). `onPlaced` gets the new devices' ids, which
 * become the selection. With `snapshot` (the project at the copy), each copy gets the
 * source's name, on/off and params as they were then, not as edited since.
 */
export async function placeDevices(
  transport: EngineTransport,
  ids: readonly DeviceId[],
  to: DeviceId | PasteChain,
  categoryOf: (d: Device) => DeviceCategory,
  move = false,
  onPlaced?: (ids: DeviceId[]) => void,
  snapshot: Project | null = null,
): Promise<void> {
  const project = useProjectStore.getState().project;
  if (!project) return;
  const target = typeof to === "string" ? project.devices[to] : null;
  if (target === undefined) return;
  const chain = target ? chainOf(target) : (to as PasteChain);
  const track = target ? target.track : chainTrack(project, chain);
  if (!track) return;
  const sources = ids
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
  const placed: DeviceId[] = [];
  for (const source of pasted) {
    const id = newId();
    placed.push(id);
    const before = source === instrument && replaced ? replaced.id : pastePlace(devices, target, categoryOf(source), categoryOf);
    // The copy is made on the target's track (a rack with its chains can't move tracks).
    commands.push(cmd("Device", { type: "Duplicate", id: source.id, new_id: id, ...(track !== source.track ? { track } : {}) }));
    if (!move) commands.push(...restoreCommands(id, source, snapshot?.devices[source.id]));
    commands.push(
      chain.type === "Pad"
        ? cmd("DrumRack", { type: "MoveDevice", id, pad: chain.pad, before })
        : chain.type === "Chain"
          ? cmd("Rack", { type: "MoveDevice", id, chain: chain.chain, before })
          : cmd("Device", { type: "Move", id, track, before }),
    );
  }
  // The replaced instrument and, for a move, the originals go last (the moves above are placed
  // against them).
  const removed = new Set([...(replaced ? [replaced.id] : []), ...(move ? pasted.map((d) => d.id) : [])]);
  for (const id of removed) commands.push(cmd("Device", { type: "Remove", id }));
  const names = pasted.map((d) => d.name).join(", ");
  const run = async () => {
    try {
      // The label is the English for agents; the UI shows the key's message.
      await transport.send(cmd("Edit", { type: "Batch", label: "paste device", key: { id: "undo.device.paste", args: {} }, commands })); // i18n-ignore
      useDeviceClipboard.setState({ selected: placed, anchor: placed.at(-1) ?? null });
      onPlaced?.(placed);
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

/** Commands giving copy `id` of `now` the settings `then` had (the device at the copy). */
function restoreCommands(id: DeviceId, now: Device, then: Device | undefined): Command[] {
  if (!then || then === now) return [];
  const out: Command[] = [];
  if (then.name !== now.name) out.push(cmd("Device", { type: "Rename", id, name: then.name }));
  if (then.enabled !== now.enabled) out.push(cmd("Device", { type: "SetEnabled", id, enabled: then.enabled }));
  for (const [param, value] of Object.entries(then.params)) {
    if (now.params[Number(param)] !== value) out.push(cmd("Device", { type: "SetParam", device: id, param: Number(param), value }));
  }
  return out;
}

/**
 * Turn `id` on or off; when it is part of a selection of several devices, all of them (one
 * undo step), so on/off on one card of a selection sets the whole selection.
 */
export async function setDevicesEnabled(transport: EngineTransport, id: DeviceId, enabled: boolean): Promise<void> {
  const project = useProjectStore.getState().project;
  const { selected } = useDeviceClipboard.getState();
  const ids = selected.length > 1 && selected.includes(id) ? selected.filter((d) => project?.devices[d]) : [id];
  const commands = ids.map((d) => cmd("Device", { type: "SetEnabled", id: d, enabled }));
  await transport.send(
    commands.length === 1
      ? commands[0]!
      : cmd("Edit", {
          type: "Batch",
          label: enabled ? "enable devices" : "bypass devices", // i18n-ignore
          key: { id: enabled ? "undo.device.enable_selected" : "undo.device.bypass_selected", args: {} },
          commands,
        }),
  );
}

/**
 * Removes `ids` (one undo step); a drum pad left without devices goes with them (like its
 * last device's ×). Nothing is selected afterwards.
 */
export async function deleteDevices(transport: EngineTransport, ids: readonly DeviceId[]): Promise<void> {
  const project = useProjectStore.getState().project;
  if (!project) return;
  const devices = ids.map((id) => project.devices[id]).filter((d): d is Device => !!d);
  // Inside a removed rack: it goes with the rack.
  const gone = new Set(devices.map((d) => d.id));
  const inside = (d: Device): boolean => {
    const rack = d.chain != null ? project.rack_chains[d.chain]?.rack : d.pad !== null ? project.drum_pads[d.pad]?.rack : undefined;
    return rack !== undefined && (gone.has(rack) || (!!project.devices[rack] && inside(project.devices[rack])));
  };
  const top = devices.filter((d) => !inside(d));
  if (top.length === 0) return;
  const pads = new Set(top.flatMap((d) => (d.pad !== null ? [d.pad] : [])));
  const emptied = [...pads].filter((pad) => chainDevices(project, { type: "Pad", pad }).every((d) => gone.has(d.id)));
  const commands: Command[] = [
    ...top.filter((d) => d.pad === null || !emptied.includes(d.pad)).map((d) => cmd("Device", { type: "Remove", id: d.id })),
    ...emptied.map((pad) => cmd("DrumRack", { type: "RemovePad", id: pad })),
  ];
  try {
    await transport.send(
      commands.length === 1
        ? commands[0]!
        : cmd("Edit", { type: "Batch", label: "delete devices", key: { id: "undo.device.remove_selected", args: {} }, commands }), // i18n-ignore
    );
    useDeviceClipboard.getState().select(null);
  } catch (e) {
    notify("Error", t("devices.device.delete_failed", { error: errorMessage(e) }));
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

/**
 * Where ⌘V pastes copied devices when the focus is on no card nor chain: the end of the
 * selected track's chain (each device at the end of its section). Not while an editor (the
 * arranger, the piano roll: their own paste), a field or a rack chain row has the focus.
 */
export function fallbackPasteTarget(): PasteChain | null {
  if (useDeviceClipboard.getState().copied.length === 0) return null;
  const el = document.activeElement;
  if (el instanceof HTMLElement && el.closest("input, textarea, select, [contenteditable='true'], [data-keymap-scope], .eth-rack__row")) return null;
  const track = useSelectionStore.getState().selectedTrack;
  return track ? { type: "Track", track } : null;
}

/** The device card holding the focus (not while typing in a field), if any. */
export function focusedDevice(): DeviceId | null {
  const el = document.activeElement;
  // A rack's chain row has its own clipboard (racks/chainClipboard.ts).
  if (!(el instanceof HTMLElement) || el.closest("input, textarea, [contenteditable='true'], .eth-rack__row")) return null;
  return el.closest<HTMLElement>(".eth-device[data-device]")?.dataset.device ?? null;
}

/** The devices a shortcut on focused card `id` acts on: the selection, else `id`. */
function actedOn(project: Project, id: DeviceId): DeviceId[] {
  const { selected } = useDeviceClipboard.getState();
  const live = selected.filter((x) => project.devices[x]);
  return live.length > 0 ? live : [id];
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
  document.addEventListener("cut", (e) => {
    const id = focusedDevice();
    if (!id) return;
    e.preventDefault();
    copyFrom(id, true);
  });
  // ⌘A / Ctrl+A (keymap `edit.selectAll`): every device of the focused card's (or chain's) chain.
  document.addEventListener("keydown", (e) => {
    if (e.defaultPrevented || !matchesAction("edit.selectAll", e) || isTextEntry(e.target)) return;
    const at = pasteTarget();
    const project = useProjectStore.getState().project;
    if (!at || !project) return;
    const chain = typeof at === "string" ? (project.devices[at] ? chainOf(project.devices[at]) : null) : at;
    if (!chain) return;
    e.preventDefault();
    const ids = chainDevices(project, chain).map((d) => d.id);
    useDeviceClipboard.setState({ selected: ids, anchor: ids[0] ?? null });
  });
  // Delete / Backspace (keymap `edit.delete`): the selection (or the focused card) goes.
  document.addEventListener("keydown", (e) => {
    if (e.defaultPrevented || !matchesAction("edit.delete", e) || isTextEntry(e.target)) return;
    const id = focusedDevice();
    if (!id || !current.transport) return;
    e.preventDefault();
    const { selected } = useDeviceClipboard.getState();
    void deleteDevices(current.transport, selected.includes(id) ? selected : [id]);
  });
  // ⌘G / Ctrl+G (keymap `device.group`): the selected cards into a rack. The highlighted
  // selection, even when the focus is on a card outside it (a control of another card);
  // the focused card when nothing is selected.
  document.addEventListener("keydown", (e) => {
    if (e.defaultPrevented || !matchesAction("device.group", e) || isTextEntry(e.target)) return;
    const id = focusedDevice();
    const project = useProjectStore.getState().project;
    if (!id || !project || !current.transport || !current.categoryOf) return;
    e.preventDefault();
    const transport = current.transport;
    void groupDevices((c) => transport.send(c), project, actedOn(project, id), current.categoryOf).then(
      (rack) => rack && useDeviceClipboard.setState({ selected: [rack], anchor: rack }),
    );
  });
  // ⌘⇧G / Ctrl+Shift+G (keymap `device.ungroup`): the selected racks (or the focused card's
  // rack, or the rack it is in) flattened into their place.
  document.addEventListener("keydown", (e) => {
    if (e.defaultPrevented || !matchesAction("device.ungroup", e) || isTextEntry(e.target)) return;
    const id = focusedDevice();
    const project = useProjectStore.getState().project;
    if (!id || !project || !current.transport) return;
    const racks = ungroupTargets(project, actedOn(project, id));
    if (racks.length === 0) return;
    e.preventDefault();
    void ungroupRacks((c) => current.transport!.send(c), racks);
  });
  document.addEventListener("paste", (e) => {
    const to = pasteTarget() ?? fallbackPasteTarget();
    if (!to || !current.transport || !current.categoryOf) return;
    e.preventDefault();
    void pasteDevice(current.transport, to, current.categoryOf);
  });
}
