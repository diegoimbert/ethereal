import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Command } from "@/generated";
import { useEditorStore, useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { resetDeviceWindows, useDeviceWindows } from "@/features/devices/window";
import { size } from "@/theme";
import { PianoRoll } from "./index";

let mock: MockTransport | undefined;
const store = () => useProjectStore.getState();

afterEach(() => {
  mock?.dispose();
  mock = undefined;
  store().reset();
  useEditorStore.getState().close();
  itemSelection.getState().clear();
  resetDeviceWindows();
});

async function send(command: Command) {
  await act(async () => {
    await mock!.send(command);
  });
}

const RACK = "01BEATGRIDRACK000000000000";
const CLIP = "01BEATGRIDCLIP000000000000";

/** A MIDI track playing a Drum Rack with a "Kick" pad (C1) and an unnamed one (D1), a 1-bar clip opened. */
async function setup() {
  mock = new MockTransport({ timers: "manual", seed: 1 });
  render(
    <TransportProvider transport={mock}>
      <PianoRoll />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  const track = Object.values(store().project!.tracks).find((t) => t.kind === "Midi")!;
  await send(cmd("Device", { type: "Insert", id: RACK, track: track.id, device: { type: "Builtin", device: { type: "DrumRack" } }, before: null }));
  await send(cmd("DrumRack", { type: "AddPad", id: "01BEATGRIDPADKICK000000000", rack: RACK, note: 36, name: "Kick" }));
  await send(cmd("DrumRack", { type: "AddPad", id: "01BEATGRIDPADSNARE00000000", rack: RACK, note: 38, name: null }));
  await send(cmd("Clip", { type: "CreateMidi", id: CLIP, track: track.id, start: 0, length: 4, name: "Beat" }));
  act(() => useEditorStore.getState().openClip(CLIP));
  return track;
}

const notes = () => Object.values(store().project!.notes).filter((n) => n.clip === CLIP);
/** Canvas coordinates of a step (1-based) on a row (0-based): the view opens at the clip's
 *  start, a 16th step 22 px wide (88 px per beat), rows `--eth-size-beat-row-height` tall;
 *  jsdom's canvas sits at 0,0. */
const ROW_H = parseFloat(size.beatRowHeight);
const at = (row: number, step: number) => ({ clientX: (step - 1) * 22 + 5, clientY: row * ROW_H + 5 });
const canvas = () => screen.getByTestId("beat-canvas");

describe("BeatGrid (step sequencer on Drum Rack tracks)", () => {
  it("shows a row per pad (its note then name, else its note), lowest first, the steps on one canvas", async () => {
    await setup();
    const grid = await screen.findByRole("grid", { name: "steps" });
    expect(
      within(grid)
        .getAllByRole("rowheader")
        .map((h) => h.textContent),
    ).toEqual(["C1Kick", "D1"]);
    expect(canvas().style.height).toBe(`${2 * ROW_H}px`);
  });

  it("left click toggles a hit, right click removes it; a drag paints a row in one undo step", async () => {
    await setup();
    await screen.findByRole("grid", { name: "steps" });
    fireEvent.pointerDown(canvas(), { button: 0, ...at(0, 1) });
    fireEvent.pointerUp(window);
    await waitFor(() => expect(notes().map((n) => [n.pitch, n.start, n.duration])).toEqual([[36, 0, 0.25]]));
    // Left click on a hit removes it; so does the right button.
    fireEvent.pointerDown(canvas(), { button: 0, ...at(0, 1) });
    fireEvent.pointerUp(window);
    await waitFor(() => expect(notes()).toEqual([]));
    fireEvent.pointerDown(canvas(), { button: 0, ...at(0, 2) });
    fireEvent.pointerUp(window);
    await waitFor(() => expect(notes()).toHaveLength(1));
    fireEvent.pointerDown(canvas(), { button: 2, ...at(0, 2) });
    fireEvent.pointerUp(window);
    await waitFor(() => expect(notes()).toEqual([]));

    // Painting stays on the pressed row (the move into the Kick row adds nothing there).
    fireEvent.pointerDown(canvas(), { button: 0, ...at(1, 5) });
    fireEvent.pointerMove(window, at(1, 6));
    fireEvent.pointerMove(window, at(0, 6));
    fireEvent.pointerMove(window, at(1, 7));
    fireEvent.pointerUp(window);
    await waitFor(() =>
      expect(
        notes()
          .map((n) => [n.pitch, n.start])
          .sort(),
      ).toEqual([
        [38, 1],
        [38, 1.25],
        [38, 1.5],
      ]),
    );
    // A right-button drag erases along the row.
    fireEvent.pointerDown(canvas(), { button: 2, ...at(1, 5) });
    fireEvent.pointerMove(window, at(1, 6));
    fireEvent.pointerUp(window);
    await waitFor(() => expect(notes().map((n) => n.start)).toEqual([1.5]));
    await send(cmd("Edit", { type: "Undo" }));
    await send(cmd("Edit", { type: "Undo" }));
    expect(notes()).toEqual([]);
  });

  it('without pads: the empty grid with an "add drums" button that opens the Drum Rack', async () => {
    mock = new MockTransport({ timers: "manual", seed: 1 });
    render(
      <TransportProvider transport={mock}>
        <PianoRoll />
      </TransportProvider>,
    );
    await waitFor(() => expect(store().project).not.toBeNull());
    const track = Object.values(store().project!.tracks).find((t) => t.kind === "Midi")!;
    await send(cmd("Device", { type: "Insert", id: RACK, track: track.id, device: { type: "Builtin", device: { type: "DrumRack" } }, before: null }));
    await send(cmd("Clip", { type: "CreateMidi", id: CLIP, track: track.id, start: 0, length: 4, name: "Beat" }));
    act(() => useEditorStore.getState().openClip(CLIP));
    expect(await screen.findByRole("grid", { name: "steps" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "add drums" }));
    expect(useDeviceWindows.getState().open).toContain(RACK);
  });

  it("a clip extended to the left past its content start opens at its start: steps in the gap add notes there", async () => {
    await setup();
    // Song 4..8, then its left edge dragged 1 beat earlier: content -1..4 (a leading gap).
    await send(cmd("Clip", { type: "SetBounds", id: CLIP, start: 4, length: 4, offset: 0 }));
    await send(cmd("Clip", { type: "SetBounds", id: CLIP, start: 3, length: 5, offset: -1 }));
    act(() => useEditorStore.getState().close());
    act(() => useEditorStore.getState().openClip(CLIP));
    await screen.findByRole("grid", { name: "steps" });
    fireEvent.pointerDown(canvas(), { button: 0, ...at(0, 2) });
    fireEvent.pointerUp(window);
    await waitFor(() => expect(notes().map((n) => [n.pitch, n.start])).toEqual([[36, -0.75]]));
  });
});
