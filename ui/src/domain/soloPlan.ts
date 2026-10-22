/**
 * Exclusive solo (Ableton-like), shared by every Solo button: arranger track header, inspector,
 * rack chain rows and drum rack pads.
 *
 * - Plain click: the clicked item becomes the ONLY soloed item of its scope (soloed or not),
 *   every other soloed item is unsoloed. Exception: when it already is the only soloed item, the
 *   click unsolos it (otherwise solo could not be cleared without a modifier).
 * - Ctrl/Cmd-click: additive toggle of the clicked item, the others untouched.
 */

export interface SoloItem<Id = string> {
  id: Id;
  solo: boolean;
}

export interface SoloPlan<Id = string> {
  /** The state to set on each item that changes (the clicked item first). */
  changes: SoloItem<Id>[];
  /** The clicked item is soloed with every other item of the scope unsoloed. */
  exclusive: boolean;
}

/**
 * Solo overrides the soloed item's own mute (CONTRACTS.md §12.10): a muted track, rack chain or
 * drum pad that is soloed is heard while soloed; its mute is kept and applies again once it is
 * unsoloed. Only the item's own solo counts (a soloed group doesn't unmute its muted children).
 * Mute buttons show it with `data-solo-override` and the `mixer.mute_overridden` tooltip.
 */
export const muteOverriddenBySolo = (mute: boolean, solo: boolean): boolean => mute && solo;

/** The mute that is applied (`TrackMixer::effective_mute` in the controller): mute unless soloed. */
export const effectiveMute = (mute: boolean, solo: boolean): boolean => mute && !solo;

/** True when the click should be additive (Ctrl or Cmd held). */
export const additiveClick = (e?: { ctrlKey?: boolean; metaKey?: boolean }): boolean => !!(e?.ctrlKey || e?.metaKey);

/** `items` is the whole scope (including the clicked item) with its current solo state. */
export function planSolo<Id>(items: readonly SoloItem<Id>[], clicked: Id, additive: boolean): SoloPlan<Id> {
  const self = items.find((i) => i.id === clicked);
  const wasSolo = self?.solo ?? false;
  if (additive) return { changes: [{ id: clicked, solo: !wasSolo }], exclusive: false };
  const others = items.filter((i) => i.id !== clicked && i.solo);
  if (wasSolo && others.length === 0) return { changes: [{ id: clicked, solo: false }], exclusive: false };
  const changes: SoloItem<Id>[] = [];
  if (!wasSolo) changes.push({ id: clicked, solo: true });
  for (const o of others) changes.push({ id: o.id, solo: false });
  return { changes, exclusive: true };
}
