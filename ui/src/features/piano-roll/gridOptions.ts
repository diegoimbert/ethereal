/** Grid choices of the piano-roll toolbar. */

import { DEFAULT_GRID, type GridSetting } from "@/timeline";
import { t } from "@/i18n";

export interface GridOption {
  /** Called at render (the current language). */
  label: () => string;
  setting: GridSetting;
}

export const GRID_OPTIONS: ReadonlyArray<GridOption> = [
  { label: () => t("piano_roll.grid_option.adaptive_narrow"), setting: { type: "Adaptive", density: "narrow", triplet: false } },
  { label: () => t("piano_roll.grid_option.adaptive"), setting: DEFAULT_GRID },
  { label: () => t("piano_roll.grid_option.adaptive_wide"), setting: { type: "Adaptive", density: "wide", triplet: false } },
  { label: () => t("piano_roll.grid_option.one_bar"), setting: { type: "Fixed", step: { kind: "bars", bars: 1 }, triplet: false } },
  { label: () => "1/2", setting: { type: "Fixed", step: { kind: "beats", beats: 2 }, triplet: false } },
  { label: () => "1/4", setting: { type: "Fixed", step: { kind: "beats", beats: 1 }, triplet: false } },
  { label: () => "1/8", setting: { type: "Fixed", step: { kind: "beats", beats: 0.5 }, triplet: false } },
  { label: () => "1/16", setting: { type: "Fixed", step: { kind: "beats", beats: 0.25 }, triplet: false } },
  { label: () => "1/32", setting: { type: "Fixed", step: { kind: "beats", beats: 0.125 }, triplet: false } },
  { label: () => t("common.off"), setting: { type: "Off" } },
];
