import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Command, Note } from "@/generated";
import { playheadStore, useEditorStore, useProjectStore } from "@/state";
import { itemSelection, wheelZoomFactor } from "@/timeline";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { DEFAULT_KEY_HEIGHT as KEY_H, MAX_KEY_HEIGHT } from "./geometry";
import { PianoRoll } from "./index";
import { pickOption } from "@/kit/testing";
import { resetPianoRollSection, usePianoRollSection } from "./section";

// The piano roll's own view starts at 40 px/beat, scrolled to 0. jsdom has no layout: the
// grid's box is at (0, 0), so client coordinates are grid-local px. Notes report their
// inline-style box so edge/body hit zones work.
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
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    if (this.dataset.noteId && this.dataset.testid === "piano-roll-note") {
      const s = this.style;
      return new DOMRect(parseFloat(s.left), parseFloat(s.top), parseFloat(s.width), parseFloat(s.height));
    }
    return new DOMRect(0, 0, 0, 0);
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  mock?.dispose();
  mock = undefined;
  store().reset();
  useEditorStore.getState().close();
  itemSelection.getState().clear();
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
  if (opts.open !== false) pickOption(screen.getByRole("combobox", { name: "Grid" }), { value: "5" });
  return { clip, a: "01PIANOROLLNOTEA0000000000", b: "01PIANOROLLNOTEB0000000000" };
}

const noteEl = (id: string) => document.querySelector(`[data-testid="piano-roll-note"][data-note-id="${id}"]`)!;
const grid = () => screen.getByTestId("piano-roll-grid");

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
  /** C minor with "Scale notes only" on: rows are the scale's pitches, 127 down to 0. */
  async function foldToCMinor() {
    await send(cmd("Project", { type: "SetScale", scale: { root: 0, kind: "Minor" } }));
    fireEvent.click(scaleControl("Scale notes only"));
    await flush();
  }
  const keys = () => [...document.querySelectorAll<HTMLElement>(".eth-pr-key")];
  /** Center of `pitch`'s row with folded rows (from the rendered keys). */
  const foldedY = (pitch: number) => keys().findIndex((key) => key.dataset.pitch === String(pitch)) * KEY_H + KEY_H / 2;

  it("edits project and custom track scales independently and can undo them", async () => {
    const { clip } = await setup();
    const track = store().project!.clips[clip]!.track;
    expect(screen.getByTestId("scale-trigger")).toHaveTextContent("Scale…");
    expect(scaleControl("Active scale type")).toHaveTextContent("Chromatic / None");
    await pickScale("Active scale type", "Minor");
    expect(store().project!.settings.scale).toEqual({ root: 0, kind: "Minor" });
    expect(screen.getByTestId("scale-trigger")).toHaveTextContent("C Minor…");
    expect(screen.getByTestId("scale-popover")).toHaveTextContent("Project");
    await pickScale("Track scale mode", "Custom");
    await pickScale("Active scale root", "9");
    expect(store().project!.tracks[track]!.scale).toEqual({ type: "Custom", scale: { root: 9, kind: "Minor" } });
    await send(cmd("Project", { type: "SetScale", scale: { root: 2, kind: "Major" } }));
    expect(scaleControl("Active scale root")).toHaveTextContent("A");
    await pickScale("Track scale mode", "FollowProject");
    expect(scaleControl("Active scale root")).toHaveTextContent("D");
    await undo();
    expect(scaleControl("Active scale root")).toHaveTextContent("A");
    await pickScale("Track scale mode", "Chromatic");
    expect(scaleControl("Active scale type")).toHaveTextContent("Chromatic / None");
    expect(scaleControl("Active scale type")).toBeDisabled();
  });

  it("keeps highlight and folding as local view state (no document edit)", async () => {
    await setup();
    await send(cmd("Project", { type: "SetScale", scale: { root: 0, kind: "Minor" } }));
    const before = store().project!;
    fireEvent.click(scaleControl("Highlight"));
    fireEvent.click(scaleControl("Scale notes only"));
    await flush();
    expect(store().project).toBe(before);
    expect(screen.getByTestId("scale-trigger")).toHaveAttribute("aria-pressed", "true");
  });

  it("highlights roots, dims outside notes and still accepts any pitch", async () => {
    const { clip, a, b } = await setup();
    await send(cmd("Project", { type: "SetScale", scale: { root: 0, kind: "Minor" } }));
    expect(noteEl(a)).toHaveAttribute("data-scale-tone", "root");
    expect(noteEl(b)).toHaveAttribute("data-scale-tone", "out");
    expect(document.querySelector('.eth-pr-key[data-pitch="63"]')).toHaveAttribute("data-scale-tone", "in");
    await doublePress(x(4), y(61));
    expect(notesOf(clip).some((n) => n.pitch === 61)).toBe(true);
    fireEvent.click(scaleControl("Highlight"));
    expect(noteEl(a)).not.toHaveAttribute("data-scale-tone");
    expect(noteEl(b)).not.toHaveAttribute("data-scale-tone");
  });

  it("folds keys and notes together, draws and drags at the displayed pitches, and restores hidden notes", async () => {
    const { clip, a, b } = await setup();
    await foldToCMinor();
    expect(keys().length).toBeLessThan(128);
    expect(document.querySelector('.eth-pr-key[data-pitch="64"]')).toBeNull();
    expect(noteEl(b)).toBeNull();
    expect(notesOf(clip)).toHaveLength(2);
    expect((noteEl(a) as HTMLElement).style.top).toBe(`${foldedY(60) - KEY_H / 2}px`);
    await doublePress(x(4), foldedY(63));
    expect(notesOf(clip).some((n) => n.pitch === 63 && n.start === 4)).toBe(true);
    await drag(noteEl(a), [x(1.5), foldedY(60)], [x(1.5), foldedY(62)]);
    expect(notesOf(clip).find((n) => n.id === a)!.pitch).toBe(62);
    await undo();
    expect(notesOf(clip).find((n) => n.id === a)!.pitch).toBe(60);
    fireEvent.click(scaleControl("Scale notes only"));
    expect(keys()).toHaveLength(128);
    expect(noteEl(b)).toBeInTheDocument();
    expect(notesOf(clip).find((n) => n.id === b)!.pitch).toBe(64);
  });

  it("cmd-drag copies to the displayed row when folded; one row up is the next scale pitch", async () => {
    const { clip, a } = await setup();
    await foldToCMinor();
    act(() => itemSelection.getState().select("note", [a], "replace"));
    // One folded row up from C (60) is D (62), two semitones.
    await drag(noteEl(a), [x(1.5), foldedY(60)], [x(3.5), foldedY(60) - KEY_H], { metaKey: true });
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
    fireEvent.click(scaleControl("Scale notes only"));
    await flush();
    await drag(noteEl(a), [x(1.5), y(60)], [x(1.5), y(61)]);
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
    fireEvent.keyDown(scaleControl("Active scale root"), { key: "ArrowUp" });
    fireEvent.keyDown(scaleControl("Track scale mode"), { key: "ArrowDown" });
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)!.pitch).toBe(60);
  });

  it("uses folded coordinates for marquee and resize without selecting hidden notes", async () => {
    const { clip, a, b } = await setup();
    await foldToCMinor();
    const top = parseFloat((noteEl(a) as HTMLElement).style.top);
    await drag(grid(), [x(0.5), top - 1], [x(3.5), top + KEY_H + 1]);
    expect([...itemSelection.getState().selected.note]).toEqual([a]);
    await drag(noteEl(a), [x(2) - 1, top + KEY_H / 2], [x(3) - 1, top + KEY_H / 2]);
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
    expect(noteEl(a)).toBeNull();
    fireEvent.click(scaleControl("Scale notes only"));
    expect(noteEl(a)).toHaveAttribute("data-pitch", "61");
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
    expect(screen.getAllByTestId("piano-roll-note")).toHaveLength(2);
    await send(cmd("Clip", { type: "Delete", ids: [clip] }));
    expect(screen.getByTestId("piano-roll-empty")).toBeInTheDocument();
  });

  it("renders the notes of the edited clip on the keyboard rows", async () => {
    const { a } = await setup();
    expect(screen.getAllByTestId("piano-roll-note")).toHaveLength(2);
    const el = noteEl(a) as HTMLElement;
    expect(el.style.left).toBe(`${x(1)}px`);
    expect(el.style.top).toBe(`${(127 - 60) * KEY_H}px`);
    expect(el.style.width).toBe(`${PX}px`);
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
    expect(parseFloat((noteEl(a) as HTMLElement).style.height)).toBeCloseTo(k);
    expect(parseFloat((noteEl(a) as HTMLElement).style.top)).toBeCloseTo((127 - 60) * k);
    act(() => {
      for (let i = 0; i < 20; i++) fireEvent.wheel(body, { deltaY: -200, ctrlKey: true, shiftKey: true });
    });
    expect((noteEl(a) as HTMLElement).style.height).toBe(`${MAX_KEY_HEIGHT}px`);
  });

  it("moves the selection with snapping (time and pitch) as one undo step", async () => {
    const { clip, a, b } = await setup();
    act(() => itemSelection.getState().select("note", [a, b], "replace"));
    // Drag note A's body by +1.2 beats and up 2 rows: snaps to +1 beat (1/4 grid).
    await drag(noteEl(a), [x(1.5), y(60)], [x(2.7), y(62)]);
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

  it("shift bypasses snapping when moving notes; alt still does when resizing", async () => {
    const { clip, a } = await setup();
    await drag(noteEl(a), [x(1.5), y(60)], [x(1.8), y(60)], { shiftKey: true });
    expect(notesOf(clip)[0]!.start).toBeCloseTo(1.3);
    await drag(noteEl(a), [x(2.3) - 1, y(60)], [x(2.6) - 1, y(60)], { altKey: true });
    expect(notesOf(clip)[0]!.duration).toBeCloseTo(1.3);
  });

  it("alt-drag on a selected note changes every selected velocity by the same amount, in one gesture", async () => {
    const { clip, a, b } = await setup();
    act(() => itemSelection.getState().select("note", [a, b], "replace"));
    const sent = vi.spyOn(mock!, "send");
    fireEvent.pointerDown(noteEl(a), { button: 0, clientX: x(1.5), clientY: y(60), altKey: true });
    expect(document.documentElement.style.getPropertyValue("--eth-drag-cursor")).toBe("ns-resize");
    // Up 36 px (a fifth of the 180 px range): +0.2; sideways movement is ignored.
    fireEvent.pointerMove(window, { clientX: x(3), clientY: y(60) - 18, altKey: true });
    fireEvent.pointerMove(window, { clientX: x(3.5), clientY: y(60) - 36, altKey: true });
    await flush();
    expect(screen.getByTestId("piano-roll-velocity-badge").textContent).toBe(`Velocity ${Math.round(1 * 127)}`);
    fireEvent.pointerUp(window, { clientX: x(3.5), clientY: y(60) - 36 });
    await flush();
    expect(screen.queryByTestId("piano-roll-velocity-badge")).toBeNull();
    const [na, nb] = notesOf(clip);
    expect(na).toMatchObject({ start: 1, pitch: 60 });
    expect(nb).toMatchObject({ start: 2, pitch: 64 });
    expect(na!.velocity).toBeCloseTo(1);
    expect(nb!.velocity).toBeCloseTo(0.7);
    // Only Note::Edit commands, all in one gesture, closed once.
    const calls = sent.mock.calls;
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
    await drag(noteEl(b), [x(2.5), y(64)], [x(2.5), y(64) + 45], { altKey: true });
    expect(notesOf(clip).map((n) => n.velocity)).toEqual([0.8, expect.closeTo(0.25)]);
    expect([...itemSelection.getState().selected.note]).toEqual([b]);
    // Shift: fine control (a tenth).
    await drag(noteEl(b), [x(2.5), y(64)], [x(2.5), y(64) - 90], { altKey: true, shiftKey: true });
    expect(notesOf(clip)[1]!.velocity).toBeCloseTo(0.3);
    const before = notesOf(clip);
    await drag(noteEl(a), [x(1.5), y(60)], [x(1.5) + 1, y(60) + 1], { altKey: true });
    expect(notesOf(clip)).toEqual(before);
    expect([...itemSelection.getState().selected.note]).toEqual([a]);
  });

  it("without alt, dragging a note still moves it (velocity unchanged)", async () => {
    const { clip, a } = await setup();
    await drag(noteEl(a), [x(1.5), y(60)], [x(2.5), y(60) - 36]);
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

  it("resizes a note from its end and start edges", async () => {
    const { clip, a } = await setup();
    await drag(noteEl(a), [x(2) - 1, y(60)], [x(3.1), y(60)]);
    expect(notesOf(clip)[0]).toMatchObject({ start: 1, duration: 2 });
    await drag(noteEl(a), [x(1) + 1, y(60)], [x(0.1), y(60)]);
    expect(notesOf(clip)[0]).toMatchObject({ start: 0, duration: 3 });
  });

  it("cmd-drag duplicates the selection to the drop point as one undo step", async () => {
    const { clip, a, b } = await setup();
    act(() => itemSelection.getState().select("note", [a, b], "replace"));
    // Drag note A's body by +2 beats and up 1 row with cmd held.
    await drag(noteEl(a), [x(1.5), y(60)], [x(3.5), y(61)], { metaKey: true });
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
    fireEvent.pointerDown(noteEl(a), { button: 0, clientX: x(1.5), clientY: y(60) });
    fireEvent.pointerMove(window, { clientX: x(3.5), clientY: y(60) });
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)).toMatchObject({ start: 3 });
    fireEvent.keyDown(window, { key: "Meta", metaKey: true });
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)).toMatchObject({ start: 1, pitch: 60 });
    expect(notesOf(clip).filter((n) => n.pitch === 60).map((n) => n.start)).toEqual([1, 3]);
    expect(document.documentElement.style.getPropertyValue("--eth-drag-cursor")).toBe("copy");

    fireEvent.keyUp(window, { key: "Meta", metaKey: false });
    await flush();
    expect(notesOf(clip).filter((n) => n.pitch === 60).map((n) => n.start)).toEqual([3]);

    fireEvent.keyDown(window, { key: "Meta", metaKey: true });
    fireEvent.pointerMove(window, { clientX: x(4.5), clientY: y(60), metaKey: true });
    fireEvent.pointerUp(window, { clientX: x(4.5), clientY: y(60), metaKey: true });
    await flush();
    expect(notesOf(clip).filter((n) => n.pitch === 60).map((n) => n.start)).toEqual([1, 4]);
    await undo();
    expect(notesOf(clip).filter((n) => n.pitch === 60).map((n) => n.start)).toEqual([1]);
  });

  it("keeps the resize (or move) cursor for the whole drag", async () => {
    const { a } = await setup();
    const root = document.documentElement;
    fireEvent.pointerDown(noteEl(a), { button: 0, clientX: x(2) - 1, clientY: y(60) });
    expect(root.style.getPropertyValue("--eth-drag-cursor")).toBe("ew-resize");
    expect(root.dataset.dragCursor).toBeDefined();
    fireEvent.pointerUp(window, { clientX: x(2) - 1, clientY: y(60) });
    expect(root.dataset.dragCursor).toBeUndefined();
    fireEvent.pointerDown(noteEl(a), { button: 0, clientX: x(1.5), clientY: y(60) });
    expect(root.style.getPropertyValue("--eth-drag-cursor")).toBe("move");
    fireEvent.pointerUp(window, { clientX: x(1.5), clientY: y(60) });
    await flush();
  });

  it("clicking a note selects it; shift adds; clicking empty space deselects", async () => {
    const { a, b } = await setup();
    await drag(noteEl(a), [x(1.5), y(60)], [x(1.5), y(60)]);
    expect([...itemSelection.getState().selected.note]).toEqual([a]);
    await drag(noteEl(b), [x(2.5), y(64)], [x(2.5), y(64)], { shiftKey: true });
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
    fireEvent.doubleClick(noteEl(a));
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)).toBeUndefined();
  });

  it("draw mode: a drag draws a note and extends it, as one undo step", async () => {
    const { clip } = await setup();
    fireEvent.click(screen.getByRole("button", { name: "Draw" }));
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
    const bar = document.querySelector(`[data-testid="piano-roll-velocity-bar"][data-note-id="${a}"]`)!;
    // Lane is 72 px tall: dragging up 14.4 px adds 0.2.
    await drag(bar, [0, 50], [0, 50 - 14.4]);
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
    fireEvent.click(screen.getByRole("button", { name: "Quantize" }));
    await flush();
    expect(notesOf(clip).map((n) => n.start)).toEqual([1, 2.4]);
    act(() => itemSelection.getState().clear("note"));
    fireEvent.keyDown(screen.getByTestId("piano-roll"), { key: "u", metaKey: true });
    await flush();
    expect(notesOf(clip).map((n) => n.start)).toEqual([1, 2]);
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

  beforeEach(() => resetPianoRollSection());

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

  it("a marquee makes a section; ⌘C then ⌘V pastes its full length right after it, repeatedly", async () => {
    const { clip } = await setup();
    // Notes at 1 (C3) and 2 (E3); the marquee covers beats 0..4 over both.
    await drag(grid(), [x(0.1), y(70)], [x(3.9), y(50)]);
    expect(usePianoRollSection.getState().section).toEqual({ clip, start: 0, end: 4 });
    await press("c");
    expect(usePianoRollSection.getState().clipboard).toMatchObject({ length: 4, notes: [{ start: 1 }, { start: 2 }] });
    await press("v");
    await press("v");
    expect(starts(clip)).toEqual([1, 2, 5, 6, 9, 10]);
    expect(clipLength(clip)).toBe(12);
    // The pasted notes are selected (so the next edit acts on them).
    expect(itemSelection.getState().selected.note.size).toBe(2);
  });

  it("a marquee over some pitches copies only those notes, still with the section's timing", async () => {
    const { clip, b } = await setup();
    // Only E3 (pitch 64, at beat 2), over beats 0..4.
    await drag(grid(), [x(0.1), y(66)], [x(3.9), y(63)]);
    expect([...itemSelection.getState().selected.note]).toEqual([b]);
    await press("d");
    expect(notesOf(clip).map((n) => [n.start, n.pitch])).toEqual([
      [1, 60],
      [2, 64],
      [6, 64],
    ]);
  });

  it("without a section: ⌘C copies the selected notes, ⌘V pastes at the playhead, then after the paste", async () => {
    const { clip, a } = await setup();
    await drag(noteEl(a), [x(1.5), y(60)], [x(1.5), y(60)]);
    expect(usePianoRollSection.getState().section).toBeNull();
    await press("c");
    // The note's span (1..2) on the 1/4 grid.
    expect(usePianoRollSection.getState().clipboard).toMatchObject({ length: 1, notes: [{ start: 0, pitch: 60 }] });
    // The clip starts at song beat 64: the playhead at 68 is content beat 4.
    await send(cmd("Transport", { type: "Locate", position: 68 }));
    await waitFor(() => expect(playheadStore.getPlayhead()?.transport.position).toBe(68));
    await press("v");
    await press("v");
    expect(notesOf(clip).filter((n) => n.pitch === 60).map((n) => n.start)).toEqual([1, 4, 5]);
  });

  it("insert marker: a click on empty grid while playing sets it (the playhead stays); ⌘V pastes there", async () => {
    const { clip, a } = await setup();
    await drag(noteEl(a), [x(1.5), y(60)], [x(1.5), y(60)]);
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
    expect(notesOf(clip).filter((n) => n.pitch === 60).map((n) => n.start)).toEqual([1, 6]);
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

  it("a click on empty grid places the insert marker; a press on a note clears the section; ⌘A selects the clip's region", async () => {
    const { clip, a } = await setup();
    await drag(grid(), [x(0.1), y(70)], [x(3.9), y(50)]);
    expect(usePianoRollSection.getState().section).not.toBeNull();
    await drag(noteEl(a), [x(1.5), y(60)], [x(1.5), y(60)]);
    expect(usePianoRollSection.getState().section).toBeNull();
    await press("a");
    expect(usePianoRollSection.getState().section).toEqual({ clip, start: 0, end: 8 });
    await drag(grid(), [x(6), y(40)], [x(6), y(40)]);
    expect(usePianoRollSection.getState().section).toEqual({ clip, start: 6, end: 6 });
    expect(screen.queryByTestId("piano-roll-section")).toBeNull();
    expect(screen.getByTestId("piano-roll-marker")).toBeTruthy();
  });

  it("takes the desktop Edit menu's copy and paste events while focused", async () => {
    const { clip } = await setup();
    await drag(grid(), [x(0.1), y(70)], [x(3.9), y(50)]);
    act(() => root().focus());
    await act(async () => void document.dispatchEvent(new Event("copy", { cancelable: true })));
    await act(async () => void document.dispatchEvent(new Event("paste", { cancelable: true })));
    await flush();
    expect(starts(clip)).toEqual([1, 2, 5, 6]);
  });
});
