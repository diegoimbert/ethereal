/**
 * Rack chain moves: drag a chain's row to reorder it in its rack or drop it on another rack's
 * chain list (on any track: a rack card, a nested rack, a rack's window); cmd/ctrl while
 * dropping copies it (the arranger's clip-drag copy key). Copy / cut / paste / duplicate from
 * the row's context menu or the keys (⌘C / ⌘X / ⌘V while a chain row has the focus, the
 * document's `copy` / `cut` / `paste` events like the device clipboard; ⌘D on the row).
 *
 * In its own rack a move is `Rack::MoveChain` (the chain keeps its id). Anywhere else, and every
 * copy, is `Rack::CopyChain` (`cut` for a move): one command, so one undo step; the engine
 * checks the content rules again (the drop targets below only mirror them for the cursor).
 */
import { create } from "zustand";
import type { Device, DeviceCategory, DeviceId, Project, RackChain, RackChainId } from "@/generated";
import { notify } from "@/features/notifications";
import { errorMessage } from "@/features/transport-bar/engine";
import { t } from "@/i18n";
import { useProjectStore } from "@/state";
import { cmd, newId, type EngineTransport } from "@/transport";
import { MAX_RACK_DEPTH, isChainRack, rackLevel, rackTypeOf } from "./model";

/** MIME type of a dragged chain row (its id; the source rack is in `useChainDrag`). */
export const CHAIN_DRAG_TYPE = "application/x-ethereal-rack-chain";

interface ChainClipboardState {
  /** The chain row last clicked (the paste target: pastes go after it). */
  selected: RackChainId | null;
  /** The copied (or cut) chain: pasting copies (or moves) it, so it must still exist. */
  copied: { id: RackChainId; cut: boolean } | null;
  select(id: RackChainId | null): void;
  copy(id: RackChainId, cut?: boolean): void;
}

export const useChainClipboard = create<ChainClipboardState>((set) => ({
  selected: null,
  copied: null,
  select: (selected) => set({ selected }),
  copy: (id, cut = false) => set({ copied: { id, cut } }),
}));

/** Where a dragged chain would land: a rack's list, before its `index`-th chain. */
export interface ChainDropTarget {
  rack: DeviceId;
  index: number;
}

interface ChainDragState {
  /** The dragged chain (`null`: no chain drag). */
  dragging: RackChainId | null;
  /** The drop line (`null`: none, or the pointer is over a rack the chain can't go in). */
  over: ChainDropTarget | null;
  setOver(over: ChainDropTarget | null): void;
}

export const useChainDrag = create<ChainDragState>((set) => ({
  dragging: null,
  over: null,
  setOver: (over) => set({ over }),
}));

export const startChainDrag = (id: RackChainId) => useChainDrag.setState({ dragging: id, over: null });
export const endChainDrag = () => useChainDrag.setState({ dragging: null, over: null });

const byOrder = <T extends { order: string; id: string }>(a: T, b: T) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1);

export function chainsOf(p: Pick<Project, "rack_chains">, rack: DeviceId): RackChain[] {
  return Object.values(p.rack_chains)
    .filter((c) => c.rack === rack)
    .sort(byOrder);
}

function devicesOn(p: Pick<Project, "devices">, chain: RackChainId): Device[] {
  return Object.values(p.devices).filter((d) => d.chain === chain);
}

/** Rack levels a device holds, itself included (0: not a rack). */
function height(p: Pick<Project, "devices" | "rack_chains" | "drum_pads">, d: Device, depth = 0): number {
  if (depth > MAX_RACK_DEPTH) return MAX_RACK_DEPTH + 1;
  const inner = isChainRack(d)
    ? chainsOf(p, d.id).flatMap((c) => devicesOn(p, c.id))
    : d.kind.type === "Builtin" && d.kind.device.type === "DrumRack"
      ? Object.values(p.devices).filter((x) => x.pad != null && p.drum_pads[x.pad]?.rack === d.id)
      : null;
  if (inner === null) return 0;
  return 1 + Math.max(0, ...inner.map((x) => height(p, x, depth + 1)));
}

/**
 * Whether chain `id` may go onto rack `target` (mirrors `Rack::CopyChain`'s checks): never in
 * a rack it holds (or under it), every device fitting the rack (audio effect racks take audio
 * effects, MIDI effect racks MIDI effects, a drum rack only an instrument rack not under a drum
 * pad), and no deeper than `MAX_RACK_DEPTH`.
 */
export function canPlaceChain(
  p: Pick<Project, "devices" | "rack_chains" | "drum_pads">,
  id: RackChainId,
  target: DeviceId,
  categoryOf: (d: Device) => DeviceCategory,
): boolean {
  const rack = p.devices[target];
  const type = rack ? rackTypeOf(rack) : null;
  if (!rack || !type || !p.rack_chains[id]) return false;
  // Up from the target: none of its enclosing devices may sit on this chain.
  let underPad = false;
  let d: Device | undefined = rack;
  for (let i = 0; d && i <= MAX_RACK_DEPTH + 1; i++) {
    if (d.chain === id) return false;
    if (d.pad != null) underPad = true;
    const up: DeviceId | undefined = d.chain != null ? p.rack_chains[d.chain]?.rack : d.pad != null ? p.drum_pads[d.pad]?.rack : undefined;
    d = up === undefined ? undefined : p.devices[up];
  }
  const level = rackLevel(p, target);
  return devicesOn(p, id).every((x) => {
    const drum = x.kind.type === "Builtin" && x.kind.device.type === "DrumRack";
    if (drum && (type !== "InstrumentRack" || underPad)) return false;
    if (level + height(p, x) > MAX_RACK_DEPTH) return false;
    const category = categoryOf(x);
    if (type === "AudioEffectRack") return category === "AudioEffect";
    if (type === "MidiEffectRack") return category === "NoteEffect";
    return true;
  });
}

/**
 * Puts chain `id` onto `rack` before `before` (`null`: last): a move (`copy` false) in its own
 * rack reorders it; anywhere else it is copied there and, for a move, removed from its rack.
 * Says why when the engine refuses. Returns the chain's id at its new place (`null`: refused).
 */
export async function placeChain(
  transport: EngineTransport,
  id: RackChainId,
  rack: DeviceId,
  before: RackChainId | null,
  copy: boolean,
): Promise<RackChainId | null> {
  const chain = useProjectStore.getState().project?.rack_chains[id];
  if (!chain) return null;
  try {
    if (!copy && chain.rack === rack) {
      if (before !== id) await transport.send(cmd("Rack", { type: "MoveChain", id, before }));
      return id;
    }
    const new_id = newId();
    await transport.send(cmd("Rack", { type: "CopyChain", id, new_id, rack, before, cut: !copy }));
    return new_id;
  } catch (e) {
    notify("Error", t(copy ? "racks.clipboard.paste_failed" : "racks.clipboard.drop_failed", { name: chain.name, error: errorMessage(e) }));
    return null;
  }
}

/** The chain after `id` in its rack (`null`: it is the last). */
export function nextChain(p: Pick<Project, "rack_chains">, id: RackChainId): RackChainId | null {
  const chain = p.rack_chains[id];
  if (!chain) return null;
  const list = chainsOf(p, chain.rack);
  return list[list.findIndex((c) => c.id === id) + 1]?.id ?? null;
}

/** Duplicates chain `id` right after itself (one undo step). */
export async function duplicateChain(transport: EngineTransport, id: RackChainId): Promise<void> {
  const p = useProjectStore.getState().project;
  const chain = p?.rack_chains[id];
  if (!p || !chain) return;
  const copy = await placeChain(transport, id, chain.rack, nextChain(p, id), true);
  if (copy) useChainClipboard.getState().select(copy);
}

/**
 * Pastes the copied chain onto `rack`, before `before` (`null`: last); a cut chain moves there
 * (and the clipboard empties). Says why when there is nothing to paste.
 */
export async function pasteChain(transport: EngineTransport, rack: DeviceId, before: RackChainId | null): Promise<void> {
  const { copied } = useChainClipboard.getState();
  const p = useProjectStore.getState().project;
  if (!p) return;
  if (!copied || !p.rack_chains[copied.id]) {
    notify("Warning", t("racks.clipboard.nothing_to_paste"));
    return;
  }
  const placed = await placeChain(transport, copied.id, rack, before, !copied.cut);
  if (!placed) return;
  if (copied.cut) useChainClipboard.setState({ copied: null });
  useChainClipboard.getState().select(placed);
}

/** Pastes after chain `target` (in its rack). */
export function pasteAfter(transport: EngineTransport, target: RackChainId): Promise<void> {
  const p = useProjectStore.getState().project;
  const chain = p?.rack_chains[target];
  if (!p || !chain) return Promise.resolve();
  return pasteChain(transport, chain.rack, nextChain(p, target));
}

/** The chain row holding the focus (not while renaming it), if any. */
export function focusedChain(): RackChainId | null {
  const el = document.activeElement;
  if (!(el instanceof HTMLElement) || el.closest("input, textarea, [contenteditable='true']")) return null;
  return el.closest<HTMLElement>(".eth-rack__row[data-chain]")?.dataset.chain ?? null;
}

const current: { transport: EngineTransport | null } = { transport: null };
let bound = false;

/**
 * Called by every rack's chain list: binds the document's copy / cut / paste events once (for
 * the app's lifetime; they act while a chain row has the focus).
 */
export function useChainClipboardKeys(transport: EngineTransport): void {
  current.transport = transport;
  if (bound || typeof document === "undefined") return;
  bound = true;
  const clip = (cut: boolean) => (e: ClipboardEvent) => {
    const id = focusedChain();
    if (!id) return;
    e.preventDefault();
    useChainClipboard.getState().copy(id, cut);
  };
  document.addEventListener("copy", clip(false));
  document.addEventListener("cut", clip(true));
  document.addEventListener("paste", (e) => {
    const id = focusedChain();
    if (!id || !current.transport) return;
    e.preventDefault();
    void pasteAfter(current.transport, id);
  });
}

/** Index of the drop line among a list's chain rows: before the first row whose middle is below `y`. */
export function dropIndex(rows: ReadonlyArray<Pick<DOMRect, "top" | "height">>, y: number): number {
  const i = rows.findIndex((r) => y < r.top + r.height / 2);
  return i < 0 ? rows.length : i;
}

/** Scrolls the nearest scrolling ancestor of `el` while a drag is near its top or bottom edge. */
export function autoscroll(el: HTMLElement, y: number): void {
  let s: HTMLElement | null = el.parentElement;
  while (s && !(s.scrollHeight > s.clientHeight && /(auto|scroll)/.test(getComputedStyle(s).overflowY))) s = s.parentElement;
  if (!s) return;
  const r = s.getBoundingClientRect();
  const edge = 32;
  if (y < r.top + edge) s.scrollTop -= 12;
  else if (y > r.bottom - edge) s.scrollTop += 12;
}
