import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Command } from "@/generated";
import { pickOption } from "@/kit/testing";
import { useEditorStore, useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, TransportProvider } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { PianoRoll } from "@/features/piano-roll";
import { ConfirmDialog, ContextMenuHost, useConfirm } from "@/kit";

// The piano roll's view starts at 40 px/beat, scrolled to 0; jsdom boxes are at (0, 0), so
// client coordinates are lane-local px. Lanes are 72 px tall (values padded by 4 px).
const PX = 40;
const H = 72;

let mock: MockTransport | undefined;
const store = () => useProjectStore.getState();
const lanes = () => Object.values(store().project!.expression_lanes);
const exprs = () => Object.values(store().project!.note_expressions);

afterEach(() => {
  mock?.dispose();
  mock = undefined;
  store().reset();
  useEditorStore.getState().close();
  itemSelection.getState().clear();
  useConfirm.getState().close();
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

const CLIP = "01EXPRESSIONTESTCLIP000000";
const NOTE = "01EXPRESSIONTESTNOTE000000";

async function setup() {
  mock = new MockTransport({ timers: "manual", seed: 1 });
  render(
    <TransportProvider transport={mock}>
      <PianoRoll />
      <ContextMenuHost />
      <ConfirmDialog />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  const track = Object.values(store().project!.tracks).find((t) => t.kind === "Midi")!;
  await send(cmd("Clip", { type: "CreateMidi", id: CLIP, track: track.id, start: 64, length: 8, name: "Expr" }));
  await send(cmd("Note", { type: "Add", clip: CLIP, notes: [{ id: NOTE, pitch: 60, velocity: 0.8, start: 1, duration: 2 }] }));
  act(() => useEditorStore.getState().openClip(CLIP));
}

const picker = () => screen.getByRole("combobox", { name: "lane" });

async function stroke(el: Element, from: [number, number], to: [number, number]) {
  fireEvent.pointerDown(el, { button: 0, clientX: from[0], clientY: from[1] });
  const steps = 8;
  for (let i = 1; i <= steps; i++) {
    fireEvent.pointerMove(window, {
      clientX: from[0] + ((to[0] - from[0]) * i) / steps,
      clientY: from[1] + ((to[1] - from[1]) * i) / steps,
    });
    await flush();
  }
  fireEvent.pointerUp(window, { clientX: to[0], clientY: to[1] });
  await flush();
}

describe("expression lanes under the piano roll", () => {
  it("velocity by default; picking Pitch Bend shows an empty lane", async () => {
    await setup();
    expect(screen.getByTestId("piano-roll-velocity")).toBeTruthy();
    pickOption(picker(), "pitch bend");
    const lane = screen.getByTestId("expression-lane");
    expect(lane.getAttribute("data-kind")).toBe("pitch bend");
    expect(lane.getAttribute("data-points")).toBe("0");
    expect(lanes()).toEqual([]);
  });

  it("a pencil stroke creates the lane and draws, as one undo step", async () => {
    await setup();
    pickOption(picker(), "pitch bend");
    // From beat 0 at the top (bend +1) to beat 2 at the bottom (bend -1).
    await stroke(screen.getByTestId("expression-lane"), [0, 4], [2 * PX, H - 4]);
    expect(lanes()).toHaveLength(1);
    const l = lanes()[0]!;
    expect(l).toMatchObject({ clip: CLIP, kind: { type: "PitchBend" } });
    expect(l.points.length).toBeGreaterThan(3);
    expect(l.points[0]!.value).toBeCloseTo(1);
    expect(l.points[l.points.length - 1]!).toMatchObject({ time: 2 });
    expect(l.points[l.points.length - 1]!.value).toBeCloseTo(-1);
    expect(screen.getByTestId("expression-curve")).toBeTruthy();
    expect(screen.getByRole("combobox", { name: "lane" }).textContent).toContain("pitch bend •");

    // Redrawing a part replaces only that range.
    const before = l.points.length;
    await stroke(screen.getByTestId("expression-lane"), [4 * PX, H / 2], [5 * PX, H / 2]);
    expect(lanes()[0]!.points.length).toBeGreaterThan(before);

    await send(cmd("Edit", { type: "Undo" }));
    expect(lanes()[0]!.points.length).toBe(before);
    await send(cmd("Edit", { type: "Undo" }));
    expect(lanes()).toEqual([]);
  });

  it("Remove lane deletes the clip's lane", async () => {
    await setup();
    await send(cmd("Expression", { type: "CreateLane", id: "01EXPRESSIONTESTLANE000000", clip: CLIP, kind: { type: "Cc", controller: 1 } }));
    pickOption(picker(), /cc 1 mod wheel/);
    fireEvent.click(screen.getByRole("button", { name: "remove lane" }));
    await flush();
    expect(lanes()).toEqual([]);
  });

  it("Other CC… picks any controller 0..119", async () => {
    await setup();
    pickOption(picker(), "other cc…");
    const field = screen.getByLabelText("controller number");
    fireEvent.change(field, { target: { value: "20" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await flush();
    expect(screen.getByTestId("expression-lane").getAttribute("data-kind")).toBe("cc 20");
    await stroke(screen.getByTestId("expression-lane"), [0, H / 2], [PX, H / 2]);
    expect(lanes()[0]!.kind).toEqual({ type: "Cc", controller: 20 });
  });

  it("note pressure: drawing over a note sets its Pressure curve (times from the note start)", async () => {
    await setup();
    pickOption(picker(), "note pressure");
    await stroke(screen.getByTestId("note-expression-lane"), [0.5 * PX, H - 4], [2.5 * PX, 4]);
    expect(exprs()).toHaveLength(1);
    const e = exprs()[0]!;
    expect(e).toMatchObject({ note: NOTE, kind: "Pressure" });
    expect(e.points[0]!.time).toBeGreaterThanOrEqual(0);
    expect(e.points[e.points.length - 1]!.time).toBeLessThan(2);
    expect(e.points[e.points.length - 1]!.value).toBeGreaterThan(0.6);
    expect(screen.getByTestId("note-expression-curve")).toBeTruthy();
    await send(cmd("Edit", { type: "Undo" }));
    expect(exprs()).toEqual([]);
  });

  describe("clearing and removing ask first (#717)", () => {
    const LANE = "01EXPRESSIONTESTLANE000000";
    const POINTS = [
      { time: 0, value: 0.2, curve: { type: "Linear" as const } },
      { time: 1, value: 0.8, curve: { type: "Linear" as const } },
    ];

    async function withPoints() {
      await setup();
      await send(cmd("Expression", { type: "CreateLane", id: LANE, clip: CLIP, kind: { type: "Cc", controller: 1 } }));
      await send(cmd("Expression", { type: "SetPoints", lane: LANE, points: POINTS }));
      pickOption(picker(), /cc 1 mod wheel/);
    }

    it("right-click → clear asks; cancel keeps the points, clear removes them", async () => {
      await withPoints();
      fireEvent.contextMenu(screen.getByTestId("expression-lane"));
      fireEvent.click(await screen.findByRole("menuitem", { name: /^clear cc 1/ }));
      const dialog = await screen.findByRole("dialog");
      expect(dialog).toHaveTextContent("this removes the lane’s 2 points. undo brings them back.");
      fireEvent.click(screen.getByRole("button", { name: "cancel" }));
      await flush();
      expect(lanes()[0]!.points).toHaveLength(2);

      fireEvent.contextMenu(screen.getByTestId("expression-lane"));
      fireEvent.click(await screen.findByRole("menuitem", { name: /^clear cc 1/ }));
      fireEvent.click(await screen.findByRole("button", { name: "clear" }));
      await flush();
      expect(lanes()[0]!.points).toEqual([]);
      await send(cmd("Edit", { type: "Undo" }));
      expect(lanes()[0]!.points).toHaveLength(2);
    });

    it("remove lane with points asks; an empty lane goes at once", async () => {
      await withPoints();
      fireEvent.click(screen.getByRole("button", { name: "remove lane" }));
      await flush();
      expect(lanes()).toHaveLength(1);
      fireEvent.click(await screen.findByRole("button", { name: "remove" }));
      await flush();
      expect(lanes()).toEqual([]);
    });

    it("note lanes: clearing curves asks first", async () => {
      await setup();
      pickOption(picker(), "note pressure");
      await stroke(screen.getByTestId("note-expression-lane"), [0.5 * PX, H - 4], [2.5 * PX, 4]);
      expect(exprs()).toHaveLength(1);
      fireEvent.contextMenu(screen.getByTestId("note-expression-lane"));
      fireEvent.click(await screen.findByRole("menuitem", { name: /^clear/ }));
      expect(await screen.findByRole("dialog")).toHaveTextContent("of 1 note");
      expect(exprs()).toHaveLength(1);
      fireEvent.click(screen.getByRole("button", { name: "clear" }));
      await flush();
      expect(exprs()).toEqual([]);
    });
  });
});
