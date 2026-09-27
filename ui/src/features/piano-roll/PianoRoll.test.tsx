import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Command, Note } from "@/generated";
import { useEditorStore, useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { DEFAULT_KEY_HEIGHT as KEY_H } from "./geometry";
import { PianoRoll } from "./index";

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
  if (opts.open !== false) fireEvent.change(screen.getByLabelText("Grid"), { target: { value: "5" } });
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
  it("edits project and custom track scales independently and can undo them", async () => {
    const { clip } = await setup();
    const track = store().project!.clips[clip]!.track;
    expect(screen.getByLabelText("Active scale type")).toHaveValue("Chromatic");
    fireEvent.change(screen.getByLabelText("Active scale type"), { target: { value: "Minor" } });
    await flush();
    expect(store().project!.settings.scale).toEqual({ root: 0, kind: "Minor" });
    expect(screen.getByText("· Project")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Track scale mode"), { target: { value: "Custom" } });
    await flush();
    fireEvent.change(screen.getByLabelText("Active scale root"), { target: { value: "9" } });
    await flush();
    expect(store().project!.tracks[track]!.scale).toEqual({ type: "Custom", scale: { root: 9, kind: "Minor" } });
    await send(cmd("Project", { type: "SetScale", scale: { root: 2, kind: "Major" } }));
    expect(screen.getByLabelText("Active scale root")).toHaveValue("9");
    fireEvent.change(screen.getByLabelText("Track scale mode"), { target: { value: "FollowProject" } });
    await flush();
    expect(screen.getByLabelText("Active scale root")).toHaveValue("2");
    await undo();
    expect(screen.getByLabelText("Active scale root")).toHaveValue("9");
    fireEvent.change(screen.getByLabelText("Track scale mode"), { target: { value: "Chromatic" } });
    await flush();
    expect(screen.getByLabelText("Active scale type")).toHaveValue("Chromatic");
    expect(screen.getByLabelText("Active scale type")).toBeDisabled();
  });

  it("highlights roots, dims outside notes and still accepts any pitch", async () => {
    const { clip, a, b } = await setup();
    await send(cmd("Project", { type: "SetScale", scale: { root: 0, kind: "Minor" } }));
    expect(noteEl(a)).toHaveAttribute("data-scale-tone", "root");
    expect(noteEl(b)).toHaveAttribute("data-scale-tone", "out");
    expect(document.querySelector('.eth-pr-key[data-pitch="63"]')).toHaveAttribute("data-scale-tone", "in");
    fireEvent.doubleClick(grid(), { clientX: x(4), clientY: y(61) });
    await flush();
    expect(notesOf(clip).some((n) => n.pitch === 61)).toBe(true);
    fireEvent.click(screen.getByLabelText("Highlight"));
    expect(noteEl(a)).not.toHaveAttribute("data-scale-tone");
    expect(noteEl(b)).not.toHaveAttribute("data-scale-tone");
  });

  it("folds keys and notes together, draws and drags at the displayed pitches, and restores hidden notes", async () => {
    const { clip, a, b } = await setup();
    await send(cmd("Project", { type: "SetScale", scale: { root: 0, kind: "Minor" } }));
    fireEvent.click(screen.getByLabelText("Scale notes only"));
    const keys = () => [...document.querySelectorAll<HTMLElement>(".eth-pr-key")];
    const foldedY = (pitch: number) => keys().findIndex((key) => key.dataset.pitch === String(pitch)) * KEY_H + KEY_H / 2;
    expect(keys().length).toBeLessThan(128);
    expect(document.querySelector('.eth-pr-key[data-pitch="64"]')).toBeNull();
    expect(noteEl(b)).toBeNull();
    expect(notesOf(clip)).toHaveLength(2);
    expect((noteEl(a) as HTMLElement).style.top).toBe(`${foldedY(60) - KEY_H / 2}px`);
    fireEvent.doubleClick(grid(), { clientX: x(4), clientY: foldedY(63) });
    await flush();
    expect(notesOf(clip).some((n) => n.pitch === 63 && n.start === 4)).toBe(true);
    await drag(noteEl(a), [x(1.5), foldedY(60)], [x(1.5), foldedY(62)]);
    expect(notesOf(clip).find((n) => n.id === a)!.pitch).toBe(62);
    await undo();
    expect(notesOf(clip).find((n) => n.id === a)!.pitch).toBe(60);
    fireEvent.click(screen.getByLabelText("Scale notes only"));
    expect(keys()).toHaveLength(128);
    expect(noteEl(b)).toBeInTheDocument();
    expect(notesOf(clip).find((n) => n.id === b)!.pitch).toBe(64);
  });

  it("does not interpret scale selector keys as note-edit shortcuts", async () => {
    const { clip, a } = await setup();
    act(() => itemSelection.getState().select("note", [a], "replace"));
    fireEvent.keyDown(screen.getByLabelText("Active scale type"), { key: "Delete" });
    fireEvent.keyDown(screen.getByLabelText("Active scale root"), { key: "ArrowUp" });
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)!.pitch).toBe(60);
  });

  it("uses folded coordinates for marquee and resize without selecting hidden notes", async () => {
    const { clip, a, b } = await setup();
    await send(cmd("Project", { type: "SetScale", scale: { root: 0, kind: "Minor" } }));
    fireEvent.click(screen.getByLabelText("Scale notes only"));
    const top = parseFloat((noteEl(a) as HTMLElement).style.top);
    await drag(grid(), [x(0.5), top - 1], [x(3.5), top + KEY_H + 1]);
    expect([...itemSelection.getState().selected.note]).toEqual([a]);
    await drag(noteEl(a), [x(2) - 1, top + KEY_H / 2], [x(3) - 1, top + KEY_H / 2]);
    expect(notesOf(clip).find((n) => n.id === a)!.duration).toBe(2);
    expect(notesOf(clip).find((n) => n.id === b)).toMatchObject({ pitch: 64, start: 2, duration: 1 });
    await undo();
    expect(notesOf(clip).find((n) => n.id === a)!.duration).toBe(1);
  });

  it("allows semitone nudges outside the scale even while rows are filtered", async () => {
    const { clip, a } = await setup();
    await send(cmd("Project", { type: "SetScale", scale: { root: 0, kind: "Minor" } }));
    fireEvent.click(screen.getByLabelText("Scale notes only"));
    act(() => itemSelection.getState().select("note", [a], "replace"));
    fireEvent.keyDown(screen.getByTestId("piano-roll"), { key: "ArrowUp" });
    await flush();
    expect(notesOf(clip).find((n) => n.id === a)!.pitch).toBe(61);
    expect(noteEl(a)).toBeNull();
    fireEvent.click(screen.getByLabelText("Scale notes only"));
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

  it("alt bypasses snapping", async () => {
    const { clip, a } = await setup();
    await drag(noteEl(a), [x(1.5), y(60)], [x(1.8), y(60)], { altKey: true });
    expect(notesOf(clip)[0]!.start).toBeCloseTo(1.3);
  });

  it("resizes a note from its end and start edges", async () => {
    const { clip, a } = await setup();
    await drag(noteEl(a), [x(2) - 1, y(60)], [x(3.1), y(60)]);
    expect(notesOf(clip)[0]).toMatchObject({ start: 1, duration: 2 });
    await drag(noteEl(a), [x(1) + 1, y(60)], [x(0.1), y(60)]);
    expect(notesOf(clip)[0]).toMatchObject({ start: 0, duration: 3 });
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

  it("marquee-selects notes", async () => {
    const { a, b } = await setup();
    await drag(grid(), [x(0.5), y(66)], [x(1.5), y(58)]);
    expect([...itemSelection.getState().selected.note]).toEqual([a]);
    await drag(grid(), [x(0.5), y(70)], [x(4), y(50)]);
    expect(new Set(itemSelection.getState().selected.note)).toEqual(new Set([a, b]));
  });

  it("double-click on empty space adds a snapped note; double-click on a note deletes it", async () => {
    const { clip, a } = await setup();
    fireEvent.doubleClick(grid(), { clientX: x(4.6), clientY: y(67) });
    await flush();
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
