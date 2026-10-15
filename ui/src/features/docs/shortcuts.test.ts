// The docs' shortcut list (shortcuts.ts) is the app's real keymap: every action the app binds
// by default (built-ins, actions features register, palette commands with their own shortcut)
// is in it with the same chords. A shortcut added anywhere fails here until the docs list it.
import { describe, expect, it } from "vitest";
import { buildCommands } from "@/app/shell/commands";
import "@/features/keyboard-midi/keyboard";
import { formatKeys } from "@/features/keymap/chords";
import { derivePaletteActions } from "@/features/keymap/palette";
import { allActions } from "@/features/keymap/store";
import { chordLabel, DOC_ACTIONS, keysLabel } from "./shortcuts";

const bound = (list: ReadonlyArray<{ id: string; chords: ReadonlyArray<string>; ableton?: ReadonlyArray<string> }>) =>
  Object.fromEntries(
    list.filter((a) => a.chords.length > 0 || (a.ableton?.length ?? 0) > 0).map((a) => [a.id, { chords: [...a.chords], ableton: [...(a.ableton ?? [])] }]),
  );

describe("docs shortcuts", () => {
  it("match every default shortcut of the app's keymap", () => {
    derivePaletteActions(buildCommands(null, []));
    expect(bound(DOC_ACTIONS)).toEqual(bound(allActions()));
  });

  it("read with the platform's modifiers", () => {
    expect(chordLabel("Mod+Shift+Z", "mac")).toBe("⇧⌘Z");
    expect(chordLabel("Mod+Shift+Z", "pc")).toBe("⇧Ctrl+Z");
    expect(keysLabel("Alt", "mac")).toBe("⌥");
    expect(keysLabel("Mod+Shift", "pc")).toBe("Ctrl+Shift");
    expect(keysLabel("Mod+Shift", "mac")).toBe("⇧⌘");
    expect(formatKeys("Ctrl+Mod", false)).toBe("Ctrl");
  });
});
