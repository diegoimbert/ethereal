import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Command } from "@/generated";
import { useEditorStore, useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, TransportProvider } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { pickOption } from "@/kit/testing";
import { DEFAULT_KEY_HEIGHT as KEY_H } from "./geometry";
import { PianoRoll } from "./index";
import { heldPreview, stopPreview, usePreviewSetting } from "./notePreview";
import { resetLastVelocity } from "./lastVelocity";

// Same jsdom geometry as PianoRoll.test.tsx: 40 px/beat, the grid at (0, 0).
const PX = 40;
const x = (beats: number) => beats * PX;
const y = (pitch: number) => (127 - pitch) * KEY_H + KEY_H / 2;

let mock: MockTransport | undefined;
const store = () => useProjectStore.getState();
const A = "01PIANOROLLNOTEA0000000000";
const B = "01PIANOROLLNOTEB0000000000";

beforeEach(() => {
  localStorage.clear();
  usePreviewSetting.setState({ enabled: true });
  // Notes are drawn on a canvas (none in jsdom): presses on the grid are hit-tested.
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
});

afterEach(() => {
  stopPreview();
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

/** A MIDI clip with notes C4 (velocity 0.8) and E4 (0.5), open in the piano roll. */
async function setup() {
  mock = new MockTransport({ timers: "manual", seed: 1 });
  const view = render(
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
        { id: A, pitch: 60, velocity: 0.8, start: 1, duration: 1 },
        { id: B, pitch: 64, velocity: 0.5, start: 2, duration: 1 },
      ],
    }),
  );
  act(() => useEditorStore.getState().openClip(clip));
  pickOption(screen.getByRole("combobox", { name: "grid" }), { value: "5" });
  return { track: track.id, view };
}

/** Presses on a note land on the grid at its position (jsdom: client px = grid px). */
const grid = () => screen.getByTestId("piano-roll-grid");
const sent = () => mock!.previewNotes.map((p) => p.data);

describe("piano-roll note preview (base-150)", () => {
  it("pressing a note plays it on the clip's track at its pitch and velocity until release", async () => {
    const { track } = await setup();
    fireEvent.pointerDown(grid(), { button: 0, clientX: x(1.5), clientY: y(60) });
    await flush();
    expect(mock!.previewNotes).toEqual([{ track, data: [0x90, 60, Math.round(0.8 * 127)] }]);
    expect(heldPreview()).toEqual({ track, pitch: 60 });
    fireEvent.pointerUp(window, { clientX: x(1.5), clientY: y(60) });
    await flush();
    expect(sent()).toEqual([
      [0x90, 60, 102],
      [0x80, 60, 0],
    ]);
    expect(heldPreview()).toBeNull();
  });

  it("a vertical drag re-auditions at each new pitch (old note off first), one off at the end", async () => {
    await setup();
    fireEvent.pointerDown(grid(), { button: 0, clientX: x(1.5), clientY: y(60) });
    for (const p of [61, 62, 62]) {
      fireEvent.pointerMove(window, { clientX: x(1.5), clientY: y(p) });
      await flush();
    }
    fireEvent.pointerUp(window, { clientX: x(1.5), clientY: y(62) });
    await flush();
    expect(sent()).toEqual([
      [0x90, 60, 102],
      [0x80, 60, 0],
      [0x90, 61, 102],
      [0x80, 61, 0],
      [0x90, 62, 102],
      [0x80, 62, 0],
    ]);
    // The move itself still happened, as one undo step (previews don't split the gesture).
    expect(store().project!.notes[A]!.pitch).toBe(62);
    await send(cmd("Edit", { type: "Undo" }));
    expect(store().project!.notes[A]!.pitch).toBe(60);
  });

  it("shift and cmd selection clicks audition too; pointer cancel and blur release", async () => {
    await setup();
    fireEvent.pointerDown(grid(), { button: 0, clientX: x(1.5), clientY: y(60) });
    fireEvent.pointerCancel(window);
    fireEvent.pointerDown(grid(), { button: 0, shiftKey: true, clientX: x(2.5), clientY: y(64) });
    fireEvent.blur(window);
    fireEvent.pointerUp(window);
    fireEvent.pointerDown(grid(), { button: 0, metaKey: true, clientX: x(1.5), clientY: y(60) });
    fireEvent.pointerUp(window);
    await flush();
    expect(sent()).toEqual([
      [0x90, 60, 102],
      [0x80, 60, 0],
      [0x90, 64, 64],
      [0x80, 64, 0],
      [0x90, 60, 102],
      [0x80, 60, 0],
    ]);
  });

  it("drawing a note auditions it; closing the editor releases a held note", async () => {
    await setup();
    fireEvent.pointerDown(grid(), { button: 0, clientX: x(4.5), clientY: y(67) });
    fireEvent.pointerUp(window);
    fireEvent.pointerDown(grid(), { button: 0, clientX: x(4.5), clientY: y(67) });
    await flush();
    expect(sent()).toEqual([[0x90, 67, 100]]);
    act(() => useEditorStore.getState().close());
    await flush();
    expect(sent()).toEqual([
      [0x90, 67, 100],
      [0x80, 67, 0],
    ]);
  });

  it("clicking a key previews its pitch", async () => {
    await setup();
    const key = document.querySelector('[data-testid="piano-roll-keys"] [data-pitch="72"]')!;
    fireEvent.pointerDown(key, { button: 0 });
    fireEvent.pointerUp(window);
    await flush();
    expect(sent()).toEqual([
      [0x90, 72, 100],
      [0x80, 72, 0],
    ]);
  });

  it("a black key's row plays it right of the key too", async () => {
    await setup();
    // The black key covers the left 65% of its row; the rest is the keyboard itself.
    fireEvent.pointerDown(screen.getByTestId("piano-roll-keys"), { button: 0, clientY: y(73) });
    fireEvent.pointerUp(window);
    await flush();
    expect(sent()).toEqual([
      [0x90, 73, 100],
      [0x80, 73, 0],
    ]);
  });

  it("the toolbar toggle (on by default, remembered) turns previews off", async () => {
    await setup();
    const toggle = screen.getByTestId("piano-roll-preview");
    expect(toggle.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(toggle);
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    expect(localStorage.getItem("ethereal.pianoRoll.preview")).toBe("off");
    fireEvent.pointerDown(grid(), { button: 0, clientX: x(1.5), clientY: y(60) });
    fireEvent.pointerMove(window, { clientX: x(1.5), clientY: y(62) });
    await flush();
    fireEvent.pointerUp(window);
    await flush();
    expect(sent()).toEqual([]);
    fireEvent.click(toggle);
    expect(localStorage.getItem("ethereal.pianoRoll.preview")).toBe("on");
  });
});
