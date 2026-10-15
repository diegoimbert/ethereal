// Multi-clip editing (base-143): two MIDI clips on two tracks, selected together and edited
// in one piano roll on the song timeline.
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Command, Note } from "@/generated";
import { useEditorStore, useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { pickOption } from "@/kit/testing";
import { DEFAULT_KEY_HEIGHT as KEY_H } from "./geometry";
import { PianoRoll } from "./index";
import { resetPianoRollSection } from "./section";
import { gridScene, type SceneItem } from "./noteLayout";

// The view starts at 40 px/beat scrolled to song beat 0 (jsdom has no layout: no fit).
const PX = 40;
const x = (beats: number) => beats * PX;
const y = (pitch: number) => (127 - pitch) * KEY_H + KEY_H / 2;

const A = "01MULTICLIPA00000000000000";
const B = "01MULTICLIPB00000000000000";
const TRACK2 = "01MULTICLIPTRACK2000000000";
const A1 = "01MULTICLIPNOTEA100000000";
const A2 = "01MULTICLIPNOTEA200000000";
const B1 = "01MULTICLIPNOTEB100000000";

let mock: MockTransport | undefined;
const store = () => useProjectStore.getState();
const notesOf = (clip: string): Note[] =>
  Object.values(store().project!.notes)
    .filter((n) => n.clip === clip)
    .sort((a, b) => a.start - b.start || a.pitch - b.pitch);

beforeEach(() => {
  // No canvas in jsdom: what the grid draws is checked through `gridScene`.
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
});

afterEach(() => {
  vi.restoreAllMocks();
  mock?.dispose();
  mock = undefined;
  store().reset();
  useEditorStore.getState().close();
  itemSelection.getState().clear();
  resetPianoRollSection();
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

const undo = () => send(cmd("Edit", { type: "Undo" }));

/**
 * Clip A on the first MIDI track at beat 4 (offset 1, notes at content 1 and 3), clip B on a
 * second MIDI track at beat 16 (a note at content 2); both selected, A opened.
 */
async function setup() {
  mock = new MockTransport({ timers: "manual", seed: 1 });
  render(
    <TransportProvider transport={mock}>
      <PianoRoll />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  const track = Object.values(store().project!.tracks).find((t) => t.kind === "Midi")!;
  await send(cmd("Track", { type: "Create", id: TRACK2, kind: "Midi", name: "Keys 2", color: null, parent: null, before: null }));
  await send(cmd("Clip", { type: "CreateMidi", id: A, track: track.id, start: 4, length: 8, name: "Verse" }));
  await send(cmd("Clip", { type: "SetBounds", id: A, start: 4, length: 8, offset: 1 }));
  await send(cmd("Clip", { type: "CreateMidi", id: B, track: TRACK2, start: 16, length: 8, name: "Chorus" }));
  await send(
    cmd("Note", {
      type: "Add",
      clip: A,
      notes: [
        { id: A1, pitch: 60, velocity: 0.8, start: 1, duration: 1 },
        { id: A2, pitch: 64, velocity: 0.8, start: 3, duration: 1 },
      ],
    }),
  );
  await send(cmd("Note", { type: "Add", clip: B, notes: [{ id: B1, pitch: 67, velocity: 0.8, start: 2, duration: 1 }] }));
  act(() => {
    itemSelection.getState().select("clip", [A, B], "replace");
    useEditorStore.getState().openClip(A);
  });
  pickOption(screen.getByRole("combobox", { name: "grid" }), { value: "5" });
  await flush();
}

const grid = () => screen.getByTestId("piano-roll-grid");
type DrawnNote = Extract<SceneItem, { kind: "note" }>;
/** Note `id` as drawn on the grid's canvas (undefined: not drawn). */
const drawnNote = (id: string) => gridScene(grid()).notes.find((i): i is DrawnNote => i.kind === "note" && i.id === id);
const press = async (key: string, init: KeyboardEventInit = {}) => {
  fireEvent.keyDown(screen.getByTestId("piano-roll"), { key, ...init });
  await flush();
};

async function drag(el: Element, from: [number, number], to: [number, number], init: MouseEventInit = {}) {
  fireEvent.pointerDown(el, { button: 0, clientX: from[0], clientY: from[1], ...init });
  for (let i = 1; i <= 3; i++) {
    fireEvent.pointerMove(window, { clientX: from[0] + ((to[0] - from[0]) * i) / 3, clientY: from[1] + ((to[1] - from[1]) * i) / 3, ...init });
    await flush();
  }
  fireEvent.pointerUp(window, { clientX: to[0], clientY: to[1] });
  await flush();
}

async function doublePress(atX: number, atY: number) {
  fireEvent.pointerDown(grid(), { button: 0, clientX: atX, clientY: atY });
  fireEvent.pointerUp(window, { clientX: atX, clientY: atY });
  await flush();
  await drag(grid(), [atX, atY], [atX, atY]);
}

describe("multi-clip editing", () => {
  it("shows both clips' notes on the song timeline with a band per clip", async () => {
    await setup();
    expect(screen.getByTestId("piano-roll-clip-count")).toHaveTextContent("2 clips");
    const bands = screen.getAllByTestId("piano-roll-clip-band");
    expect(bands.map((b) => b.textContent)).toEqual(["Verse", "Chorus"]);
    expect(bands[0]).toHaveAttribute("aria-pressed", "true");
    // A plays content 1 at song 4: A1 at song 4, A2 at song 6; B1 at song 18.
    expect(drawnNote(A1)!.x).toBe(x(4));
    expect(drawnNote(A2)!.x).toBe(x(6));
    expect(drawnNote(B1)!.x).toBe(x(18));
    expect(drawnNote(B1)!.inactive).toBe(true);
    expect(drawnNote(A1)!.inactive).toBe(false);
    // The other clip's notes are dimmed.
    expect(drawnNote(B1)!.opacity).toBeCloseTo(drawnNote(A1)!.opacity * 0.6);
  });

  it("select all + transpose edits notes of both clips in one undo step", async () => {
    await setup();
    await press("a", { metaKey: true });
    expect(new Set(itemSelection.getState().selected.note)).toEqual(new Set([A1, A2, B1]));
    await press("ArrowUp");
    expect(notesOf(A).map((n) => n.pitch)).toEqual([61, 65]);
    expect(notesOf(B).map((n) => n.pitch)).toEqual([68]);
    await undo();
    expect(notesOf(A).map((n) => n.pitch)).toEqual([60, 64]);
    expect(notesOf(B).map((n) => n.pitch)).toEqual([67]);
  });

  it("dragging notes of both clips moves each in its own clip content; one undo restores both", async () => {
    await setup();
    act(() => itemSelection.getState().select("note", [A1, B1], "replace"));
    await drag(grid(), [x(18.5), y(67)], [x(19.5), y(67)]);
    expect(notesOf(A).find((n) => n.id === A1)!.start).toBe(2);
    expect(notesOf(B)[0]!.start).toBe(3);
    // Pressing B's note focused B.
    expect(useEditorStore.getState().clip).toBe(B);
    await undo();
    expect(notesOf(A).find((n) => n.id === A1)!.start).toBe(1);
    expect(notesOf(B)[0]!.start).toBe(2);
  });

  it("a move never takes a note out of its clip (clamped at the clip end)", async () => {
    await setup();
    act(() => itemSelection.getState().select("note", [A2], "replace"));
    // A ends at song 12: A2 (song 6) dragged by +8 stops before the end.
    await drag(grid(), [x(6.5), y(64)], [x(14.5), y(64)]);
    const a2 = notesOf(A).find((n) => n.id === A2)!;
    expect(a2.start + 3).toBeLessThan(12);
    expect(a2.start).toBeGreaterThan(3);
    expect(notesOf(B)).toHaveLength(1);
  });

  it("drawing puts the note in the clip under the pointer and focuses it; nothing outside clips", async () => {
    await setup();
    await doublePress(x(20), y(72));
    const drawn = notesOf(B).find((n) => n.pitch === 72)!;
    expect(drawn.start).toBe(4);
    expect(useEditorStore.getState().clip).toBe(B);
    await doublePress(x(5), y(55));
    expect(notesOf(A).find((n) => n.pitch === 55)!.start).toBe(2);
    const count = Object.keys(store().project!.notes).length;
    await doublePress(x(14), y(50));
    expect(Object.keys(store().project!.notes)).toHaveLength(count);
  });

  it("a click on a clip's band focuses it; deselecting the other clip returns to single-clip editing", async () => {
    await setup();
    fireEvent.click(screen.getAllByTestId("piano-roll-clip-band")[1]!);
    await flush();
    expect(useEditorStore.getState().clip).toBe(B);
    expect(screen.getAllByTestId("piano-roll-clip-band")[1]).toHaveAttribute("aria-pressed", "true");
    act(() => itemSelection.getState().select("clip", [B], "replace"));
    await flush();
    expect(screen.queryByTestId("piano-roll-clip-bands")).toBeNull();
    // Single-clip editing: B's content axis (B1 at content 2).
    expect(drawnNote(B1)!.x).toBe(x(2));
    expect(drawnNote(A1)).toBeUndefined();
  });

  it("quantize with nothing selected quantizes every clip (one undo step)", async () => {
    await setup();
    await send(cmd("Note", { type: "Edit", edits: [{ id: A1, pitch: null, velocity: null, start: 1.2, duration: null, muted: null }] }));
    await send(cmd("Note", { type: "Edit", edits: [{ id: B1, pitch: null, velocity: null, start: 2.2, duration: null, muted: null }] }));
    fireEvent.click(screen.getByRole("button", { name: "quantize" }));
    await flush();
    expect(notesOf(A).find((n) => n.id === A1)!.start).toBe(1);
    expect(notesOf(B)[0]!.start).toBe(2);
    await undo();
    expect(notesOf(A).find((n) => n.id === A1)!.start).toBeCloseTo(1.2);
    expect(notesOf(B)[0]!.start).toBeCloseTo(2.2);
  });

  it("copy and paste put the notes back into the clips under them", async () => {
    await setup();
    act(() => itemSelection.getState().select("note", [B1], "replace"));
    await press("c", { metaKey: true });
    // The insert marker at song 21 (B, content 5).
    await drag(grid(), [x(21), y(40)], [x(21), y(40)]);
    await press("v", { metaKey: true });
    expect(notesOf(B).map((n) => [n.pitch, n.start])).toEqual([
      [67, 2],
      [67, 5],
    ]);
  });
});
