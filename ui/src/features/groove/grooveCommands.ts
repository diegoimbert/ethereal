/**
 * Groove commands and the remembered quantize/humanize settings (piano-roll popovers and
 * Cmd/Ctrl+U share them). Semantics: CONTRACTS.md §11.11.
 */

import { create } from "zustand";
import type { Beats, ClipId, Command, NoteId } from "@/generated";
import { MOD_KEY, type ContextMenuEntry } from "@/kit";
import { cmd } from "@/transport";

/** Quantize grid choices; `"roll"` = the piano roll's current grid step. */
export const QUANTIZE_GRIDS = [
  { value: "roll", label: "Roll grid", beats: null },
  { value: "1/4", label: "1/4", beats: 1 },
  { value: "1/8", label: "1/8", beats: 1 / 2 },
  { value: "1/8T", label: "1/8T", beats: 1 / 3 },
  { value: "1/16", label: "1/16", beats: 1 / 4 },
  { value: "1/16T", label: "1/16T", beats: 1 / 6 },
  { value: "1/32", label: "1/32", beats: 1 / 8 },
] as const;
export type QuantizeGrid = (typeof QUANTIZE_GRIDS)[number]["value"];

/** Project swing grid choices (`ProjectSettings::swing_grid`). */
export const SWING_GRIDS = [
  { value: "1/8", label: "1/8", beats: 1 / 2 },
  { value: "1/16", label: "1/16", beats: 1 / 4 },
] as const;
export type SwingGrid = (typeof SWING_GRIDS)[number]["value"];

/** A sixteenth note: the unit of the humanize timing amount (100% = ±1/16). */
export const HUMANIZE_TIMING_UNIT: Beats = 1 / 4;

export interface QuantizeSettings {
  grid: QuantizeGrid;
  /** 0..1 */
  strength: number;
  /** 0..1 */
  swing: number;
  ends: boolean;
}

export interface HumanizeSettings {
  /** 0..1 of `HUMANIZE_TIMING_UNIT`. */
  timing: number;
  /** 0..1 (velocity range). */
  velocity: number;
}

export const DEFAULT_QUANTIZE: QuantizeSettings = { grid: "roll", strength: 1, swing: 0, ends: false };
export const DEFAULT_HUMANIZE: HumanizeSettings = { timing: 0.1, velocity: 0.1 };

interface GrooveSettingsState {
  quantize: QuantizeSettings;
  humanize: HumanizeSettings;
  setQuantize(patch: Partial<QuantizeSettings>): void;
  setHumanize(patch: Partial<HumanizeSettings>): void;
  reset(): void;
}

/** Session-local (not saved) quantize/humanize settings. */
export const useGrooveSettings = create<GrooveSettingsState>()((set) => ({
  quantize: DEFAULT_QUANTIZE,
  humanize: DEFAULT_HUMANIZE,
  setQuantize: (patch) => set((s) => ({ quantize: { ...s.quantize, ...patch } })),
  setHumanize: (patch) => set((s) => ({ humanize: { ...s.humanize, ...patch } })),
  reset: () => set({ quantize: DEFAULT_QUANTIZE, humanize: DEFAULT_HUMANIZE }),
}));

/** Grid length in beats for a quantize grid choice (`rollStep` for `"roll"`). */
export function quantizeGridBeats(grid: QuantizeGrid, rollStep: Beats): Beats {
  return QUANTIZE_GRIDS.find((g) => g.value === grid)?.beats ?? rollStep;
}

const targets = (selected: ReadonlyArray<NoteId>): NoteId[] | null => (selected.length > 0 ? [...selected] : null);

/** `NoteCommand::Quantize` for the selected notes (all notes of the clip if none). */
export function grooveQuantizeCommand(
  clip: ClipId,
  selected: ReadonlyArray<NoteId>,
  settings: QuantizeSettings,
  rollStep: Beats,
): Command {
  return cmd("Note", {
    type: "Quantize",
    clip,
    notes: targets(selected),
    grid: quantizeGridBeats(settings.grid, rollStep),
    strength: settings.strength,
    ends: settings.ends,
    swing: settings.swing,
  });
}

/** A fresh humanize seed (u32). */
export function newSeed(): number {
  return Math.floor(Math.random() * 0x1_0000_0000) >>> 0;
}

/** `GrooveCommand::Humanize` for the selected notes (all notes of the clip if none). */
export function humanizeCommand(
  clip: ClipId,
  selected: ReadonlyArray<NoteId>,
  settings: HumanizeSettings,
  seed: number,
): Command {
  return cmd("Groove", {
    type: "Humanize",
    clip,
    notes: targets(selected),
    timing: settings.timing * HUMANIZE_TIMING_UNIT,
    velocity: settings.velocity,
    seed: seed >>> 0,
  });
}

/** `GrooveCommand::SetSwing` (project playback swing). */
export function setSwingCommand(amount: number, grid: Beats): Command {
  return cmd("Groove", { type: "SetSwing", amount: Math.min(1, Math.max(0, amount)), grid });
}

/**
 * Note context-menu items (piano roll): Quantize and Humanize the clicked selection with
 * the remembered settings. Each is one command = one undo step.
 */
export function grooveMenuItems(
  clip: ClipId,
  ids: ReadonlyArray<NoteId>,
  rollStep: Beats,
  send: (command: Command) => unknown,
): ContextMenuEntry[] {
  const { quantize, humanize } = useGrooveSettings.getState();
  return [
    {
      label: "Quantize",
      shortcut: `${MOD_KEY}U`,
      onSelect: () => void send(grooveQuantizeCommand(clip, ids, quantize, rollStep)),
    },
    { label: "Humanize", onSelect: () => void send(humanizeCommand(clip, ids, humanize, newSeed())) },
  ];
}

/** The swing grid choice for `beats` (closest match, default 1/16). */
export function swingGridOf(beats: Beats): SwingGrid {
  return SWING_GRIDS.find((g) => Math.abs(g.beats - beats) < 1e-6)?.value ?? "1/16";
}
