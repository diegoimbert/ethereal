import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Command, Note } from "@/generated";
import { playheadStore, useEditorStore, useProjectStore } from "@/state";
import { itemSelection, wheelZoomFactor } from "@/timeline";
import { cmd, TransportProvider } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { DEFAULT_KEY_HEIGHT as KEY_H, MAX_KEY_HEIGHT } from "./geometry";
import { PianoRoll } from "./index";
import { pickOption } from "@/kit/testing";
import { resetPianoRollSection, usePianoRollSection } from "./section";
import { resetLastVelocity } from "./lastVelocity";
import { gridScene, velocityScene, type SceneItem } from "./noteLayout";
import { resetTimeSelection, useTimeSelection } from "@/features/time-edits/store";
import { useContextMenuStore, type ContextMenuItem } from "@/kit";
import { shortcutLabel } from "@/features/keymap";

// The piano roll's own view starts at 40 px/beat, scrolled to 0. jsdom has no layout: the
// grid's box is at (0, 0), so client coordinates are grid-local px. Notes are drawn on a
// canvas: presses on the grid are hit-tested against them, and `gridScene` tells what is drawn.
const PX = 40;
const x = (beats: number) => beats * PX;
const y = (pitch: number) => (127 - pitch) * KEY_H + KEY_H / 2;

let mock: MockTransport | undefined;
const store = () => useProjectStore.getState();
const notesOf = (clip: string): Note[] =>
  Object.values(store().project!.notes)
    .filter((n) => n.clip === clip)
    .sort((a, b) => a.start - b.start || a.pitch - b.pitch);

beforeEach(() => {
  // No canvas in jsdom (and no "not implemented" noise): the scene is checked instead.
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
});

afterEach(() => {
  vi.restoreAllMocks();
  mock?.dispose();
  mock = undefined;
  store().reset();
  useEditorStore.getState().close();
  itemSelection.getState().clear();
  resetLastVelocity();
});

async function flush() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

async function send(command: Command) {
  await act(async () => {
    await mock!.send(command);
  });
}

/** A fresh MIDI clip on the first MIDI track with two notes, opened in the piano roll. */
async function setup(opts: { open?: boolean } = {}) {
  mock = new MockTransport({ timers: "manual", seed: 1 });
  render(
    <TransportProvider transport={mock}>
      <PianoRoll />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  const track = Object.values(store().project!.tracks).find((t) => t.kind === "Midi")!;
  const clip = "01PIANOROLLTESTCLIP0000000";
  await send(cmd("Clip", { type: "CreateMidi", id: clip, track: track.id, start: 64, length: 8, name: "Test" }));
  await send(
    cmd("Note", {
      type: "Add",
      clip,
      notes: [
        { id: "01PIANOROLLNOTEA0000000000", pitch: 60, velocity: 0.8, start: 1, duration: 1 },
        { id: "01PIANOROLLNOTEB0000000000", pitch: 64, velocity: 0.5, start: 2, duration: 1 },
      ],
    }),
  );
  if (opts.open !== false) act(() => useEditorStore.getState().openClip(clip));
  // Fixed 1/4 grid for deterministic snapping.
  if (opts.open !== false) pickOption(screen.getByRole("combobox", { name: "grid" }), { value: "5" });
  return { clip, a: "01PIANOROLLNOTEA0000000000", b: "01PIANOROLLNOTEB0000000000" };
}

const grid = () => screen.getByTestId("piano-roll-grid");
type DrawnNote = Extract<SceneItem, { kind: "note" }>;
/** The notes drawn on the grid's canvas (grid px boxes and states). */
const drawnNotes = () => gridScene(grid()).notes.filter((i): i is DrawnNote => i.kind === "note");
const drawnNote = (id: string) => drawnNotes().find((n) => n.id === id);
/** A double-click at grid point (cx, cy): two presses, then the dblclick. */
async function doubleClickAt(cx: number, cy: number) {
  for (let i = 0; i < 2; i++) {
    fireEvent.pointerDown(grid(), { button: 0, clientX: cx, clientY: cy });
    fireEvent.pointerUp(window, { clientX: cx, clientY: cy });
    await flush();
  }
  fireEvent.doubleClick(grid(), { clientX: cx, clientY: cy });
}

async function drag(el: Element, from: [number, number], to: [number, number], init: MouseEventInit = {}) {
  fireEvent.pointerDown(el, { button: 0, clientX: from[0], clientY: from[1], ...init });
  const steps = 3;
  for (let i = 1; i <= steps; i++) {
    const px = from[0] + ((to[0] - from[0]) * i) / steps;
    const py = from[1] + ((to[1] - from[1]) * i) / steps;
    fireEvent.pointerMove(window, { clientX: px, clientY: py, ...init });
    await flush();
  }
  fireEvent.pointerUp(window, { clientX: to[0], clientY: to[1] });
  await flush();
}

const undo = () => send(cmd("Edit", { type: "Undo" }));

describe("PianoRoll", () => {
  /** A control of the scale popover (opens it first if needed). */
  function scaleControl(label: string): HTMLElement {
    const trigger = screen.getByTestId("scale-trigger");
    if (trigger.getAttribute("aria-expanded") !== "true") fireEvent.click(trigger);
    // Not a Select's list (it shares the Select's label while it animates out).
    return screen.getAllByLabelText(label).find((el) => el.getAttribute("role") !== "listbox")!;
  }
  const pickScale = async (label: string, value: string) => {
    pickOption(scaleControl(label), { value });
    await flush();
  };
  /** C minor with "scale notes only" on: rows are the scale's pitches, 127 down to 0. */
  async function foldToCMinor() {
    await send(cmd("Project", { type: "SetScale", scale: { root: 0, kind: "Minor" } }));
    fireEvent.click(scaleControl("scale notes only"));
    await flush();
  }
  const keys = () => [...document.querySelectorAll<HTMLElement>(".eth-pr-key")];
  /** Center of `pitch`'s row with folded rows (from the rendered keys). */
  const foldedY = (pitch: number) => keys().findIndex((key) => key.dataset.pitch === String(pitch)) * KEY_H + KEY_H / 2;

  it("edits project and custom track scales independently and can undo them", async () => {
    const { clip } = await setup();
    const track = store().project!.clips[clip]!.track;
    expect(screen.getByTestId("scale-trigger")).toHaveTextContent("scale…");
    expect(scaleControl("active scale type")).toHaveTextContent("chromatic / none");
    await pickScale("active scale type", "Minor");
    expect(store().project!.settings.scale).toEqual({ root: 0, kind: "Minor" });
    expect(screen.getByTestId("scale-trigger")).toHaveTextContent("C minor…");
    expect(screen.getByTestId("scale-popover")).toHaveTextContent("project");
    await pickScale("track scale mode", "Custom");
    await pickScale("active scale root", "9");
    expect(store().project!.tracks[track]!.scale).toEqual({ type: "Custom", scale: { root: 9, kind: "Minor" } });
    await send(cmd("Project", { type: "SetScale", scale: { root: 2, kind: "Major" } }));
    expect(scaleControl("active scale root")).toHaveTextContent("A");
    await pickScale("track scale mode", "FollowProject");
    expect(scaleControl("active scale root")).toHaveTextContent("D");
    await undo();
    expect(scaleControl("active scale root")).toHaveTextContent("A");
    await pickScale("track scale mode", "Chromatic");
    expect(scaleControl("active scale type")).toHaveTextContent("chromatic / none");
    expect(scaleControl("active scale type")).toBeDisabled();
  });

  it("keeps highlight and folding as local view state (no document edit)", async () => {
    await setup();
    await send(cmd("Project", { type: "SetScale", scale: { root: 0, kind: "Minor" } }));
    const before = store().project!;
    fireEvent.click(scaleControl("highlight"));
    fireEvent.click(scaleControl("scale notes only"));
    await flush();
    expect(store().project).toBe(before);
    expect(screen.getByTestId("scale-trigger")).toHaveAttribute("aria-pressed", "true");
  });

  it("highlights roots, dims outside notes and still accepts any pitch", async () => {
    const { clip, a, b } = await setup();
    await send(cmd("Project", { type: "SetScale", scale: { root: 0, kind: "Minor" } }));
    expect(drawnNote(a)!.tone).toBe("root");
    expect(drawnNote(b)!.tone).toBe("out");
    // Out of the scale: dimmed (the velocity opacity times the scale-out one).
    expect(drawnNote(b)!.opacity).toBeCloseTo((0.45 + 0.55 * 0.5) * 0.45);
    // A root's border is topped with the text color.
    expect(drawnNote(a)!.top).not.toEqual(drawnNote(a)!.border);
    expect(document.querySelector('.eth-pr-key[data-pitch="63"]')).toHaveAttribute("data-scale-tone", "in");
    await doublePress(x(4), y(61));
    expect(notesOf(clip).some((n) => n.pitch === 61)).toBe(true);
    fireEvent.click(scaleControl("highlight"));
    expect(drawnNote(a)!.tone).toBeUndefined();
    expect(drawnNote(b)!.tone).toBeUndefined();
  });

  it("folds keys and notes together, draws and drags at the displayed pitches, and restores hidden notes", async () => {
    const { clip, a, b } = await setup();
    await foldToCMinor();
    expect(keys().length).toBeLessThan(128);
    expect(document.querySelector('.eth-pr-key[data-pitch="64"]')).toBeNull();
    expect(drawnNote(b)).toBeUndefined();
    expect(notesOf(clip)).toHaveLength(2);
    expect(drawnNote(a)!.y).toBe(foldedY(60) - KEY_H / 2);
    await doublePress(x(4), foldedY(63));
    expect(notesOf(clip).some((n) => n.pitch === 63 && n.start === 4)).toBe(true);
    await drag(grid(), [x(1.5), foldedY(60)], [x(1.5), foldedY(62)]);
    expect(notesOf(clip).find((n) => n.id === a)!.pitch).toBe(62);
    await undo();
    expect(notesOf(clip).find((n) => n.id === a)!.pitch).toBe(60);
    fireEvent.click(scaleControl("scale notes only"));
    expect(keys()).toHaveLength(128);
    expect(drawnNote(b)).toBeDefined();
    expect(notesOf(clip).find((n) => n.id === b)!.pitch).toBe(64);
  });

  it("cmd-drag copies to the displayed row when folded; one row up is the next scale pitch", async () => {
    const { clip, a } = await setup();
    await foldToCMinor();
    act(() => itemSelection.getState().select("note", [a], "replace"));
    // One folded row up from C (60) is D (62), two semitones.
    await drag(grid(), [x(1.5), foldedY(60)], [x(3.5), foldedY(60) - KEY_H], { metaKey: true });
    const all = notesOf(clip);
    expect(all.find((n) => n.id === a)).toMatchObject({ start: 1, pitch: 60 });
    const copy = all.find((n) => n.id !== a && n.pitch !== 64)!;
    expect(copy).toMatchObject({ start: 3, pitch: 62 });
    expect([...itemSelection.getState().selected.note]).toEqual([copy.id]);
    await undo();
    expect(notesOf(clip)).toHaveLength(2);
  });

  it("maps drags to chromatic rows again once unfolded", async () => {
    const { clip, a } = await setup();
    await foldToCMinor();
    fireEvent.click(scaleControl("scale notes only"));
    await flush();
    await drag(grid(), [x(1.5), y(60)], [x(1.5), y(61)]);
    expect(notesOf(clip).find((n) => n.id === a)!.pitch).toBe(61);
  });

  it("clicking a folded key selects the notes on that pitch", async () => {
    const { a } = await setup();
    await foldToCMinor();
    fireEvent.pointerDown(document.querySelector('.eth-pr-key[data-pitch="60"]')!, { button: 0 });
    expect([...itemSelection.getState().selected.note]).toEqual([a]);
  });

  it("does not interpret scale selector keys as note-edit shortcuts", async () => {
    const { clip, a } = await setup();
    act(() => itemSelection.getState().select("note", [a], "replace"));
    // Arrows on a closed Select open it (and are consumed): no transpose.
    fireEvent.keyDown(scaleControl("active scale root"), { key: "ArrowUp" });
    fireEvent.keyDown(scaleControl("track scale mode"), { key: "ArrowDown" });
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)!.pitch).toBe(60);
  });

  it("uses folded coordinates for marquee and resize without selecting hidden notes", async () => {
    const { clip, a, b } = await setup();
    await foldToCMinor();
    const top = drawnNote(a)!.y;
    await drag(grid(), [x(0.5), top - 1], [x(3.5), top + KEY_H + 1]);
    expect([...itemSelection.getState().selected.note]).toEqual([a]);
    await drag(grid(), [x(2) - 1, top + KEY_H / 2], [x(3) - 1, top + KEY_H / 2]);
    expect(notesOf(clip).find((n) => n.id === a)!.duration).toBe(2);
    expect(notesOf(clip).find((n) => n.id === b)).toMatchObject({ pitch: 64, start: 2, duration: 1 });
    await undo();
    expect(notesOf(clip).find((n) => n.id === a)!.duration).toBe(1);
  });

  it("select-all takes only the displayed notes while folded", async () => {
    const { a } = await setup();
    await foldToCMinor();
    fireEvent.keyDown(screen.getByTestId("piano-roll"), { key: "a", metaKey: true });
    expect([...itemSelection.getState().selected.note]).toEqual([a]);
  });

  it("allows semitone nudges outside the scale even while rows are filtered", async () => {
    const { clip, a } = await setup();
    await foldToCMinor();
    act(() => itemSelection.getState().select("note", [a], "replace"));
    fireEvent.keyDown(screen.getByTestId("piano-roll"), { key: "ArrowUp" });
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)!.pitch).toBe(61);
    expect(drawnNote(a)).toBeUndefined();
    fireEvent.click(scaleControl("scale notes only"));
    expect(drawnNote(a)).toMatchObject({ pitch: 61, y: (127 - 61) * KEY_H });
  });

  it("shows an empty state without an edited clip, for deleted ids and for audio clips", async () => {
    const { clip } = await setup({ open: false });
    expect(screen.getByTestId("piano-roll-empty")).toBeInTheDocument();
    act(() => useEditorStore.getState().openClip("01DOESNOTEXIST000000000000"));
    expect(screen.getByTestId("piano-roll-empty")).toBeInTheDocument();
    const audio = Object.values(store().project!.clips).find((c) => c.content.type === "Audio")!;
    act(() => useEditorStore.getState().openClip(audio.id));
    expect(screen.getByTestId("piano-roll-empty").textContent).toMatch(/audio/i);
    act(() => useEditorStore.getState().openClip(clip));
    expect(drawnNotes()).toHaveLength(2);
    await send(cmd("Clip", { type: "Delete", ids: [clip] }));
    expect(screen.getByTestId("piano-roll-empty")).toBeInTheDocument();
  });

  it("renders the notes of the edited clip on the keyboard rows", async () => {
    const { a } = await setup();
    expect(drawnNotes()).toHaveLength(2);
    expect(drawnNote(a)).toMatchObject({ x: x(1), y: (127 - 60) * KEY_H, w: PX, h: KEY_H, selected: false, muted: false });
    // The velocity sets the opacity (0.8 → 0.89).
    expect(drawnNote(a)!.opacity).toBeCloseTo(0.45 + 0.55 * 0.8);
    expect(screen.getByTestId("piano-roll-keys").textContent).toContain("C3");
    expect(screen.getByTestId("piano-roll-step").textContent).toBe("1/4");
  });

  it("double-click and drag sets the new note's length", async () => {
    const { clip } = await setup();
    await doublePress(x(2.2), y(70), x(5.4));
    const added = notesOf(clip).filter((n) => n.pitch === 70);
    expect(added).toHaveLength(1);
    expect(added[0]).toMatchObject({ start: 2, duration: 4 });
    expect([...itemSelection.getState().selected.note]).toEqual([added[0]!.id]);
  });

  it("zooms the key height with cmd+shift+wheel, within limits", async () => {
    const { a } = await setup();
    const body = document.querySelector<HTMLElement>(".eth-pr__body")!;
    act(() => {
      fireEvent.wheel(body, { deltaY: -100, metaKey: true, shiftKey: true });
    });
    const k = KEY_H * wheelZoomFactor(-100);
    expect(drawnNote(a)!.h).toBeCloseTo(k);
    expect(drawnNote(a)!.y).toBeCloseTo((127 - 60) * k);
    act(() => {
      for (let i = 0; i < 20; i++) fireEvent.wheel(body, { deltaY: -200, ctrlKey: true, shiftKey: true });
    });
    expect(drawnNote(a)!.h).toBe(MAX_KEY_HEIGHT);
  });

  it("a pressed key shows it is held until release", async () => {
    await setup();
    const key = document.querySelector<HTMLElement>('[data-pitch="60"]')!;
    fireEvent.pointerDown(key, { button: 0 });
    expect(key).toHaveClass("eth-pr-key--pressed");
    fireEvent.pointerUp(window);
    expect(document.querySelector('[data-pitch="60"]')).not.toHaveClass("eth-pr-key--pressed");
  });

  it("the grip left of the keys resizes them: drag down taller, up shorter; double-click resets", async () => {
    const { a } = await setup();
    const grip = screen.getByRole("separator", { name: "key height (drag up or down)" });
    const height = () => drawnNote(a)!.h;
    expect(height()).toBeCloseTo(KEY_H);
    act(() => {
      fireEvent.pointerDown(grip, { button: 0, clientY: 100 });
      fireEvent.pointerMove(window, { clientY: 150 });
    });
    expect(height()).toBeCloseTo(KEY_H * Math.exp(0.5));
    act(() => {
      fireEvent.pointerMove(window, { clientY: 60 });
      fireEvent.pointerUp(window, { clientY: 60 });
    });
    expect(height()).toBeCloseTo(KEY_H * Math.exp(-0.4));
    act(() => {
      fireEvent.doubleClick(grip);
    });
    expect(height()).toBeCloseTo(KEY_H);
  });

  it("moves the selection with snapping (time and pitch) as one undo step", async () => {
    const { clip, a, b } = await setup();
    act(() => itemSelection.getState().select("note", [a, b], "replace"));
    // Drag note A's body by +1.2 beats and up 2 rows: snaps to +1 beat (1/4 grid).
    await drag(grid(), [x(1.5), y(60)], [x(2.7), y(62)]);
    expect(notesOf(clip).map((n) => [n.start, n.pitch])).toEqual([
      [2, 62],
      [3, 66],
    ]);
    await undo();
    expect(notesOf(clip).map((n) => [n.start, n.pitch])).toEqual([
      [1, 60],
      [2, 64],
    ]);
  });

  it("deleting the clip mid-drag ends the drag: no more note edits, one EndGesture (#591)", async () => {
    const { clip } = await setup();
    const sent = vi.spyOn(mock!, "send");
    fireEvent.pointerDown(grid(), { button: 0, clientX: x(1.5), clientY: y(60) });
    fireEvent.pointerMove(window, { clientX: x(2.5), clientY: y(60) });
    await flush();
    expect(sent.mock.calls.some(([c]) => c.domain === "Note")).toBe(true);
    // A collaborator deletes the clip: the grid unmounts mid-drag.
    await send(cmd("Clip", { type: "Delete", ids: [clip] }));
    await flush();
    expect(screen.queryByTestId("piano-roll-grid")).toBeNull();
    const before = sent.mock.calls.length;
    fireEvent.pointerMove(window, { clientX: x(3.5), clientY: y(62) });
    fireEvent.pointerUp(window, { clientX: x(3.5), clientY: y(62) });
    await flush();
    expect(sent.mock.calls.slice(before)).toEqual([]);
    expect(sent.mock.calls.filter(([c]) => c.domain === "Edit" && c.command.type === "EndGesture")).toHaveLength(1);
  });

  it("alt pressed once a move has started bypasses snapping; shift no longer does (#954)", async () => {
    const { clip, a } = await setup();
    // Shift held through the move: still snapped (+0.3 beat rounds to 0).
    await drag(grid(), [x(1.5), y(60)], [x(1.8), y(60)], { shiftKey: true });
    expect(notesOf(clip).find((n) => n.id === a)!.start).toBe(1);
    // A plain press, then alt held on the moves: free (+0.3 beat).
    fireEvent.pointerDown(grid(), { button: 0, clientX: x(1.5), clientY: y(60) });
    fireEvent.pointerMove(window, { clientX: x(1.7), clientY: y(60), altKey: true });
    fireEvent.pointerMove(window, { clientX: x(1.8), clientY: y(60), altKey: true });
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)!.start).toBeCloseTo(1.3);
    // Releasing alt mid-move snaps again at once (no pointer move needed).
    fireEvent.keyUp(window, { key: "Alt", altKey: false });
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)!.start).toBe(1);
    // And pressing it frees the move again.
    fireEvent.keyDown(window, { key: "Alt", altKey: true });
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)!.start).toBeCloseTo(1.3);
    fireEvent.pointerUp(window, { clientX: x(1.8), clientY: y(60), altKey: true });
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)).toMatchObject({ pitch: 60, velocity: 0.8 });
  });

  it("alt bypasses snapping when resizing and drawing notes; shift does not", async () => {
    const { clip, a } = await setup();
    await drag(grid(), [x(2) - 1, y(60)], [x(2.3) - 1, y(60)], { shiftKey: true });
    expect(notesOf(clip).find((n) => n.id === a)!.duration).toBe(1);
    await drag(grid(), [x(2) - 1, y(60)], [x(2.3) - 1, y(60)], { altKey: true });
    expect(notesOf(clip).find((n) => n.id === a)!.duration).toBeCloseTo(1.3);
    // Draw mode: an alt press draws off the grid.
    fireEvent.click(screen.getByRole("button", { name: "draw" }));
    fireEvent.pointerDown(grid(), { button: 0, clientX: x(5.3), clientY: y(67), altKey: true });
    fireEvent.pointerUp(window, { clientX: x(5.3), clientY: y(67), altKey: true });
    await flush();
    const drawn = notesOf(clip).find((n) => n.pitch === 67);
    expect(drawn?.start).toBeCloseTo(5.3);
  });

  it("alt-drag on a selected note changes every selected velocity by the same amount, in one gesture", async () => {
    const { clip, a, b } = await setup();
    act(() => itemSelection.getState().select("note", [a, b], "replace"));
    const sent = vi.spyOn(mock!, "send");
    fireEvent.pointerDown(grid(), { button: 0, clientX: x(1.5), clientY: y(60), altKey: true });
    expect(document.documentElement.style.getPropertyValue("--eth-drag-cursor")).toBe("ns-resize");
    // Up 36 px (a fifth of the 180 px range): +0.2; sideways movement is ignored.
    fireEvent.pointerMove(window, { clientX: x(3), clientY: y(60) - 18, altKey: true });
    fireEvent.pointerMove(window, { clientX: x(3.5), clientY: y(60) - 36, altKey: true });
    await flush();
    expect(screen.getByTestId("piano-roll-velocity-badge").textContent).toBe(`velocity ${Math.round(1 * 127)}`);
    fireEvent.pointerUp(window, { clientX: x(3.5), clientY: y(60) - 36 });
    await flush();
    expect(screen.queryByTestId("piano-roll-velocity-badge")).toBeNull();
    const [na, nb] = notesOf(clip);
    expect(na).toMatchObject({ start: 1, pitch: 60 });
    expect(nb).toMatchObject({ start: 2, pitch: 64 });
    expect(na!.velocity).toBeCloseTo(1);
    expect(nb!.velocity).toBeCloseTo(0.7);
    // Only Note::Edit commands (besides the note preview), all in one gesture, closed once.
    const calls = sent.mock.calls.filter(([c]) => c.domain !== "Recording");
    const kinds = calls.map(([c]) => `${c.domain}.${c.command.type}`);
    expect(new Set(kinds)).toEqual(new Set(["Note.Edit", "Edit.EndGesture"]));
    expect(kinds.filter((k) => k === "Edit.EndGesture")).toHaveLength(1);
    expect(new Set(calls.filter(([c]) => c.domain === "Note").map(([, o]) => o?.gesture)).size).toBe(1);
    await undo();
    expect(notesOf(clip).map((n) => n.velocity)).toEqual([0.8, 0.5]);
  });

  it("alt-drag on an unselected note edits just it and selects it; alt-click changes nothing", async () => {
    const { clip, a, b } = await setup();
    act(() => itemSelection.getState().select("note", [a], "replace"));
    await drag(grid(), [x(2.5), y(64)], [x(2.5), y(64) + 45], { altKey: true });
    expect(notesOf(clip).map((n) => n.velocity)).toEqual([0.8, expect.closeTo(0.25)]);
    expect([...itemSelection.getState().selected.note]).toEqual([b]);
    // Shift: fine control (a tenth).
    await drag(grid(), [x(2.5), y(64)], [x(2.5), y(64) - 90], { altKey: true, shiftKey: true });
    expect(notesOf(clip)[1]!.velocity).toBeCloseTo(0.3);
    const before = notesOf(clip);
    await drag(grid(), [x(1.5), y(60)], [x(1.5) + 1, y(60) + 1], { altKey: true });
    expect(notesOf(clip)).toEqual(before);
    expect([...itemSelection.getState().selected.note]).toEqual([a]);
  });

  it("without alt, dragging a note still moves it (velocity unchanged)", async () => {
    const { clip } = await setup();
    await drag(grid(), [x(1.5), y(60)], [x(2.5), y(60) - 36]);
    expect(notesOf(clip)[0]).toMatchObject({ start: 2, velocity: 0.8 });
    expect(notesOf(clip)[0]!.pitch).toBeGreaterThan(60);
  });

  it("shows the ns-resize cursor on notes while alt is held", async () => {
    await setup();
    expect(grid().classList.contains("eth-pr-grid--alt")).toBe(false);
    fireEvent.keyDown(window, { key: "Alt", altKey: true });
    expect(grid().classList.contains("eth-pr-grid--alt")).toBe(true);
    fireEvent.keyUp(window, { key: "Alt", altKey: false });
    expect(grid().classList.contains("eth-pr-grid--alt")).toBe(false);
  });

  it("the cursor follows what is under the pointer: note body, edges, empty space, alt, draw mode", async () => {
    await setup();
    const hover = (cx: number, cy: number, init: MouseEventInit = {}) => {
      fireEvent.pointerMove(grid(), { clientX: cx, clientY: cy, ...init });
      return grid().style.cursor;
    };
    expect(hover(x(1.5), y(60))).toBe("move");
    expect(hover(x(2) - 1, y(60))).toBe("ew-resize");
    expect(hover(x(1) + 1, y(60))).toBe("ew-resize");
    expect(hover(x(5), y(60))).toBe("default");
    expect(hover(x(1.5), y(61))).toBe("default");
    // Alt over a note body: velocity drag; pressed or released without moving applies too.
    expect(hover(x(1.5), y(60), { altKey: true })).toBe("ns-resize");
    fireEvent.keyUp(window, { key: "Alt", altKey: false });
    expect(grid().style.cursor).toBe("move");
    fireEvent.keyDown(window, { key: "Alt", altKey: true });
    expect(grid().style.cursor).toBe("ns-resize");
    fireEvent.keyUp(window, { key: "Alt", altKey: false });
    fireEvent.click(screen.getByRole("button", { name: "draw" }));
    expect(hover(x(5), y(60))).toBe("crosshair");
    expect(hover(x(1.5), y(60))).toBe("move");
  });

  it("resizes a note from its end and start edges", async () => {
    const { clip } = await setup();
    await drag(grid(), [x(2) - 1, y(60)], [x(3.1), y(60)]);
    expect(notesOf(clip)[0]).toMatchObject({ start: 1, duration: 2 });
    await drag(grid(), [x(1) + 1, y(60)], [x(0.1), y(60)]);
    expect(notesOf(clip)[0]).toMatchObject({ start: 0, duration: 3 });
  });

  it("cmd-drag duplicates the selection to the drop point as one undo step", async () => {
    const { clip, a, b } = await setup();
    act(() => itemSelection.getState().select("note", [a, b], "replace"));
    // Drag note A's body by +2 beats and up 1 row with cmd held.
    await drag(grid(), [x(1.5), y(60)], [x(3.5), y(61)], { metaKey: true });
    const all = notesOf(clip);
    expect(all).toHaveLength(4);
    expect(all.find((n) => n.id === a)).toMatchObject({ start: 1, pitch: 60 });
    expect(all.find((n) => n.id === b)).toMatchObject({ start: 2, pitch: 64 });
    const copies = all.filter((n) => n.id !== a && n.id !== b);
    expect(copies.map((n) => [n.start, n.pitch])).toEqual([
      [3, 61],
      [4, 65],
    ]);
    expect(new Set(itemSelection.getState().selected.note)).toEqual(new Set(copies.map((n) => n.id)));
    await undo();
    expect(notesOf(clip)).toHaveLength(2);
  });

  it("pressing cmd mid-drag switches to duplicating (the original goes back); releasing it undoes that", async () => {
    const { clip, a } = await setup();
    act(() => itemSelection.getState().select("note", [a], "replace"));
    fireEvent.pointerDown(grid(), { button: 0, clientX: x(1.5), clientY: y(60) });
    fireEvent.pointerMove(window, { clientX: x(3.5), clientY: y(60) });
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)).toMatchObject({ start: 3 });
    fireEvent.keyDown(window, { key: "Meta", metaKey: true });
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)).toMatchObject({ start: 1, pitch: 60 });
    expect(
      notesOf(clip)
        .filter((n) => n.pitch === 60)
        .map((n) => n.start),
    ).toEqual([1, 3]);
    expect(document.documentElement.style.getPropertyValue("--eth-drag-cursor")).toBe("copy");

    fireEvent.keyUp(window, { key: "Meta", metaKey: false });
    await flush();
    expect(
      notesOf(clip)
        .filter((n) => n.pitch === 60)
        .map((n) => n.start),
    ).toEqual([3]);

    fireEvent.keyDown(window, { key: "Meta", metaKey: true });
    fireEvent.pointerMove(window, { clientX: x(4.5), clientY: y(60), metaKey: true });
    fireEvent.pointerUp(window, { clientX: x(4.5), clientY: y(60), metaKey: true });
    await flush();
    expect(
      notesOf(clip)
        .filter((n) => n.pitch === 60)
        .map((n) => n.start),
    ).toEqual([1, 4]);
    await undo();
    expect(
      notesOf(clip)
        .filter((n) => n.pitch === 60)
        .map((n) => n.start),
    ).toEqual([1]);
  });

  it("keeps the resize (or move) cursor for the whole drag", async () => {
    await setup();
    const root = document.documentElement;
    fireEvent.pointerDown(grid(), { button: 0, clientX: x(2) - 1, clientY: y(60) });
    expect(root.style.getPropertyValue("--eth-drag-cursor")).toBe("ew-resize");
    expect(root.dataset.dragCursor).toBeDefined();
    fireEvent.pointerUp(window, { clientX: x(2) - 1, clientY: y(60) });
    expect(root.dataset.dragCursor).toBeUndefined();
    fireEvent.pointerDown(grid(), { button: 0, clientX: x(1.5), clientY: y(60) });
    expect(root.style.getPropertyValue("--eth-drag-cursor")).toBe("move");
    fireEvent.pointerUp(window, { clientX: x(1.5), clientY: y(60) });
    await flush();
  });

  it("clicking a note selects it; shift adds; clicking empty space deselects", async () => {
    const { a } = await setup();
    await drag(grid(), [x(1.5), y(60)], [x(1.5), y(60)]);
    expect([...itemSelection.getState().selected.note]).toEqual([a]);
    await drag(grid(), [x(2.5), y(64)], [x(2.5), y(64)], { shiftKey: true });
    expect(itemSelection.getState().selected.note.size).toBe(2);
    await drag(grid(), [x(6), y(40)], [x(6), y(40)]);
    expect(itemSelection.getState().selected.note.size).toBe(0);
  });

  it("a click on empty space moves the playhead there (song time, snapped)", async () => {
    await setup();
    // The clip starts at song beat 64; a click at content beat 4.2 snaps to 4 (1/4 grid).
    await drag(grid(), [x(4.2), y(70)], [x(4.2), y(70)]);
    await waitFor(() => expect(playheadStore.getPlayhead()?.transport.position).toBe(68));
  });

  it("marquee-selects notes", async () => {
    const { a, b } = await setup();
    await drag(grid(), [x(0.5), y(66)], [x(1.5), y(58)]);
    expect([...itemSelection.getState().selected.note]).toEqual([a]);
    await drag(grid(), [x(0.5), y(70)], [x(4), y(50)]);
    expect(new Set(itemSelection.getState().selected.note)).toEqual(new Set([a, b]));
  });

  /** Two presses at the same spot; the second one is held and dragged to `toX`. */
  async function doublePress(atX: number, atY: number, toX = atX) {
    fireEvent.pointerDown(grid(), { button: 0, clientX: atX, clientY: atY });
    fireEvent.pointerUp(window, { clientX: atX, clientY: atY });
    await flush();
    await drag(grid(), [atX, atY], [toX, atY]);
  }

  it("double-click on empty space adds a snapped note; double-click on a note deletes it", async () => {
    const { clip, a } = await setup();
    await doublePress(x(4.6), y(67));
    const added = notesOf(clip).find((n) => n.pitch === 67)!;
    expect(added).toMatchObject({ start: 4.5 - 0.5, duration: 1 });
    expect([...itemSelection.getState().selected.note]).toEqual([added.id]);
    await doubleClickAt(x(1.5), y(60));
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)).toBeUndefined();
  });

  it("a new note takes the velocity of the last touched note (the default before any)", async () => {
    const { clip, a, b } = await setup();
    // Nothing touched yet: the default (MIDI 100).
    await doublePress(x(4.6), y(67));
    expect(notesOf(clip).find((n) => n.pitch === 67)!.velocity).toBeCloseTo(100 / 127);
    // Select both, last clicking b (0.5): the new note takes b's velocity, in one undo step.
    await drag(grid(), [x(1.5), y(60)], [x(1.5), y(60)]);
    await drag(grid(), [x(2.5), y(64)], [x(2.5), y(64)], { shiftKey: true });
    await doublePress(x(6.6), y(69));
    expect(notesOf(clip).find((n) => n.pitch === 69)!.velocity).toBeCloseTo(0.5);
    await undo();
    expect(notesOf(clip).find((n) => n.pitch === 69)).toBeUndefined();
    // Editing the last touched note's velocity counts: alt-drag a up 36 px (0.8 → 1).
    await drag(grid(), [x(1.5), y(60)], [x(1.5), y(60) - 36], { altKey: true });
    expect(notesOf(clip).find((n) => n.id === a)!.velocity).toBeCloseTo(1);
    await doublePress(x(6.6), y(71));
    expect(notesOf(clip).find((n) => n.pitch === 71)!.velocity).toBeCloseTo(1);
    // Draw mode too, from the note just drawn (itself the last touched one).
    fireEvent.click(screen.getByRole("button", { name: "draw" }));
    await drag(grid(), [x(5.2), y(48)], [x(5.2), y(48)]);
    expect(notesOf(clip).find((n) => n.pitch === 48)!.velocity).toBeCloseTo(1);
    expect(notesOf(clip).find((n) => n.id === b)!.velocity).toBe(0.5);
  });

  it("draw mode: a drag draws a note and extends it, as one undo step", async () => {
    const { clip } = await setup();
    fireEvent.click(screen.getByRole("button", { name: "draw" }));
    await drag(grid(), [x(5.2), y(48)], [x(7.1), y(48)]);
    const drawn = notesOf(clip).find((n) => n.pitch === 48)!;
    expect(drawn).toMatchObject({ start: 5, duration: 3 });
    await undo();
    expect(notesOf(clip).find((n) => n.pitch === 48)).toBeUndefined();
  });

  it("keyboard: delete, select all, transpose, duplicate", async () => {
    const { clip, a } = await setup();
    const root = screen.getByTestId("piano-roll");
    act(() => itemSelection.getState().select("note", [a], "replace"));
    fireEvent.keyDown(root, { key: "ArrowUp", shiftKey: true });
    await flush();
    expect(notesOf(clip)[0]!.pitch).toBe(72);
    fireEvent.keyDown(root, { key: "ArrowRight" });
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)!.start).toBe(2);
    fireEvent.keyDown(root, { key: "a", metaKey: true });
    expect(itemSelection.getState().selected.note.size).toBe(2);
    fireEvent.keyDown(root, { key: "d", ctrlKey: true });
    await flush();
    expect(notesOf(clip)).toHaveLength(4);
    expect(itemSelection.getState().selected.note.size).toBe(2);
    fireEvent.keyDown(root, { key: "Delete" });
    await flush();
    expect(notesOf(clip)).toHaveLength(2);
  });

  it("velocity lane: dragging a bar changes the selected notes' velocity in one gesture", async () => {
    const { clip, a, b } = await setup();
    act(() => itemSelection.getState().select("note", [a, b], "replace"));
    const lane = screen.getByTestId("piano-roll-velocity");
    // One bar per note at its start (lane px), as tall as its velocity; selected: outlined.
    const bars = () => velocityScene(lane).filter((i) => i.kind === "velocity");
    expect(bars().map((i) => [i.x, i.h, i.kind === "velocity" && i.selected])).toEqual([
      [x(1) - 1, expect.closeTo(0.8 * 68, 3), true],
      [x(2) - 1, expect.closeTo(0.5 * 68, 3), true],
    ]);
    expect(velocityScene(lane).filter((i) => i.kind === "velocity-outline")).toHaveLength(2);
    // Hovering a bar: its velocity as the tooltip, the resize cursor.
    fireEvent.pointerMove(lane, { clientX: x(1) + 1, clientY: 60 });
    expect(lane.title).toBe("velocity 102");
    expect(lane.style.cursor).toBe("ns-resize");
    fireEvent.pointerMove(lane, { clientX: x(1) + 1, clientY: 5 });
    expect(lane.title).toBe("");
    // Lane is 72 px tall: dragging up 14.4 px adds 0.2.
    await drag(lane, [x(1) + 1, 50], [x(1) + 1, 50 - 14.4]);
    const [na, nb] = notesOf(clip);
    expect(na!.velocity).toBeCloseTo(1);
    expect(nb!.velocity).toBeCloseTo(0.7);
    await undo();
    expect(notesOf(clip).map((n) => n.velocity)).toEqual([0.8, 0.5]);
  });

  it("quantize snaps the selection (or the whole clip) to the grid", async () => {
    const { clip, a, b } = await setup();
    await send(cmd("Note", { type: "Edit", edits: [{ id: a, pitch: null, velocity: null, start: 1.2, duration: null, muted: null }] }));
    await send(cmd("Note", { type: "Edit", edits: [{ id: b, pitch: null, velocity: null, start: 2.4, duration: null, muted: null }] }));
    act(() => itemSelection.getState().select("note", [a], "replace"));
    fireEvent.click(screen.getByRole("button", { name: "quantize" }));
    await flush();
    expect(notesOf(clip).map((n) => n.start)).toEqual([1, 2.4]);
    act(() => itemSelection.getState().clear("note"));
    fireEvent.keyDown(screen.getByTestId("piano-roll"), { key: "u", metaKey: true });
    await flush();
    expect(notesOf(clip).map((n) => n.start)).toEqual([1, 2]);
  });

  it("reverse mirrors the selected notes inside their span (button, R key, menu), one undo step", async () => {
    const { clip, a, b } = await setup();
    const c = "01PIANOROLLNOTEC0000000000";
    await send(cmd("Note", { type: "Add", clip, notes: [{ id: c, pitch: 67, velocity: 0.8, start: 5, duration: 1 }] }));
    const start = (id: string) => store().project!.notes[id]!.start;
    // Nothing selected, no section: the button does nothing.
    act(() => resetPianoRollSection());
    expect(screen.getByTestId("piano-roll-reverse")).toBeDisabled();
    act(() => itemSelection.getState().select("note", [a, b], "replace"));
    fireEvent.click(screen.getByTestId("piano-roll-reverse"));
    await flush();
    // Span 1..3: a (1..2) -> 2..3, b (2..3) -> 1..2; c is not selected.
    expect([start(a), start(b), start(c)]).toEqual([2, 1, 5]);
    await undo();
    expect([start(a), start(b), start(c)]).toEqual([1, 2, 5]);
    fireEvent.keyDown(screen.getByTestId("piano-roll"), { key: "r" });
    await flush();
    expect([start(a), start(b), start(c)]).toEqual([2, 1, 5]);
    fireEvent.keyDown(screen.getByTestId("piano-roll"), { key: "r" });
    await flush();
    expect([start(a), start(b), start(c)]).toEqual([1, 2, 5]);
  });

  it("shows the clip loop region and dims content outside the clip", async () => {
    const { clip } = await setup();
    expect(screen.queryByTestId("piano-roll-loop")).toBeNull();
    expect(screen.getByTestId("piano-roll-clip-end").style.left).toBe(`${x(8)}px`);
    await send(cmd("Clip", { type: "SetLoop", id: clip, looping: { enabled: true, start: 1, end: 3 } }));
    const loop = screen.getByTestId("piano-roll-loop");
    expect(loop.style.left).toBe(`${x(1)}px`);
    expect(loop.style.width).toBe(`${x(2)}px`);
    expect(screen.getByTestId("piano-roll-clip-end").style.left).toBe(`${x(3)}px`);
  });
});

describe("PianoRoll sections and note clipboard (section-edit)", () => {
  const root = () => screen.getByTestId("piano-roll");
  const press = async (k: string) => {
    fireEvent.keyDown(root(), { key: k, metaKey: !["Delete", "Backspace", "Escape"].includes(k) });
    await flush();
  };
  const starts = (clip: string) => notesOf(clip).map((n) => n.start);
  const clipLength = (clip: string) => store().project!.clips[clip]!.length;
  const strip = () => screen.getByTestId("piano-roll-loopbar");
  /** Replace the clip's notes with one C3 at each content beat of `at`. */
  async function notesAt(clip: string, at: number[]) {
    await send(cmd("Note", { type: "Remove", ids: notesOf(clip).map((n) => n.id) }));
    await send(
      cmd("Note", {
        type: "Add",
        clip,
        notes: at.map((start, i) => ({ id: `01SECTIONNOTE${String(i).padStart(13, "0")}`, pitch: 60, velocity: 0.8, start, duration: 0.5 })),
      }),
    );
  }

  beforeEach(() => {
    resetPianoRollSection();
    resetTimeSelection();
  });

  it("the owner's case: a 4-beat section with notes at 0 and 3, ⌘D three times, tiles with the gaps", async () => {
    const { clip } = await setup();
    await notesAt(clip, [0, 3]);
    // A drag on the strip under the ruler selects the section 0..4 and its notes.
    await drag(strip(), [x(0), 2], [x(4), 2]);
    expect(usePianoRollSection.getState().section).toEqual({ clip, start: 0, end: 4 });
    expect(itemSelection.getState().selected.note.size).toBe(2);
    expect(screen.getByTestId("piano-roll-section").style.width).toBe(`${x(4)}px`);
    for (let i = 0; i < 3; i++) await press("d");
    expect(starts(clip)).toEqual([0, 3, 4, 7, 8, 11, 12, 15]);
    // The clip grew to hold the last copy; the section sits on it.
    expect(clipLength(clip)).toBe(16);
    expect(usePianoRollSection.getState().section).toMatchObject({ start: 12, end: 16 });
    // Each duplicate is one undo step (the notes and the clip growth together).
    await undo();
    expect(starts(clip)).toEqual([0, 3, 4, 7, 8, 11]);
    expect(clipLength(clip)).toBe(12);
  });

  it("a Shift-drag makes a section; ⌘C then ⌘V pastes its full length right after it, repeatedly", async () => {
    const { clip } = await setup();
    // Notes at 1 (C3) and 2 (E3); the Shift-drag covers beats 0..4 over both.
    await drag(grid(), [x(0.1), y(70)], [x(3.9), y(50)], { shiftKey: true });
    expect(usePianoRollSection.getState().section).toEqual({ clip, start: 0, end: 4, pitches: { low: 50, high: 70 } });
    // Drawn over those rows only (like the arrangement's section over its tracks).
    const box = screen.getByTestId("piano-roll-section");
    expect(box.style.top).toBe(`${(127 - 70) * KEY_H}px`);
    expect(box.style.height).toBe(`${21 * KEY_H}px`);
    await press("c");
    expect(usePianoRollSection.getState().clipboard).toMatchObject({ length: 4, notes: [{ start: 1 }, { start: 2 }] });
    await press("v");
    await press("v");
    expect(starts(clip)).toEqual([1, 2, 5, 6, 9, 10]);
    expect(clipLength(clip)).toBe(12);
    // The pasted notes are selected (so the next edit acts on them).
    expect(itemSelection.getState().selected.note.size).toBe(2);
  });

  it("a Shift-click extends the section outside it (rows too) and retracts the nearer edge inside it", async () => {
    const { clip } = await setup();
    await drag(grid(), [x(0.1), y(70)], [x(3.9), y(50)], { shiftKey: true });
    const shiftClick = (beats: number, pitch: number) => drag(grid(), [x(beats), y(pitch)], [x(beats), y(pitch)], { shiftKey: true });
    await shiftClick(6.1, 40);
    expect(usePianoRollSection.getState().section).toEqual({ clip, start: 0, end: 6, pitches: { low: 40, high: 70 } });
    await shiftClick(0.9, 60);
    expect(usePianoRollSection.getState().section).toEqual({ clip, start: 1, end: 6, pitches: { low: 40, high: 70 } });
    await shiftClick(4.9, 60);
    expect(usePianoRollSection.getState().section).toEqual({ clip, start: 1, end: 5, pitches: { low: 40, high: 70 } });
    // The notes under it are selected, like after a Shift-drag (C3 at 1, E3 at 2).
    expect(itemSelection.getState().selected.note.size).toBe(2);
  });

  it("a plain drag selects notes and makes no section; ⌘D duplicates them right after their span", async () => {
    const { clip, b } = await setup();
    // Only E3 (pitch 64, at beat 2, one beat long), over beats 0..4.
    await drag(grid(), [x(0.1), y(66)], [x(3.9), y(63)]);
    expect([...itemSelection.getState().selected.note]).toEqual([b]);
    expect(usePianoRollSection.getState().section).toBeNull();
    await press("d");
    expect(notesOf(clip).map((n) => [n.start, n.pitch])).toEqual([
      [1, 60],
      [2, 64],
      [3, 64],
    ]);
  });

  it("the note menu has cut / copy / paste / duplicate with keymap hints; the empty grid has paste (#575)", async () => {
    const { clip, a } = await setup();
    const items = () => useContextMenuStore.getState().menu!.items.filter((i): i is ContextMenuItem => i !== "separator");
    const item = (label: string) => items().find((i) => i.label === label)!;
    // The empty grid: paste, disabled (with why) until notes were copied.
    fireEvent.contextMenu(grid(), { clientX: x(6.5), clientY: y(70) });
    expect(item("paste")).toMatchObject({ disabled: true, reason: expect.any(String) });
    // A note: the clip menu's clipboard set, hints from the keymap (not hard-coded).
    fireEvent.contextMenu(grid(), { clientX: x(1.5), clientY: y(60) });
    expect(
      items()
        .map((i) => i.label)
        .slice(0, 4),
    ).toEqual(["cut", "copy", "paste", "duplicate"]);
    expect(item("copy").shortcut).toBe(shortcutLabel("edit.copy"));
    expect(item("delete note").shortcut).toBe(shortcutLabel("edit.delete"));
    expect(itemSelection.getState().selected.note).toEqual(new Set([a]));
    act(() => item("copy").onSelect());
    expect(usePianoRollSection.getState().clipboard).toMatchObject({ notes: [{ pitch: 60 }] });
    act(() => item("duplicate").onSelect());
    await flush();
    expect(notesOf(clip).filter((n) => n.pitch === 60)).toHaveLength(2);
    // Paste from the empty grid's menu, where ⌘V pastes: right after the duplicate's section.
    fireEvent.contextMenu(grid(), { clientX: x(6.5), clientY: y(70) });
    expect(item("paste").disabled).toBe(false);
    act(() => item("paste").onSelect());
    await flush();
    expect(
      notesOf(clip)
        .filter((n) => n.pitch === 60)
        .map((n) => n.start),
    ).toEqual([1, 2, 3]);
  });

  it("without a section: ⌘C copies the selected notes, ⌘V pastes at the selection cursor, then after the paste", async () => {
    const { clip } = await setup();
    await drag(grid(), [x(1.5), y(60)], [x(1.5), y(60)]);
    expect(usePianoRollSection.getState().section).toBeNull();
    await press("c");
    // The note's span (1..2) on the 1/4 grid.
    expect(usePianoRollSection.getState().clipboard).toMatchObject({ length: 1, notes: [{ start: 0, pitch: 60 }] });
    // The clip starts at song beat 64: the selection cursor at 68 is content beat 4. The
    // playhead elsewhere doesn't matter.
    act(() => useTimeSelection.setState({ cursor: 68 }));
    await send(cmd("Transport", { type: "Locate", position: 80 }));
    await waitFor(() => expect(playheadStore.getPlayhead()?.transport.position).toBe(80));
    await press("v");
    await press("v");
    expect(
      notesOf(clip)
        .filter((n) => n.pitch === 60)
        .map((n) => n.start),
    ).toEqual([1, 4, 5]);
  });

  it("insert marker: a click on empty grid while playing sets it (the playhead stays); ⌘V pastes there", async () => {
    const { clip } = await setup();
    await drag(grid(), [x(1.5), y(60)], [x(1.5), y(60)]);
    await press("c");
    await send(cmd("Transport", { type: "Play" }));
    await waitFor(() => expect(store().transport?.playing).toBe(true));
    const locates: number[] = [];
    const real = mock!.send.bind(mock);
    vi.spyOn(mock!, "send").mockImplementation((c, o) => {
      if (c.domain === "Transport" && c.command.type === "Locate") locates.push(c.command.position);
      return real(c, o);
    });
    const playhead = playheadStore.getPlayhead()?.transport.position;
    // Content beat 6.1 snaps to 6 (1/4 grid): the marker, drawn as a line.
    await drag(grid(), [x(6.1), y(40)], [x(6.1), y(40)]);
    expect(usePianoRollSection.getState().section).toEqual({ clip, start: 6, end: 6 });
    expect(screen.getByTestId("piano-roll-marker").dataset.beats).toBe("6");
    await press("v");
    expect(
      notesOf(clip)
        .filter((n) => n.pitch === 60)
        .map((n) => n.start),
    ).toEqual([1, 6]);
    // The pasted range becomes the section, so ⌘V again lands right after it.
    expect(usePianoRollSection.getState().section).toEqual({ clip, start: 6, end: 7 });
    expect(locates).toEqual([]);
    expect(playheadStore.getPlayhead()?.transport.position).toBe(playhead);
  });

  it("insert marker while stopped: the playhead goes there, so Play starts from it", async () => {
    const { clip } = await setup();
    await drag(grid(), [x(4.2), y(70)], [x(4.2), y(70)]);
    expect(usePianoRollSection.getState().section).toEqual({ clip, start: 4, end: 4 });
    await waitFor(() => expect(playheadStore.getPlayhead()?.transport.position).toBe(68));
  });

  it("⌘X cuts the section's notes; ⌫ with a section and no selected notes clears it; Esc leaves it", async () => {
    const { clip } = await setup();
    await notesAt(clip, [0, 1, 2, 5]);
    await drag(strip(), [x(0), 2], [x(2), 2]);
    await press("x");
    expect(starts(clip)).toEqual([2, 5]);
    await press("v");
    expect(starts(clip)).toEqual([2, 2, 3, 5]);
    // A section with nothing selected: ⌫ removes the notes starting in it.
    await drag(strip(), [x(4), 2], [x(6), 2]);
    act(() => itemSelection.getState().clear("note"));
    await press("Delete");
    expect(starts(clip)).toEqual([2, 2, 3]);
    await press("Escape");
    expect(usePianoRollSection.getState().section).toBeNull();
    expect(screen.queryByTestId("piano-roll-section")).toBeNull();
  });

  it("⌘A selects every note like a plain marquee over them all: no section, the old one goes", async () => {
    const { clip, a, b } = await setup();
    // A plain (no Shift) marquee over both notes.
    await drag(grid(), [x(0.1), y(70)], [x(3.9), y(50)]);
    const marquee = [...itemSelection.getState().selected.note].sort();
    expect(marquee).toEqual([a, b].sort());
    expect(usePianoRollSection.getState().section).toBeNull();
    // A Shift-drag section over one note, then ⌘A: the same state as the marquee.
    await drag(grid(), [x(0.1), y(61)], [x(1.9), y(59)], { shiftKey: true });
    expect(usePianoRollSection.getState().section).toMatchObject({ clip });
    expect(itemSelection.getState().selected.note.size).toBe(1);
    await press("a");
    expect([...itemSelection.getState().selected.note].sort()).toEqual(marquee);
    expect(usePianoRollSection.getState().section).toBeNull();
    expect(screen.queryByTestId("piano-roll-section")).toBeNull();
    expect(useTimeSelection.getState().selection).toBeNull();
  });

  it("a click on empty grid places the insert marker; a press on a note clears the section", async () => {
    const { clip } = await setup();
    await drag(grid(), [x(0.1), y(70)], [x(3.9), y(50)], { shiftKey: true });
    expect(usePianoRollSection.getState().section).not.toBeNull();
    await drag(grid(), [x(1.5), y(60)], [x(1.5), y(60)]);
    expect(usePianoRollSection.getState().section).toBeNull();
    await drag(grid(), [x(6), y(40)], [x(6), y(40)]);
    expect(usePianoRollSection.getState().section).toEqual({ clip, start: 6, end: 6 });
    expect(screen.queryByTestId("piano-roll-section")).toBeNull();
    expect(screen.getByTestId("piano-roll-marker")).toBeTruthy();
  });

  it("takes the desktop Edit menu's copy and paste events while focused", async () => {
    const { clip } = await setup();
    await drag(grid(), [x(0.1), y(70)], [x(3.9), y(50)], { shiftKey: true });
    act(() => root().focus());
    await act(async () => void document.dispatchEvent(new Event("copy", { cancelable: true })));
    await act(async () => void document.dispatchEvent(new Event("paste", { cancelable: true })));
    await flush();
    expect(starts(clip)).toEqual([1, 2, 5, 6]);
  });
});

describe("sections are free, like the arrangement's; edits act on what is inside them", () => {
  const strip = () => screen.getByTestId("piano-roll-loopbar");
  const section = () => usePianoRollSection.getState().section;
  const press = async (k: string) => {
    fireEvent.keyDown(screen.getByTestId("piano-roll"), { key: k, metaKey: true });
    await flush();
  };
  const A = "01SECTIONLONG0000000000000";
  const B = "01SECTIONSHORT000000000000";
  /** A C3 at 1 lasting to 4 (A), an E3 at 5 lasting to 6 (B). */
  async function longNotes(clip: string) {
    await send(cmd("Note", { type: "Remove", ids: notesOf(clip).map((n) => n.id) }));
    await send(
      cmd("Note", {
        type: "Add",
        clip,
        notes: [
          { id: A, pitch: 60, velocity: 0.8, start: 1, duration: 3 },
          { id: B, pitch: 64, velocity: 0.8, start: 5, duration: 1 },
        ],
      }),
    );
  }

  beforeEach(() => {
    resetPianoRollSection();
    resetTimeSelection();
  });

  it("strip drag: the section is the drag, not widened to the notes it selects (any note overlapping it)", async () => {
    const { clip } = await setup();
    await longNotes(clip);
    await drag(strip(), [x(0), 2], [x(2), 2]);
    expect([...itemSelection.getState().selected.note]).toEqual([A]);
    expect(section()).toEqual({ clip, start: 0, end: 2 });
    expect(screen.getByTestId("piano-roll-strip-section").style.width).toBe(`${x(2)}px`);
  });

  it("like the arrangement: a plain drag selects notes (no section), a Shift-drag makes a section and keeps the selected notes, live; Shift switches mid-drag", async () => {
    const { clip } = await setup();
    await longNotes(clip);
    // Plain, over A only: A selected, no section.
    await drag(grid(), [x(2), y(61)], [x(3), y(59)]);
    expect([...itemSelection.getState().selected.note]).toEqual([A]);
    expect(section()).toBeNull();
    // Shift, over A: A stays selected (notes and a section at once); the section, shown while
    // dragging, is the drag (2..3), not widened to A (1..4).
    // (Pressed from the other end: a second press on the same spot would be a double-click.)
    fireEvent.pointerDown(grid(), { button: 0, clientX: x(3), clientY: y(59), shiftKey: true });
    fireEvent.pointerMove(window, { clientX: x(2), clientY: y(61), shiftKey: true });
    await flush();
    expect(section()).toMatchObject({ clip, start: 2, end: 3 });
    expect(screen.getByTestId("piano-roll-section")).toBeTruthy();
    expect([...itemSelection.getState().selected.note]).toEqual([A]);
    // Releasing Shift mid-drag: back to selecting notes, the section preview goes.
    fireEvent.keyUp(window, { key: "Shift", shiftKey: false });
    await flush();
    expect(section()).toBeNull();
    expect([...itemSelection.getState().selected.note]).toEqual([A]);
    expect(screen.getByTestId("piano-roll-marquee")).toBeTruthy();
    fireEvent.pointerUp(window, { clientX: x(2), clientY: y(61) });
    await flush();
    expect(section()).toBeNull();
  });

  it("a Shift-drag section is free (not widened to notes under it) and changing the selection leaves it", async () => {
    const { clip } = await setup();
    await longNotes(clip);
    // Over B (5..6) from 5 to 7: B selected, the section 5..7 on those rows.
    await drag(grid(), [x(5), y(65)], [x(7), y(63)], { shiftKey: true });
    expect([...itemSelection.getState().selected.note]).toEqual([B]);
    expect(section()).toEqual({ clip, start: 5, end: 7, pitches: { low: 63, high: 65 } });
    // Inside A's time span but on other rows: nothing selected, the section 2..3.
    await drag(grid(), [x(2), y(90)], [x(3), y(85)], { shiftKey: true });
    expect(itemSelection.getState().selected.note.size).toBe(0);
    expect(section()).toMatchObject({ clip, start: 2, end: 3 });
    act(() => itemSelection.getState().select("note", [B], "replace"));
    await flush();
    expect(section()).toMatchObject({ clip, start: 2, end: 3 });
  });

  it("copy trims the notes to the section; cut removes only what is inside (a spanning note is split)", async () => {
    const { clip } = await setup();
    await longNotes(clip);
    // Beats 2..3 over A's row: A (1..4) spans it.
    await drag(grid(), [x(2), y(61)], [x(3), y(59)], { shiftKey: true });
    await press("c");
    expect(usePianoRollSection.getState().clipboard).toMatchObject({ length: 1, notes: [{ pitch: 60, start: 0, duration: 1 }] });
    await press("x");
    const pieces = notesOf(clip)
      .filter((n) => n.pitch === 60)
      .map((n) => [n.start, n.duration])
      .sort((a, b) => a[0]! - b[0]!);
    expect(pieces).toEqual([
      [1, 1],
      [3, 1],
    ]);
    // B, on another row, is untouched.
    expect(notesOf(clip).find((n) => n.id === B)).toMatchObject({ start: 5, duration: 1 });
  });

  it("⌘D on selected notes (no section) pastes right at the end of their span (no grid rounding)", async () => {
    const { clip } = await setup();
    await longNotes(clip);
    const C = "01SECTIONODD0000000000000A";
    await send(cmd("Note", { type: "Add", clip, notes: [{ id: C, pitch: 70, velocity: 0.8, start: 2.5, duration: 0.25 }] }));
    act(() => itemSelection.getState().select("note", [C], "replace"));
    await press("d");
    // Span 2.5..2.75: the copy starts at 2.75.
    expect(
      notesOf(clip)
        .filter((n) => n.pitch === 70)
        .map((n) => n.start)
        .sort(),
    ).toEqual([2.5, 2.75]);
  });
});

describe("PianoRoll: a clip extended to the left past its content start", () => {
  // The clip (song 64, 8 beats) extended 2 beats to the left: start 62, offset -2. The editor
  // shows content -2 .. 8 (bar 1 stays at content 0).
  async function extended() {
    // The ruler measures a width: the editor fits the clip when it opens.
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockReturnValue(800);
    const ids = await setup({ open: false });
    await send(cmd("Clip", { type: "SetBounds", id: ids.clip, start: 62, length: 10, offset: -2 }));
    await send(cmd("Note", { type: "Add", clip: ids.clip, notes: [{ id: "01PIANOROLLNOTEGAP00000000", pitch: 62, velocity: 0.8, start: -1, duration: 0.5 }] }));
    act(() => useEditorStore.getState().openClip(ids.clip));
    pickOption(screen.getByRole("combobox", { name: "grid" }), { value: "5" });
    await flush();
    // The fitted view: content -2 .. 8 over 800 px (8 px padding), scrolled to the clip start.
    const ppb = (800 - 16) / 10;
    const scroll = -2;
    const gx = (beats: number) => (beats - scroll) * ppb;
    return { ...ids, gap: "01PIANOROLLNOTEGAP00000000", gx, ppb };
  }

  it("shows the leading gap and the notes in it, where they play", async () => {
    const { gap, a, gx, ppb } = await extended();
    const n = drawnNote(gap)!;
    expect(n).toBeDefined();
    expect(n.x).toBeCloseTo(gx(-1));
    expect(n.x).toBeGreaterThan(0);
    expect(n.w).toBeCloseTo(0.5 * ppb);
    expect(drawnNote(a)!.x).toBeCloseTo(gx(1));
    // Nothing is shaded at the gap: the clip starts at content -2.
    const shaded = [...document.querySelectorAll<HTMLElement>(".eth-pr-grid__outside")];
    for (const s of shaded) expect(parseFloat(s.style.left) + (parseFloat(s.style.width) || Infinity)).not.toBeCloseTo(gx(0));
    expect(document.querySelector<HTMLElement>(".eth-pr__start")!.style.left).toBe(`${gx(-2)}px`);
  });

  it("draws and moves notes in the gap (never before the clip start)", async () => {
    const { clip, gap, gx } = await extended();
    // Draw at content -1.6 (snapped down to the beat grid: -2, the clip start).
    await doublePressAt(gx(-1.6), y(70));
    const drawn = notesOf(clip).find((n) => n.pitch === 70)!;
    expect(drawn).toMatchObject({ start: -2, duration: 1 });
    // Drag the gap note 3 beats left: it stops at the clip start (content -2).
    await drag(grid(), [gx(-0.75), y(62)], [gx(-3.75), y(62)]);
    expect(notesOf(clip).find((n) => n.id === gap)!.start).toBeCloseTo(-2);
    // A note from the clip's content moves into the gap.
    const b = notesOf(clip).find((n) => n.pitch === 64)!;
    await drag(grid(), [gx(b.start + 0.5), y(64)], [gx(b.start + 0.5 - 3), y(64)]);
    expect(notesOf(clip).find((n) => n.id === b.id)!.start).toBeCloseTo(-1);
  });

  async function doublePressAt(atX: number, atY: number) {
    fireEvent.pointerDown(grid(), { button: 0, clientX: atX, clientY: atY });
    fireEvent.pointerUp(window, { clientX: atX, clientY: atY });
    await flush();
    await drag(grid(), [atX, atY], [atX, atY]);
  }
});
