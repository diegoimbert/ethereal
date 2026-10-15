/**
 * Pure helpers of the drum rack view: note names, pad banks (4×4 grids of keys), pads by
 * note, slice helpers and command builders. Semantics: CONTRACTS.md §11.12.
 */

import type { Command, Device, DeviceId, SliceSettings, SlicePadIds, TrackId } from "@/generated";
import { t } from "@/i18n";
import { cmd, newId } from "@/transport";

const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/** MIDI note name with C3 = 60 (Ableton convention): 36 → "C1". */
export function noteName(note: number): string {
  return `${NOTE_NAMES[note % 12]}${Math.floor(note / 12) - 2}`;
}

export const PADS_PER_BANK = 16;
export const GRID_COLUMNS = 4;
/** The default bank starts on C1 (36), the usual first drum pad. */
export const DEFAULT_BANK_START = 36;

/** First keys of the selectable banks (…, 20, 36, 52, …), all within 0..=127. */
export function bankStarts(): number[] {
  const out: number[] = [];
  for (let s = DEFAULT_BANK_START % PADS_PER_BANK; s <= 127; s += PADS_PER_BANK) out.push(s);
  return out;
}

/** The bank containing `note`. */
export function bankOf(note: number): number {
  const starts = bankStarts();
  return [...starts].reverse().find((s) => s <= note) ?? starts[0]!;
}

/**
 * Keys of a bank's 4×4 grid in display order (row by row, top row first): like Ableton,
 * the lowest key is bottom-left and keys rise left to right, then upwards. Keys past 127
 * are `null` (empty cells).
 */
export function gridNotes(bankStart: number): (number | null)[] {
  const rows = PADS_PER_BANK / GRID_COLUMNS;
  const out: (number | null)[] = [];
  for (let r = rows - 1; r >= 0; r--) {
    for (let c = 0; c < GRID_COLUMNS; c++) {
      const n = bankStart + r * GRID_COLUMNS + c;
      out.push(n <= 127 ? n : null);
    }
  }
  return out;
}

export function isDrumRack(d: Device): boolean {
  return d.kind.type === "Builtin" && d.kind.device.type === "DrumRack";
}

export function isSampler(d: Device): d is Device & { kind: { type: "Builtin"; device: { type: "Sampler" } } } {
  return d.kind.type === "Builtin" && d.kind.device.type === "Sampler";
}

/** Slice settings of a sampler device (`null` for other devices). */
export function slicesOf(d: Device): SliceSettings | null {
  return d.kind.type === "Builtin" && d.kind.device.type === "Sampler" ? d.kind.device.slices : null;
}

/** Sample of a sampler device (`null` for none / other devices). */
export function sampleOf(d: Device): string | null {
  return d.kind.type === "Builtin" && d.kind.device.type === "Sampler" ? d.kind.device.sample : null;
}

/** Racks on a track's own chain, in chain order. */
export function racksOf(chain: Device[]): Device[] {
  return chain.filter(isDrumRack);
}

/** Playable slices: markers from `base_note` up to key 127. */
export function playableSlices(s: SliceSettings): number {
  return Math.max(0, Math.min(s.markers.length, 128 - s.base_note));
}

/** Client-chosen ids for `SliceCommand::ToDrumRack` (one pad + one sampler per slice). */
export function slicePadIds(count: number): SlicePadIds[] {
  return Array.from({ length: count }, () => ({
    pad: newId(),
    device: newId(),
  }));
}

/** Choke group choices: none, 1..16 (call at render: the "none" label follows the language). */
export function chokeOptions(): { value: string; label: string }[] {
  return [
    { value: "", label: t("drum_rack.pad.choke_none") },
    ...Array.from({ length: 16 }, (_, i) => ({
      value: String(i + 1),
      label: String(i + 1),
    })),
  ];
}

/** A new drum rack at the front of `track`'s chain (instruments go first). */
export function insertRackCommand(track: TrackId, id: DeviceId, before: DeviceId | null): Command {
  return cmd("Device", {
    type: "Insert",
    id,
    track,
    device: { type: "Builtin", device: { type: "DrumRack" } },
    before,
  });
}
