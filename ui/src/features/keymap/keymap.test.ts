/** Keymap: chords, the registry and presets, lookups, palette actions, the dispatcher. */
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Keymap } from "@/generated";
import { eventChords, formatChord, isValidChord, normalizeChord, parseShortcutHint } from "./chords";
import { conflictsFor, findConflicts } from "./conflicts";
import { dispatchKey } from "./dispatcher";
import { derivePaletteActions, paletteActionId, paletteShortcut, setPaletteSource, type PaletteEntry } from "./palette";
import { BUILTIN_ACTIONS, registerActions, registeredAction } from "./registry";
import {
  allActions,
  chordsFor,
  DEFAULT_KEYMAP,
  effectiveChords,
  firstMatch,
  matchesAction,
  presetChords,
  setPaletteActions,
  useKeymapStore,
  withBinding,
  withPreset,
} from "./store";

const key = (k: string, mods: Partial<{ metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; altKey: boolean; code: string }> = {}) => ({
  key: k,
  metaKey: false,
  ctrlKey: false,
  shiftKey: false,
  altKey: false,
  ...mods,
});

function setKeymap(keymap: Keymap) {
  useKeymapStore.setState({ keymap });
}

afterEach(() => {
  setKeymap(DEFAULT_KEYMAP);
  setPaletteSource(null);
  setPaletteActions([]);
  vi.restoreAllMocks();
});

describe("chords", () => {
  it("validates like the engine", () => {
    for (const ok of ["A", "Mod+Shift+D", "Mod+Ctrl+Alt+Shift+Z", "Space", "F24", "Mod+,", "Shift+/", "9", "Mod+Shift+Backspace"]) expect(isValidChord(ok), ok).toBe(true);
    for (const bad of ["", "a", "Shift+Mod+D", "Mod+Mod+D", "Mod+", "F25", "Cmd+D", "Mod+?", "Insert"]) expect(isValidChord(bad), bad).toBe(false);
  });

  it("reads key events (Ctrl and Cmd are Mod off macOS; shifted keys by their physical key)", () => {
    expect(eventChords(key("d", { ctrlKey: true }), false)).toEqual(["Mod+D"]);
    expect(eventChords(key("d", { metaKey: true }), false)).toEqual(["Mod+D"]);
    expect(eventChords(key("L", { metaKey: true, shiftKey: true }), false)).toEqual(["Mod+Shift+L"]);
    expect(eventChords(key(" "), false)).toEqual(["Space"]);
    expect(eventChords(key("?", { shiftKey: true, code: "Slash" }), false)).toEqual(["Shift+/"]);
    expect(eventChords(key("∂", { altKey: true, code: "KeyD" }), true)).toEqual(["Alt+D"]);
    expect(eventChords(key("Shift", { shiftKey: true, code: "ShiftLeft" }), false)).toEqual([]);
    // macOS: Cmd is Mod, Control is Ctrl, with the Mod form as a fallback.
    expect(eventChords(key("d", { metaKey: true }), true)).toEqual(["Mod+D"]);
    expect(eventChords(key("y", { ctrlKey: true }), true)).toEqual(["Ctrl+Y", "Mod+Y"]);
  });

  it("formats per platform and parses palette hints", () => {
    expect(formatChord("Mod+Shift+D", true)).toBe("⇧⌘D");
    expect(formatChord("Mod+Shift+D", false)).toBe("⇧Ctrl+D");
    expect(formatChord("Ctrl+Y", false)).toBe("Ctrl+Y");
    expect(formatChord("Ctrl+Y", true)).toBe("⌃Y");
    expect(formatChord("Backspace", true)).toBe("⌫");
    expect(normalizeChord("Ctrl+Alt+K", false)).toBe("Mod+Alt+K");
    expect(parseShortcutHint("⇧⌘M")).toBe("Mod+Shift+M");
    expect(parseShortcutHint("⇧Ctrl+L")).toBe("Mod+Shift+L");
    expect(parseShortcutHint("Ctrl+Shift+L")).toBe("Mod+Shift+L");
    expect(parseShortcutHint("⌘I")).toBe("Mod+I");
    expect(parseShortcutHint("Space")).toBe("Space");
    expect(parseShortcutHint("⌫")).toBe("Backspace");
    expect(parseShortcutHint("whatever")).toBeNull();
  });
});

describe("registry and presets", () => {
  it("has unique ids and valid chords", () => {
    const ids = BUILTIN_ACTIONS.map((a) => a.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const a of BUILTIN_ACTIONS) for (const c of [...a.chords, ...(a.ableton ?? [])]) expect(isValidChord(c), `${a.id} ${c}`).toBe(true);
  });

  it("Ethereal is exactly the app's shortcuts (no behaviour change by default)", () => {
    const defaults = Object.fromEntries(BUILTIN_ACTIONS.filter((a) => a.chords.length).map((a) => [a.id, [...a.chords]]));
    expect(defaults).toEqual({
      "transport.play": ["Space"],
      "edit.undo": ["Mod+Z"],
      "edit.redo": ["Mod+Shift+Z", "Ctrl+Y"],
      "edit.copy": ["Mod+C"],
      "edit.cut": ["Mod+X"],
      "edit.paste": ["Mod+V"],
      "edit.duplicate": ["Mod+D"],
      "edit.delete": ["Backspace", "Delete"],
      "edit.selectAll": ["Mod+A"],
      "edit.deselect": ["Escape"],
      "edit.split": ["Mod+E"],
      "clip.toggleLoop": ["Mod+Shift+L"],
      "track.group": ["Mod+G"],
      "track.ungroup": ["Mod+Shift+G"],
      "take.previous": ["ArrowUp"],
      "take.next": ["ArrowDown"],
      "time.cut": ["Mod+Shift+X"],
      "time.copy": ["Mod+Shift+C"],
      "time.paste": ["Mod+Shift+V"],
      "time.duplicate": ["Mod+Shift+D"],
      "time.insertSilence": ["Mod+Shift+I"],
      "time.delete": ["Mod+Shift+Backspace", "Mod+Shift+Delete"],
      "pianoRoll.quantize": ["Mod+U"],
      "pianoRoll.drawMode": ["B"],
      "nudge.up": ["ArrowUp"],
      "nudge.down": ["ArrowDown"],
      "nudge.left": ["ArrowLeft"],
      "nudge.right": ["ArrowRight"],
      "pianoRoll.octaveUp": ["Shift+ArrowUp"],
      "pianoRoll.octaveDown": ["Shift+ArrowDown"],
      "automation.fineUp": ["Shift+ArrowUp"],
      "automation.fineDown": ["Shift+ArrowDown"],
      "automation.fineLeft": ["Shift+ArrowLeft"],
      "automation.fineRight": ["Shift+ArrowRight"],
      "palette.open": ["Mod+K"],
      "project.save": ["Mod+S"],
      "file.import": ["Mod+I"],
      "view.editor": ["Mod+J"],
      "collab.focusChat": ["Mod+Shift+M"],
    });
  });

  it("neither preset has conflicts, on either platform", () => {
    for (const preset of ["Ethereal", "AbletonLike"] as const)
      for (const mac of [true, false]) expect([...findConflicts(BUILTIN_ACTIONS, { preset, overrides: [] }, mac).keys()], `${preset} mac=${mac}`).toEqual([]);
  });

  it("Ableton-like keeps every default chord and only adds", () => {
    for (const a of BUILTIN_ACTIONS) {
      const ableton = presetChords(a, "AbletonLike");
      expect(ableton.slice(0, a.chords.length)).toEqual([...a.chords]);
    }
    expect(presetChords(registeredAction("transport.record")!, "AbletonLike")).toEqual(["F9"]);
    expect(presetChords(registeredAction("view.editor")!, "AbletonLike")).toEqual(["Mod+J", "Mod+Alt+L"]);
  });

  it("other features can register actions", () => {
    const off = registerActions([{ id: "test.thing", label: "Thing", group: "Test", scopes: ["global"], chords: ["Mod+Alt+T"], handled: true }]);
    expect(matchesAction("test.thing", key("t", { ctrlKey: true, altKey: true }))).toBe(true);
    off();
    expect(matchesAction("test.thing", key("t", { ctrlKey: true, altKey: true }))).toBe(false);
  });
});

describe("lookups", () => {
  it("match the default chords", () => {
    expect(matchesAction("edit.duplicate", key("d", { ctrlKey: true }))).toBe(true);
    expect(matchesAction("edit.duplicate", key("d", { ctrlKey: true, shiftKey: true }))).toBe(false);
    expect(firstMatch(["time.duplicate", "edit.duplicate"], key("D", { ctrlKey: true, shiftKey: true }))).toBe("time.duplicate");
    expect(matchesAction("edit.redo", key("y", { ctrlKey: true }))).toBe(true);
    expect(matchesAction("transport.play", key(" "))).toBe(true);
  });

  it("follow the user's overrides and preset", () => {
    setKeymap(withBinding(DEFAULT_KEYMAP, "edit.duplicate", ["Alt+D"]));
    expect(matchesAction("edit.duplicate", key("d", { ctrlKey: true }))).toBe(false);
    expect(matchesAction("edit.duplicate", key("∂", { altKey: true, code: "KeyD" }))).toBe(true);
    setKeymap(withBinding(useKeymapStore.getState().keymap, "transport.play", []));
    expect(matchesAction("transport.play", key(" "))).toBe(false);
    setKeymap(withPreset(useKeymapStore.getState().keymap, "AbletonLike"));
    expect(matchesAction("transport.record", key("F9"))).toBe(true);
    expect(chordsFor("edit.duplicate")).toEqual(["Alt+D"]);
  });

  it("on macOS a Control chord falls back to Cmd unless something binds it", () => {
    vi.spyOn(navigator, "platform", "get").mockReturnValue("MacIntel");
    expect(matchesAction("edit.duplicate", key("d", { ctrlKey: true }))).toBe(true);
    expect(matchesAction("edit.redo", key("y", { ctrlKey: true }))).toBe(true);
    expect(matchesAction("edit.undo", key("z", { metaKey: true }))).toBe(true);
    setKeymap(withBinding(DEFAULT_KEYMAP, "transport.loop", ["Ctrl+D"]));
    expect(matchesAction("edit.duplicate", key("d", { ctrlKey: true }))).toBe(false);
  });

  it("withBinding keeps overrides sorted, unique and drops ones equal to the preset", () => {
    let k = withBinding(DEFAULT_KEYMAP, "time.copy", ["F2"]);
    k = withBinding(k, "edit.copy", ["F1"]);
    k = withBinding(k, "edit.copy", ["F3", "F3"]);
    expect(k.overrides).toEqual([
      { action: "edit.copy", chords: ["F3"] },
      { action: "time.copy", chords: ["F2"] },
    ]);
    k = withBinding(k, "edit.copy", ["Mod+C"]);
    expect(k.overrides.map((o) => o.action)).toEqual(["time.copy"]);
    k = withBinding(k, "time.copy", null);
    expect(k).toEqual(DEFAULT_KEYMAP);
    // An override that the new preset already gives is dropped.
    k = withPreset(withBinding(DEFAULT_KEYMAP, "transport.record", ["F9"]), "AbletonLike");
    expect(k).toEqual({ preset: "AbletonLike", overrides: [] });
  });
});

describe("conflicts", () => {
  it("are shown on both actions when scopes overlap", () => {
    const k = withBinding(DEFAULT_KEYMAP, "transport.metronome", ["Mod+D"]);
    const c = findConflicts(BUILTIN_ACTIONS, k, false);
    expect(c.get("transport.metronome")?.[0]?.with.map((a) => a.id)).toEqual(["edit.duplicate"]);
    expect(c.get("edit.duplicate")?.[0]?.with.map((a) => a.id)).toEqual(["transport.metronome"]);
  });

  it("not across disjoint scopes (arrows in the arrangement vs the piano roll)", () => {
    expect(conflictsFor(registeredAction("take.previous")!, "ArrowUp", BUILTIN_ACTIONS, DEFAULT_KEYMAP, false)).toEqual([]);
    expect(conflictsFor(registeredAction("pianoRoll.drawMode")!, "Mod+D", BUILTIN_ACTIONS, DEFAULT_KEYMAP, false).map((a) => a.id)).toEqual(["edit.duplicate"]);
  });
});

/** Commands as other (in-flight) features add them to the palette. */
function paletteWith(extra: PaletteEntry[]) {
  const runs: string[] = [];
  const entries: PaletteEntry[] = [
    { id: "transport:record", group: "Transport", label: "Record", run: () => runs.push("record") },
    { id: "edit:undo", group: "Edit", label: "Undo", shortcut: "⌘Z", run: () => runs.push("undo") },
    { id: "goto:t1", group: "Go to", label: "Go to track A", bindable: false, run: () => runs.push("goto") },
    ...extra.map((e) => ({ ...e, run: () => runs.push(e.id) })),
  ];
  setPaletteSource(() => entries);
  return { runs, entries };
}

describe("palette commands are actions", () => {
  it("linked to built-ins by id, palette id or default chord; others become their own actions", () => {
    const { entries } = paletteWith([
      { id: "ai:ask", group: "AI", label: "Ask AI", shortcut: "⇧⌘L", run: () => {} },
      { id: "project:save", group: "Project", label: "Save", shortcut: "⌘S", run: () => {} },
      { id: "history:checkpoint", group: "Edit", label: "Name checkpoint…", run: () => {} },
    ]);
    derivePaletteActions();
    expect(paletteActionId(entries[0]!)).toBe("transport.record");
    expect(paletteActionId(entries[1]!)).toBe("edit.undo");
    expect(paletteActionId(entries[3]!)).toBe("ai:ask");
    expect(paletteActionId(entries[4]!)).toBe("project.save");
    const ids = allActions().map((a) => a.id);
    expect(ids).toContain("ai:ask");
    expect(ids).toContain("history:checkpoint");
    expect(ids).not.toContain("goto:t1");
    expect(ids).not.toContain("project:save");
    // The hint is the derived action's default; the palette shows the keymap's chord.
    expect(effectiveChords(allActions().find((a) => a.id === "ai:ask")!, DEFAULT_KEYMAP)).toEqual(["Mod+Shift+L"]);
    expect(paletteShortcut(entries[3]!)).toBe("⇧Ctrl+L");
    setKeymap(withBinding(DEFAULT_KEYMAP, "ai:ask", ["F6"]));
    expect(paletteShortcut(entries[3]!)).toBe("F6");
    setKeymap(withBinding(DEFAULT_KEYMAP, "edit.undo", []));
    expect(paletteShortcut(entries[1]!)).toBeUndefined();
  });

  it("a palette command's default chord that clashes with a built-in shows as a conflict", () => {
    paletteWith([{ id: "ai:ask", group: "AI", label: "Ask AI", shortcut: "⇧⌘L", run: () => {} }]);
    derivePaletteActions();
    const c = findConflicts(allActions(), DEFAULT_KEYMAP, false);
    expect(c.get("ai:ask")?.[0]?.with.map((a) => a.id)).toEqual(["clip.toggleLoop"]);
  });
});

describe("dispatcher", () => {
  const press = (k: string, init: KeyboardEventInit = {}) => {
    const e = new KeyboardEvent("keydown", { key: k, cancelable: true, bubbles: true, ...init });
    document.body.dispatchEvent(e);
    return { e, result: dispatchKey(e) };
  };

  it("does nothing with the default keymap", () => {
    const { runs } = paletteWith([{ id: "ai:ask", group: "AI", label: "Ask AI", shortcut: "⇧⌘L", run: () => {} }]);
    expect(press("L", { ctrlKey: true, shiftKey: true }).result).toBeNull();
    expect(press("F9").result).toBeNull();
    expect(runs).toEqual([]);
  });

  it("runs built-in palette actions on their chords (Ableton-like F9 = Record)", () => {
    const { runs } = paletteWith([]);
    setKeymap({ preset: "AbletonLike", overrides: [] });
    const { e, result } = press("F9");
    expect(result).toBe("ran");
    expect(e.defaultPrevented).toBe(true);
    expect(runs).toEqual(["record"]);
  });

  it("runs a palette command on a new chord, leaves its own chord to its handler, blocks it once unbound", () => {
    const { runs } = paletteWith([{ id: "join:open", group: "Session", label: "Join shared project…", shortcut: "⇧⌘J", run: () => {} }]);
    derivePaletteActions();
    setKeymap(withBinding(DEFAULT_KEYMAP, "join:open", ["Mod+Shift+J", "F6"]));
    expect(press("F6").result).toBe("ran");
    expect(press("J", { ctrlKey: true, shiftKey: true }).result).toBeNull();
    expect(runs).toEqual(["join:open"]);
    setKeymap(withBinding(DEFAULT_KEYMAP, "join:open", ["F6"]));
    const blocked = press("J", { ctrlKey: true, shiftKey: true });
    expect(blocked.result).toBe("blocked");
    expect(blocked.e.defaultPrevented).toBe(true);
  });

  it("never acts in text fields", () => {
    paletteWith([]);
    setKeymap({ preset: "AbletonLike", overrides: [] });
    const input = document.createElement("input");
    document.body.append(input);
    const e = new KeyboardEvent("keydown", { key: "F9", cancelable: true, bubbles: true });
    input.dispatchEvent(e);
    expect(dispatchKey(e)).toBeNull();
    input.remove();
  });
});
