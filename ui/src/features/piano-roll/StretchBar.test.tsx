import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Command, Note } from "@/generated";
import { useEditorStore, useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { pickOption } from "@/kit/testing";
import { PianoRoll } from "./index";
import { resetPianoRollSection, usePianoRollSection } from "./section";

// Same harness as PianoRoll.test.tsx: 40 px/beat, scrolled to 0; jsdom boxes are at (0, 0)
// except the stretch bar and notes, which report their inline-style box.
const PX = 40;
const x = (beats: number) => beats * PX;

let mock: MockTransport | undefined;
const store = () => useProjectStore.getState();
const notesOf = (clip: string): Note[] =>
  Object.values(store().project!.notes)
    .filter((n) => n.clip === clip)
    .sort((a, b) => a.start - b.start || a.pitch - b.pitch);
const placed = (clip: string) => notesOf(clip).map((n) => [n.start, n.duration]);

beforeEach(() => {
  resetPianoRollSection();
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    const id = this.dataset.testid;
    if (id === "piano-roll-stretch" || (id === "piano-roll-note" && this.dataset.noteId)) {
      const s = this.style;
      return new DOMRect(parseFloat(s.left), parseFloat(s.top) || 0, parseFloat(s.width), parseFloat(s.height) || 6);
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

const A = "01STRETCHNOTEA000000000000";
const B = "01STRETCHNOTEB000000000000";

/** A 4-beat MIDI clip with notes at 1 and 2 (1 beat long), open in the piano roll, 1/4 grid. */
async function setup() {
  mock = new MockTransport({ timers: "manual", seed: 1 });
  render(
    <TransportProvider transport={mock}>
      <PianoRoll />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  const track = Object.values(store().project!.tracks).find((t) => t.kind === "Midi")!;
  const clip = "01STRETCHTESTCLIP000000000";
  await send(cmd("Clip", { type: "CreateMidi", id: clip, track: track.id, start: 64, length: 4, name: "Test" }));
  await send(
    cmd("Note", {
      type: "Add",
      clip,
      notes: [
        { id: A, pitch: 60, velocity: 0.8, start: 1, duration: 1 },
        { id: B, pitch: 64, velocity: 0.5, start: 2, duration: 1 },
      ],
    }),
  );
  act(() => useEditorStore.getState().openClip(clip));
  pickOption(screen.getByRole("combobox", { name: "Grid" }), { value: "5" });
  return clip;
}

async function drag(el: Element, from: number, to: number, init: MouseEventInit = {}) {
  fireEvent.pointerDown(el, { button: 0, clientX: from, clientY: 2, ...init });
  for (let i = 1; i <= 3; i++) {
    fireEvent.pointerMove(window, { clientX: from + ((to - from) * i) / 3, clientY: 2, ...init });
    await flush();
  }
  fireEvent.pointerUp(window, { clientX: to, clientY: 2 });
  await flush();
}

const bar = () => screen.queryByTestId("piano-roll-stretch");
const undo = () => send(cmd("Edit", { type: "Undo" }));

describe("note stretch bar (base-109)", () => {
  it("is hidden without a selection; spans the selected notes, or a section with notes", async () => {
    const clip = await setup();
    expect(bar()).toBeNull();
    act(() => itemSelection.getState().select("note", [A, B], "replace"));
    expect(bar()!.style.left).toBe(`${x(1)}px`);
    expect(bar()!.style.width).toBe(`${x(2)}px`);
    act(() => {
      itemSelection.getState().clear("note");
      usePianoRollSection.getState().setSection({ clip, start: 0, end: 4 });
    });
    expect(bar()!.style.left).toBe("0px");
    expect(bar()!.style.width).toBe(`${x(4)}px`);
    // A section without notes has nothing to stretch.
    act(() => usePianoRollSection.getState().setSection({ clip, start: 3, end: 4 }));
    expect(bar()).toBeNull();
    // A zero-length section (the insert marker) shows no bar of its own...
    act(() => usePianoRollSection.getState().setSection({ clip, start: 2, end: 2 }));
    expect(bar()).toBeNull();
    // ...and leaves the selected notes' bar alone.
    act(() => itemSelection.getState().select("note", [A, B], "replace"));
    expect(bar()!.style.left).toBe(`${x(1)}px`);
    expect(bar()!.style.width).toBe(`${x(2)}px`);
  });

  it("hovering the edges shows a resize cursor, the body a grab cursor", async () => {
    await setup();
    act(() => itemSelection.getState().select("note", [A, B], "replace"));
    const el = bar()!;
    fireEvent.pointerMove(el, { clientX: x(1) + 2 });
    expect(el.style.cursor).toBe("ew-resize");
    fireEvent.pointerMove(el, { clientX: x(2) });
    expect(el.style.cursor).toBe("grab");
    fireEvent.pointerMove(el, { clientX: x(3) - 2 });
    expect(el.style.cursor).toBe("ew-resize");
  });

  it("the owner's case: a section 1..4, drag the right edge to 7, notes scale about 1; one gesture, one undo", async () => {
    const clip = await setup();
    await drag(screen.getByTestId("piano-roll-loopbar"), x(1), x(4));
    expect(usePianoRollSection.getState().section).toEqual({ clip, start: 1, end: 4 });
    const spy = vi.spyOn(mock!, "send");
    await drag(bar()!, x(4) - 2, x(7) - 2);
    expect(placed(clip)).toEqual([
      [1, 2],
      [3, 2],
    ]);
    // The section follows; the clip grew to hold it.
    expect(usePianoRollSection.getState().section).toEqual({ clip, start: 1, end: 7 });
    expect(store().project!.clips[clip]!.length).toBe(7);
    // Every edit of the drag carries one gesture, closed once.
    const sent = spy.mock.calls.filter(([c]) => !(c.domain === "Edit" && c.command.type === "EndGesture"));
    const gestures = new Set(sent.map(([, o]) => o?.gesture));
    expect(sent.length).toBeGreaterThan(0);
    expect(gestures.size).toBe(1);
    expect([...gestures][0]).toBeDefined();
    expect(spy.mock.calls.filter(([c]) => c.domain === "Edit" && c.command.type === "EndGesture")).toHaveLength(1);
    await undo();
    expect(placed(clip)).toEqual([
      [1, 1],
      [2, 1],
    ]);
    expect(store().project!.clips[clip]!.length).toBe(4);
  });

  it("the left edge scales about the right edge; Alt disables snapping", async () => {
    const clip = await setup();
    act(() => itemSelection.getState().select("note", [A, B], "replace"));
    // Selection 1..3: drag the left edge to 2 (compress ×0.5 toward 3).
    await drag(bar()!, x(1) + 2, x(2) + 2);
    expect(placed(clip)).toEqual([
      [2, 0.5],
      [2.5, 0.5],
    ]);
    await undo();
    // Alt: an off-grid edge (right edge 3 → 3.5 + a bit).
    await drag(bar()!, x(3) - 2, x(3.6) - 2, { altKey: true });
    const [a] = placed(clip);
    expect(a![1]).toBeCloseTo(1.3);
  });

  it("dragging the body moves the notes; ×2 doubles the selection in one step", async () => {
    const clip = await setup();
    act(() => itemSelection.getState().select("note", [A, B], "replace"));
    await drag(bar()!, x(2), x(2.9));
    expect(placed(clip)).toEqual([
      [2, 1],
      [3, 1],
    ]);
    await undo();
    fireEvent.click(screen.getByRole("button", { name: "×2" }));
    await flush();
    expect(placed(clip)).toEqual([
      [1, 2],
      [3, 2],
    ]);
    await undo();
    expect(placed(clip)).toEqual([
      [1, 1],
      [2, 1],
    ]);
  });
});
