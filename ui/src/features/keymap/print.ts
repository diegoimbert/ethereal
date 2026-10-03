import { create } from "zustand";
import { derivePaletteActions } from "./palette";

export const usePrinting = create<{ printing: boolean }>()(() => ({ printing: false }));

/** Body class while the cheat sheet prints (keymap.css hides everything else). */
export const PRINTING_CLASS = "eth-keymap-printing";

/**
 * Print the cheat sheet: every bound action with its chords, for this keymap and platform.
 * `CheatSheetPrinter` renders the sheet, opens the print dialog and removes it afterwards.
 */
export function printCheatSheet(): void {
  derivePaletteActions();
  usePrinting.setState({ printing: true });
}
