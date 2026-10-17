import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AutomationLane, AutomationPoint, CurveShape, Track } from "@/generated";
import { useProjectStore, useSelectionStore } from "@/state";
import { createTimelineViewStore, DEFAULT_GRID, itemSelection, resolveGrid, snapToGrid, TempoMap, type TimelineViewStore } from "@/timeline";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { AutomationToggleButton } from "./AutomationToggleButton";
import { AutomationLanes, TrackAutomationLanes } from "./index";
import { LANE_PAD } from "./geometry";
import { AUTOMATION_BAR_HEIGHT, LANE_HEIGHT, resetAutomationUi, useAutomationUi } from "./uiStore";
import { pickOption } from "@/kit/testing";
import { resetTimeSelection, useTimeSelection } from "@/features/time-edits/store";
import { resetArrangementUi, useArrangementUi } from "@/features/arrangement/uiStore";
import { valueAt, valueBefore } from "./range";
import { evaluatePoints } from "./curve";

const store = () => useProjectStore.getState();
const project = () => store().project!;

let mock: MockTransport | undefined;
let view: TimelineViewStore;

// jsdom has no layout: element rects are all zero, so client coords are lane-local px.
const PX_PER_BEAT = 10;
const y = (v: number) => LANE_PAD + (1 - v) * (LANE_HEIGHT - 2 * LANE_PAD);

function keysTrack(): Track {
  return Object.values(project().tracks).find((t) => t.name === "Keys")!;
}

function volumeLane(track: Track): AutomationLane {
  return Object.values(project().automation_lanes).find((l) => l.target.type === "TrackVolume" && l.owner.type === "Track" && l.owner.track === track.id)!;
}

function lanePoints(lane: string): AutomationPoint[] {
  return Object.values(project().automation_points)
    .filter((p) => p.lane === lane)
    .sort((a, b) => a.time - b.time);
}

async function flush(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

async function renderLanes(track?: () => Track): Promise<Track> {
  mock = new MockTransport({ timers: "manual", seed: 1 });
  let t: Track | undefined;
  const Probe = () => {
    const p = useProjectStore((s) => s.project);
    if (!p) return null;
    t ??= (track ?? keysTrack)();
    return (
      <>
        <AutomationToggleButton trackId={t.id} trackName={t.name} />
        <TrackAutomationLanes trackId={t.id} view={view} />
      </>
    );
  };
  render(
    <TransportProvider transport={mock}>
      <Probe />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  await flush();
  return t!;
}

async function send(command: Parameters<MockTransport["send"]>[0]) {
  await act(async () => {
    await mock!.send(command);
  });
}

/** A press and release at `at` (window-level release, like the lane's drags). */
async function press(el: Element, at: [number, number], opts: { shiftKey?: boolean; altKey?: boolean } = {}) {
  await act(async () => {
    fireEvent.pointerDown(el, { button: 0, clientX: at[0], clientY: at[1], ...opts });
  });
  await flush();
  await act(async () => {
    fireEvent.pointerUp(window, { clientX: at[0], clientY: at[1], ...opts });
  });
  await flush();
}

/** A double-click as the lane sees it: two presses (it acts on the second pointerdown). */
async function doublePress(el: Element, at: [number, number]) {
  await press(el, at);
  await press(el, at);
}

async function drag(el: Element, from: [number, number], to: [number, number], opts: { altKey?: boolean; shiftKey?: boolean; ctrlKey?: boolean } = {}) {
  await act(async () => {
    fireEvent.pointerDown(el, { button: 0, clientX: from[0], clientY: from[1], ...opts });
  });
  for (let i = 1; i <= 4; i++) {
    const x = from[0] + ((to[0] - from[0]) * i) / 4;
    const yy = from[1] + ((to[1] - from[1]) * i) / 4;
    await act(async () => {
      fireEvent.pointerMove(window, { clientX: x, clientY: yy, ...opts });
    });
    await flush();
  }
  await act(async () => {
    fireEvent.pointerUp(window, { clientX: to[0], clientY: to[1], ...opts });
  });
  await flush();
}

function openTrack() {
  fireEvent.click(screen.getByRole("button", { name: /show automation of/ }));
}

/** The lane's grid step at the test zoom. */
const gridStep = () => resolveGrid(DEFAULT_GRID, PX_PER_BEAT, TempoMap.constant().signatureAt(0));

beforeEach(() => {
  resetAutomationUi();
  resetTimeSelection();
  resetArrangementUi();
  view = createTimelineViewStore({ pxPerBeat: PX_PER_BEAT });
  view.getState().setWidth(400);
});

afterEach(() => {
  mock?.dispose();
  mock = undefined;
  store().reset();
  itemSelection.getState().clear();
  useSelectionStore.getState().selectTrack(null);
});

describe("TrackAutomationLanes", () => {
  it("takes no room until opened, then shows the existing lanes", async () => {
    const track = await renderLanes();
    expect(document.querySelector(".eth-auto-track")).toHaveStyle({ height: "0px" });
    expect(screen.queryByTestId("automation-lane-svg")).toBeNull();
    openTrack();
    await flush();
    expect(document.querySelector(".eth-auto-track")).toHaveStyle({ height: `${AUTOMATION_BAR_HEIGHT + LANE_HEIGHT}px` });
    expect(useAutomationUi.getState().shown[track.id]).toEqual([`volume:${track.id}`]);
    const circles = document.querySelectorAll("circle[data-point]");
    expect(circles).toHaveLength(3);
    // Point at beat 8, value 0.9.
    const second = circles[1]!;
    expect(Number(second.getAttribute("cx"))).toBe(80);
    expect(Number(second.getAttribute("cy"))).toBeCloseTo(y(0.9), 6);
    // The value readout goes through ParamInfo.scale (fader law, dB).
    expect(second.querySelector("title")?.textContent).toMatch(/db$/);
  });

  it("does no work for a closed track: no parameter lists or descriptors until opened (audit #12)", async () => {
    const sent: string[] = [];
    const orig = MockTransport.prototype.send;
    const spy = vi.spyOn(MockTransport.prototype, "send").mockImplementation(function (this: MockTransport, c, o) {
      sent.push(`${c.domain}:${c.command.type}`);
      return orig.call(this, c, o);
    });
    try {
      const track = await renderLanes();
      expect(Object.values(project().devices).some((d) => d.track === track.id)).toBe(true);
      // A device change (as on every knob drag) while closed: nothing is computed or fetched.
      const device = Object.values(project().devices).find((d) => d.track === track.id)!;
      await send(cmd("Device", { type: "SetEnabled", id: device.id, enabled: false }));
      await flush();
      expect(sent.filter((s) => s === "Device:GetDescriptor")).toEqual([]);
      expect(document.querySelector(".eth-auto-track")!.childElementCount).toBe(0);
      openTrack();
      await flush();
      expect(screen.getAllByTestId("automation-lane-svg")).toHaveLength(1);
      expect(sent.filter((s) => s === "Device:GetDescriptor").length).toBeGreaterThan(0);
    } finally {
      spy.mockRestore();
    }
  });

  it("shows and hides lanes per parameter", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    pickOption(screen.getByRole("combobox", { name: "show parameter" }), { value: `pan:${track.id}` });
    await flush();
    expect(screen.getAllByTestId("automation-lane-svg")).toHaveLength(2);
    fireEvent.click(screen.getAllByRole("button", { name: "hide lane" })[0]!);
    await flush();
    expect(screen.getAllByTestId("automation-lane-svg")).toHaveLength(1);
    // Hiding keeps the document lane.
    expect(volumeLane(track)).toBeDefined();
  });

  it("moves points with snapping as one undo step", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    const lane = volumeLane(track);
    const [, mid] = lanePoints(lane.id);
    const circle = document.querySelector(`circle[data-point="${mid!.id}"]`)!;
    // Drag right by 33 px (3.3 beats: snaps to the adaptive grid) and down to value 0.5.
    await drag(circle, [80, y(0.9)], [113, y(0.5)]);
    const moved = project().automation_points[mid!.id]!;
    const step = resolveGrid(DEFAULT_GRID, PX_PER_BEAT, TempoMap.constant().signatureAt(0));
    expect(moved.time).toBe(snapToGrid(11.3, step, TempoMap.constant()));
    expect(moved.time).not.toBeCloseTo(11.3, 3);
    expect(moved.value).toBeCloseTo(0.5, 6);
    expect(itemSelection.getState().selected.automationPoint.has(mid!.id)).toBe(true);

    await send(cmd("Edit", { type: "Undo" }));
    expect(project().automation_points[mid!.id]).toMatchObject({ time: 8, value: 0.9 });
    expect(store().history.can_undo).toBe(false);
  });

  it("alt bypasses snapping", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    const [, mid] = lanePoints(volumeLane(track).id);
    const circle = document.querySelector(`circle[data-point="${mid!.id}"]`)!;
    await drag(circle, [80, y(0.9)], [83, y(0.9)], { altKey: true });
    // ⌥: off the grid, and a precision drag (1/10 of the 3 px, #793).
    expect(project().automation_points[mid!.id]!.time).toBeCloseTo(8.03, 6);
  });

  it("an empty lane draws the current value dashed; the first point takes it, creating the lane in the same undo step", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    await send(cmd("Mixer", { type: "SetPan", track: track.id, pan: -0.5 }));
    pickOption(screen.getByRole("combobox", { name: "show parameter" }), { value: `pan:${track.id}` });
    await flush();
    const panSvg = screen.getAllByTestId("automation-lane-svg")[1]!;
    // Pan 50 % left: a quarter of the way up (bipolar).
    const baseline = () => panSvg.querySelector<SVGLineElement>('[data-testid="automation-baseline"]');
    expect(Number(baseline()!.getAttribute("y1"))).toBeCloseTo(y(0.25), 6);
    // It follows the knob.
    await send(cmd("Mixer", { type: "SetPan", track: track.id, pan: 0.5 }));
    expect(Number(baseline()!.getAttribute("y1"))).toBeCloseTo(y(0.75), 6);
    expect(panSvg.dataset.baseline).toBe("0.75");

    // Double-click far from the line: the point takes the current value, not the pointer's.
    await doublePress(panSvg, [40, y(0.1)]);
    const lane = Object.values(project().automation_lanes).find((l) => l.target.type === "TrackPan")!;
    expect(lane).toBeDefined();
    const pts = lanePoints(lane.id);
    expect(pts).toHaveLength(1);
    expect(pts[0]).toMatchObject({ time: 4, value: 0.75, curve: { type: "Linear" } });
    expect(itemSelection.getState().selected.automationPoint.has(pts[0]!.id)).toBe(true);
    // With a point, no baseline; the next point takes the lane's value there too (held).
    expect(baseline()).toBeNull();
    await doublePress(panSvg, [120, y(0.1)]);
    expect(lanePoints(lane.id).map((p) => p.value)).toEqual([0.75, 0.75]);

    await send(cmd("Edit", { type: "Undo" }));
    await send(cmd("Edit", { type: "Undo" }));
    expect(Object.values(project().automation_lanes).some((l) => l.target.type === "TrackPan")).toBe(false);
    expect(baseline()).not.toBeNull();
  });

  it("⌘/Ctrl-drag marquee-selects points, deletes them with Delete, and removes one with double-click", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    const lane = volumeLane(track);
    const svg = screen.getByTestId("automation-lane-svg");
    await drag(svg, [-5, 0], [90, LANE_HEIGHT], { ctrlKey: true });
    const [a, b, c] = lanePoints(lane.id);
    const sel = itemSelection.getState().selected.automationPoint;
    expect(sel.has(a!.id) && sel.has(b!.id) && !sel.has(c!.id)).toBe(true);

    const group = screen.getByRole("group", { name: "volume automation" });
    fireEvent.keyDown(group, { key: "Delete" });
    await flush();
    expect(lanePoints(lane.id).map((p) => p.id)).toEqual([c!.id]);

    await doublePress(document.querySelector(`circle[data-point="${c!.id}"]`)!, [160, y(0.6)]);
    expect(lanePoints(lane.id)).toHaveLength(0);
  });

  it("sets the curve type of the selected points", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    const lane = volumeLane(track);
    const [a] = lanePoints(lane.id);
    const select = screen.getByRole("combobox", { name: "curve of selected points" });
    expect(select).toBeDisabled();
    await act(async () => {
      fireEvent.pointerDown(document.querySelector(`circle[data-point="${a!.id}"]`)!, { button: 0, clientX: 0, clientY: y(0.7) });
      fireEvent.pointerUp(window, { clientX: 0, clientY: y(0.7) });
    });
    await flush();
    expect(select).toBeEnabled();
    pickOption(select, { value: "Step" });
    await flush();
    expect(project().automation_points[a!.id]!.curve).toEqual({ type: "Step" });
    pickOption(select, { value: "Curve" });
    await flush();
    expect(project().automation_points[a!.id]!.curve).toEqual({ type: "Curve", tension: 0.5 });
    // The drawn path follows the curve: not a straight line between the two points.
    const d = document.querySelector(".eth-auto-lane__line")!.getAttribute("d")!;
    expect(d.split("L").length).toBeGreaterThan(20);
  });

  it("alt-drag on a segment bends it as one undo step", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    const lane = volumeLane(track);
    const [a] = lanePoints(lane.id);
    const svg = screen.getByTestId("automation-lane-svg");
    // Segment 0 → 8 beats rises (0.7 → 0.9): dragging up is a fast start (negative tension).
    await drag(svg, [40, 30], [40, 10], { altKey: true });
    const curve = project().automation_points[a!.id]!.curve;
    expect(curve.type).toBe("Curve");
    expect(curve.type === "Curve" && curve.tension).toBeLessThan(0);
    await send(cmd("Edit", { type: "Undo" }));
    expect(project().automation_points[a!.id]!.curve).toEqual({ type: "Linear" });
  });

  it("a click on empty space selects the track and the lane, places the insert marker (snapped) and clears the points", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    const lane = volumeLane(track);
    const [a] = lanePoints(lane.id);
    act(() => itemSelection.getState().select("automationPoint", [a!.id], "replace"));
    const svg = screen.getByTestId("automation-lane-svg");
    await press(svg, [41, y(0.2)]);
    const at = snapToGrid(4.1, gridStep(), TempoMap.constant());
    expect(useTimeSelection.getState().selection).toEqual({ start: at, end: at, tracks: [track.id] });
    expect(useTimeSelection.getState().cursor).toBe(at);
    expect(useArrangementUi.getState().selectedTracks).toEqual(new Set([track.id]));
    expect(itemSelection.getState().selected.automationPoint.size).toBe(0);
    expect(document.activeElement).toBe(screen.getByRole("group", { name: "volume automation" }));
    // No point was added or moved.
    expect(lanePoints(lane.id)).toHaveLength(3);
  });

  it("like the arranger: a drag selects points in a box, a ⇧-drag a time range in the lane; ⇧-click extends it (#621)", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    const svg = screen.getByTestId("automation-lane-svg");
    // A plain drag over the whole height: the points in the box, no range.
    const pts = lanePoints(volumeLane(track).id);
    await drag(svg, [-5, 0], [1000, 200]);
    expect(useTimeSelection.getState().selection?.automationLane).toBeUndefined();
    expect([...itemSelection.getState().selected.automationPoint].sort()).toEqual(pts.map((p) => p.id).sort());
    await drag(svg, [21, y(0.2)], [79, y(0.2)], { shiftKey: true });
    const s = (beats: number) => snapToGrid(beats, gridStep(), TempoMap.constant());
    const tKey = `volume:${track.id}`;
    expect(useTimeSelection.getState().selection).toEqual({ start: s(2.1), end: s(7.9), tracks: [track.id], automationLane: tKey });
    expect(screen.getByTestId("automation-range")).toBeInTheDocument();
    // ⇧-click right of it: the range grows to the click.
    await press(svg, [121, y(0.2)], { shiftKey: true });
    expect(useTimeSelection.getState().selection).toMatchObject({ start: s(2.1), end: s(12.1), automationLane: tKey });
    // ⇧-click left of it: grows to the left.
    await press(svg, [11, y(0.2)], { shiftKey: true });
    expect(useTimeSelection.getState().selection).toMatchObject({ start: s(1.1), end: s(12.1), automationLane: tKey });
    // ⇧-click inside, nearer the start: the start retracts to the click.
    await press(svg, [41, y(0.2)], { shiftKey: true });
    expect(useTimeSelection.getState().selection).toMatchObject({ start: s(4.1), end: s(12.1), automationLane: tKey });
    // ⇧-click inside, nearer the end: the end retracts to the click.
    await press(svg, [101, y(0.2)], { shiftKey: true });
    expect(useTimeSelection.getState().selection).toMatchObject({ start: s(4.1), end: s(10.1), automationLane: tKey });
    // A plain click: back to the insert marker.
    await press(svg, [301, y(0.2)]);
    expect(useTimeSelection.getState().selection).toMatchObject({ start: s(30.1), end: s(30.1) });
    expect(screen.queryByTestId("automation-range")).toBeNull();
  });

  it("⌘D duplicates the lane's range right after it, keeping the curve outside, as one undo step", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    const lane = volumeLane(track);
    const before = lanePoints(lane.id).map((p) => [p.time, p.value]);
    act(() => useTimeSelection.getState().setSelection({ start: 0, end: 8, tracks: [track.id], automationLane: `volume:${track.id}` }));
    const group = screen.getByRole("group", { name: "volume automation" });
    fireEvent.keyDown(group, { key: "d", ctrlKey: true });
    await flush();
    const pts = lanePoints(lane.id);
    // 0..8 as before, then the copy over 8..16 (a jump at 8), then the old value from 16 on.
    expect(valueAt(pts, 4)).toBeCloseTo(0.8, 9);
    expect(valueBefore(pts, 8)).toBeCloseTo(0.9, 9);
    expect(valueAt(pts, 8)).toBeCloseTo(0.7, 9);
    expect(valueAt(pts, 12)).toBeCloseTo(0.8, 9);
    expect(valueBefore(pts, 16)).toBeCloseTo(0.9, 9);
    expect(valueAt(pts, 16)).toBeCloseTo(0.6, 9);
    expect(valueAt(pts, 30)).toBeCloseTo(0.6, 9);
    // The copy is the selection now.
    expect(useTimeSelection.getState().selection).toMatchObject({ start: 8, end: 16 });

    await send(cmd("Edit", { type: "Undo" }));
    expect(lanePoints(lane.id).map((p) => [p.time, p.value])).toEqual(before);
    expect(store().history.can_undo).toBe(false);
  });

  it("Delete clears the lane's range (ramp across it, outside kept); ⌘C / ⌘V paste it after the range", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    const lane = volumeLane(track);
    const tKey = `volume:${track.id}`;
    const group = screen.getByRole("group", { name: "volume automation" });
    act(() => useTimeSelection.getState().setSelection({ start: 4, end: 12, tracks: [track.id], automationLane: tKey }));
    fireEvent.keyDown(group, { key: "c", ctrlKey: true });
    fireEvent.keyDown(group, { key: "Delete" });
    await flush();
    let pts = lanePoints(lane.id);
    expect(pts.some((p) => p.time === 8)).toBe(false);
    expect(valueBefore(pts, 4)).toBeCloseTo(0.8, 9);
    expect(valueAt(pts, 8)).toBeCloseTo(0.775, 9);
    expect(valueAt(pts, 14)).toBeCloseTo(0.675, 9);
    // One step.
    await send(cmd("Edit", { type: "Undo" }));
    expect(lanePoints(lane.id)).toHaveLength(3);

    // Paste: right after the range (the pasted range becomes the selection).
    fireEvent.keyDown(group, { key: "v", ctrlKey: true });
    await flush();
    pts = lanePoints(lane.id);
    expect(valueAt(pts, 12)).toBeCloseTo(0.8, 9);
    expect(valueAt(pts, 16)).toBeCloseTo(0.9, 9);
    expect(valueBefore(pts, 20)).toBeCloseTo(0.75, 9);
    expect(useTimeSelection.getState().selection).toMatchObject({ start: 12, end: 20, automationLane: tKey });
  });

  it("right-click in the lane's range offers the range edits", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    act(() => useTimeSelection.getState().setSelection({ start: 4, end: 12, tracks: [track.id], automationLane: `volume:${track.id}` }));
    const { useContextMenuStore } = await import("@/kit/contextMenuStore");
    fireEvent.contextMenu(screen.getByTestId("automation-lane-svg"), { clientX: 60, clientY: y(0.5) });
    const labels = useContextMenuStore.getState().menu!.items.map((i) => (i === "separator" ? "-" : i.label));
    expect(labels).toEqual(["cut", "copy", "paste", "duplicate", "-", "delete"]);
  });

  it("double-click: the point exists on the second press and follows the pointer until release (one undo step)", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    pickOption(screen.getByRole("combobox", { name: "show parameter" }), { value: `pan:${track.id}` });
    await flush();
    const panSvg = screen.getAllByTestId("automation-lane-svg")[1]!;
    await press(panSvg, [40, y(0.25)]);
    await act(async () => {
      fireEvent.pointerDown(panSvg, { button: 0, clientX: 40, clientY: y(0.25) });
    });
    await flush();
    const lane = Object.values(project().automation_lanes).find((l) => l.target.type === "TrackPan")!;
    expect(lanePoints(lane.id)).toHaveLength(1);
    const id = lanePoints(lane.id)[0]!.id;
    // Created at the current pan (center, 0.5), whatever the pointer's y; the drag then
    // moves it relative to that.
    expect(lanePoints(lane.id)[0]!.value).toBe(0.5);
    for (const [x, v] of [
      [60, 0.35],
      [83, 0.55],
    ] as const) {
      await act(async () => {
        fireEvent.pointerMove(window, { clientX: x, clientY: y(v) });
      });
      await flush();
    }
    // Still held: already moved, time on the grid.
    expect(project().automation_points[id]!.time).toBe(snapToGrid(8.3, gridStep(), TempoMap.constant()));
    expect(project().automation_points[id]!.value).toBeCloseTo(0.8, 6);
    expect(screen.getByTestId("automation-drag-tip")).toBeInTheDocument();
    await act(async () => {
      fireEvent.pointerUp(window, { clientX: 83, clientY: y(0.8) });
    });
    await flush();
    expect(itemSelection.getState().selected.automationPoint).toEqual(new Set([id]));
    await send(cmd("Edit", { type: "Undo" }));
    expect(Object.values(project().automation_lanes).some((l) => l.target.type === "TrackPan")).toBe(false);
  });

  describe("adding points and moving sections keep the lane where it isn't edited", () => {
    const PAN_LANE = "01INSERTONCURVEPANLANE0000";
    const usable = LANE_HEIGHT - 2 * LANE_PAD;

    /** A pan lane on the track with `pts` ([time, value, curve]), shown. */
    async function panLane(pts: Array<[number, number, CurveShape?]>): Promise<SVGSVGElement> {
      const track = await renderLanes();
      await send(
        cmd("Automation", { type: "CreateLane", id: PAN_LANE, owner: { type: "Track", track: track.id }, target: { type: "TrackPan", track: track.id } }),
      );
      await send(
        cmd("Automation", {
          type: "AddPoints",
          lane: PAN_LANE,
          points: pts.map(([time, value, curve], i) => ({ id: `01INSERTONCURVEPOINT00000${i}`, time, value, curve: curve ?? { type: "Linear" } })),
        }),
      );
      openTrack();
      await flush();
      if (screen.getAllByTestId("automation-lane-svg").length < 2) {
        pickOption(screen.getByRole("combobox", { name: "show parameter" }), { value: `pan:${track.id}` });
        await flush();
      }
      return screen.getAllByTestId("automation-lane-svg")[1]! as unknown as SVGSVGElement;
    }

    const snapshot = () => lanePoints(PAN_LANE).map((p) => ({ id: p.id, time: p.time, value: p.value, curve: p.curve }));
    const added = (before: ReturnType<typeof snapshot>) => lanePoints(PAN_LANE).find((p) => !before.some((b) => b.id === p.id))!;
    const asCurve = (pts: ReadonlyArray<{ time: number; value: number; curve: CurveShape }>) => [...pts].sort((a, b) => a.time - b.time);

    /** The second press of a double-click, held (`release` = where it's released, after `moves`). */
    async function holdAndMove(el: Element, at: [number, number], moves: Array<[number, number]>) {
      await press(el, at);
      await act(async () => {
        fireEvent.pointerDown(el, { button: 0, clientX: at[0], clientY: at[1] });
      });
      await flush();
      for (const [x, yy] of moves) {
        await act(async () => {
          fireEvent.pointerMove(window, { clientX: x, clientY: yy });
        });
        await flush();
      }
    }
    async function release(at: [number, number]) {
      await act(async () => {
        fireEvent.pointerUp(window, { clientX: at[0], clientY: at[1] });
      });
      await flush();
    }

    it("linear: a double-click far from the line adds the point on it; the curve and the neighbours don't change; one undo step", async () => {
      const svg = await panLane([
        [0, 0.2],
        [16, 0.8],
      ]);
      const before = snapshot();
      await doublePress(svg, [80, y(0.95)]);
      const p = added(before);
      expect(p.time).toBe(8);
      expect(p.value).toBe(0.5);
      expect(p.curve).toEqual({ type: "Linear" });
      for (const b of before) expect(project().automation_points[b.id]).toMatchObject({ time: b.time, value: b.value, curve: b.curve });
      for (let t = -1; t <= 17; t += 0.25) expect(evaluatePoints(asCurve(lanePoints(PAN_LANE)), t)).toBeCloseTo(evaluatePoints(asCurve(before), t)!, 12);
      await send(cmd("Edit", { type: "Undo" }));
      expect(snapshot()).toEqual(before);
    });

    it("snapped time: the value is the lane's at the snapped time; ⌥ adds it off the grid", async () => {
      const svg = await panLane([
        [0, 0.2],
        [16, 0.8],
      ]);
      let before = snapshot();
      await doublePress(svg, [83, y(0.05)]);
      const t = snapToGrid(8.3, gridStep(), TempoMap.constant());
      expect(added(before)).toMatchObject({ time: t, value: evaluatePoints(asCurve(before), t)! });
      before = snapshot();
      await press(svg, [127, y(0.05)], { altKey: true });
      await act(async () => {
        fireEvent.pointerDown(svg, { button: 0, clientX: 127, clientY: y(0.05), altKey: true });
      });
      await release([127, y(0.05)]);
      expect(added(before)).toMatchObject({ time: 12.7, value: evaluatePoints(asCurve(before), 12.7)! });
    });

    it("curved and step segments, before the first and after the last point", async () => {
      const svg = await panLane([
        [4, 0.3, { type: "Curve", tension: 0.5 }],
        [12, 0.9, { type: "Step" }],
        [20, 0.1],
      ]);
      for (const x of [20, 80, 160, 300]) {
        const before = snapshot();
        await doublePress(svg, [x, y(0.99)]);
        const p = added(before);
        expect(p.value).toBe(evaluatePoints(asCurve(before), p.time));
        // Within a hair on the curve (its right half's tension is fitted), exact elsewhere.
        for (let t = 0; t <= 32; t += 0.125)
          expect(Math.abs(evaluatePoints(asCurve(lanePoints(PAN_LANE)), t)! - evaluatePoints(asCurve(before), t)!)).toBeLessThan(0.006);
      }
      // Held after the last point, before the first, on the step (held) and on the curve.
      const values = lanePoints(PAN_LANE).map((p) => p.value);
      expect(values[0]).toBe(0.3);
      expect(values.at(-1)).toBe(0.1);
    });

    it("a release under the 3 px threshold leaves the exact value; past it, the point moves from there (no jump to the pointer)", async () => {
      const svg = await panLane([
        [0, 0.2],
        [16, 0.8],
      ]);
      let before = snapshot();
      await holdAndMove(
        svg,
        [80, y(0.95)],
        [
          [81, y(0.95) + 1],
          [82, y(0.95) - 1],
        ],
      );
      await release([82, y(0.95) - 1]);
      expect(added(before)).toMatchObject({ time: 8, value: 0.5 });

      before = snapshot();
      await holdAndMove(
        svg,
        [40, y(0.95)],
        [
          [40, y(0.95) - 2],
          [40, y(0.95) - 10],
        ],
      );
      // Relative to its value (0.35), not to the pointer's 0.95+.
      const p = added(before);
      expect(p.time).toBe(4);
      expect(p.value).toBeCloseTo(0.35 + 10 / usable, 9);
      await release([40, y(0.95) - 10]);
      // Creation and drag: one undo step.
      await send(cmd("Edit", { type: "Undo" }));
      expect(snapshot()).toEqual(before);
    });

    it("holding after the double-click drags like a grabbed point: onto and past a neighbour's time, no value swap", async () => {
      const svg = await panLane([
        [0, 0.2],
        [8, 0.8],
        [16, 0.2],
      ]);
      const before = snapshot();
      const path: Array<[number, number]> = [
        [50, y(0.5)],
        [65, y(0.5)],
        [80, y(0.5)],
      ];
      await holdAndMove(svg, [40, y(0.5)], path);
      const p = added(before);
      // On the neighbour's exact time it stays on its side (a nudge before it), so the lane
      // still reaches 0.8 at beat 8.
      expect(p.time).toBeLessThan(8);
      expect(p.time).toBeCloseTo(8, 4);
      expect(evaluatePoints(asCurve(lanePoints(PAN_LANE)), 8)).toBe(0.8);
      await act(async () => {
        fireEvent.pointerMove(window, { clientX: 120, clientY: y(0.5) });
      });
      await release([120, y(0.5)]);
      for (const b of before) expect(project().automation_points[b.id]).toMatchObject({ time: b.time, value: b.value });
      const held = snapshot().map(({ time, value }) => ({ time, value }));

      // The same moves after releasing and grabbing the new point give the same lane.
      await send(cmd("Edit", { type: "Undo" }));
      await doublePress(svg, [40, y(0.5)]);
      const id = added(before).id;
      await act(async () => {
        fireEvent.pointerDown(document.querySelector(`circle[data-point="${id}"]`)!, { button: 0, clientX: 40, clientY: y(0.5) });
      });
      for (const [x, yy] of [...path, [120, y(0.5)] as [number, number]]) {
        await act(async () => {
          fireEvent.pointerMove(window, { clientX: x, clientY: yy });
        });
        await flush();
      }
      await release([120, y(0.5)]);
      expect(snapshot().map(({ time, value }) => ({ time, value }))).toEqual(held);
    });

    /** The lane in engine order (time, then id: two points at one time are a jump). */
    const ordered = (pts: ReadonlyArray<{ id: string; time: number; value: number; curve: CurveShape }>) =>
      [...pts].sort((a, b) => a.time - b.time || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
    const selectSection = (start: number, end: number, own = true) => {
      const track = keysTrack();
      act(() => useTimeSelection.getState().setSelection({ start, end, tracks: [track.id], ...(own ? { automationLane: `pan:${track.id}` } : {}) }));
    };
    /** A ⇧-press at `from`, pointer moves, release at the last one. */
    async function shiftDrag(el: Element, from: [number, number], moves: Array<[number, number]>) {
      await act(async () => {
        fireEvent.pointerDown(el, { button: 0, clientX: from[0], clientY: from[1], shiftKey: true });
      });
      await flush();
      for (const [x, yy] of moves) {
        await act(async () => {
          fireEvent.pointerMove(window, { clientX: x, clientY: yy, shiftKey: true });
        });
        await flush();
      }
      await release(moves.at(-1) ?? from);
    }

    it("⇧-drag on the line inside the lane's section moves only the section (edge jumps added); one undo step", async () => {
      const svg = await panLane([
        [0, 0.2],
        [16, 0.8],
      ]);
      selectSection(4, 12);
      const before = snapshot();
      // On the line at beat 8 (0.5), 10 px up.
      await shiftDrag(
        svg,
        [80, y(0.5)],
        [
          [80, y(0.5) - 2],
          [80, y(0.5) - 10],
        ],
      );
      const after = ordered(snapshot());
      const dv = 10 / usable;
      expect(after).toHaveLength(6);
      for (const b of before) expect(project().automation_points[b.id]).toMatchObject({ time: b.time, value: b.value, curve: b.curve });
      for (let t = 0; t < 4; t += 0.125) expect(Math.abs(evaluatePoints(after, t)! - evaluatePoints(before, t)!)).toBeLessThan(1e-12);
      for (let t = 12.125; t <= 20; t += 0.125) expect(Math.abs(evaluatePoints(after, t)! - evaluatePoints(before, t)!)).toBeLessThan(1e-12);
      for (let t = 4; t < 12; t += 0.125) expect(evaluatePoints(after, t)).toBeCloseTo(evaluatePoints(before, t)! + dv, 9);
      // The section stays selected.
      expect(useTimeSelection.getState().selection).toMatchObject({ start: 4, end: 12 });
      await send(cmd("Edit", { type: "Undo" }));
      expect(snapshot()).toEqual(before);
    });

    it("a ⇧-click (under the threshold) changes nothing; off the line, ⇧-drag still selects a range", async () => {
      const svg = await panLane([
        [0, 0.2],
        [16, 0.8],
      ]);
      selectSection(4, 12);
      const before = snapshot();
      await shiftDrag(svg, [80, y(0.5)], [[81, y(0.5) - 1]]);
      expect(snapshot()).toEqual(before);
      expect(useTimeSelection.getState().selection).toMatchObject({ start: 4, end: 12 });
      // Far from the line: the range gesture, values untouched.
      await shiftDrag(
        svg,
        [60, y(0.95)],
        [
          [80, y(0.95)],
          [140, y(0.95)],
        ],
      );
      expect(snapshot()).toEqual(before);
      const snap = (b: number) => snapToGrid(b, gridStep(), TempoMap.constant());
      expect(useTimeSelection.getState().selection).toMatchObject({ start: snap(6), end: snap(14) });
    });

    it("a section of the track's row works too, reusing points at its edges; the group clamps", async () => {
      const svg = await panLane([
        [0, 0.2],
        [4, 0.6],
        [8, 0.9],
        [16, 0.2],
      ]);
      selectSection(4, 12, false);
      const before = snapshot();
      // Drag far up: the inside's highest point (0.9) stops at 1, so everything inside +0.1.
      await shiftDrag(
        svg,
        [60, y(0.75)],
        [
          [60, y(0.75) - 5],
          [60, y(0.75) - 50],
        ],
      );
      const after = ordered(snapshot());
      // One point added at 4 (the existing one stays outside), two at 12.
      expect(after).toHaveLength(7);
      expect(after.filter((p) => p.time === 4)).toHaveLength(2);
      expect(after.filter((p) => p.time === 12)).toHaveLength(2);
      expect(project().automation_points[before[2]!.id]!.value).toBeCloseTo(1, 12);
      for (let t = 0; t < 4; t += 0.125) expect(evaluatePoints(after, t)).toBe(evaluatePoints(before, t));
      for (let t = 12.125; t <= 20; t += 0.125) expect(Math.abs(evaluatePoints(after, t)! - evaluatePoints(before, t)!)).toBeLessThan(1e-12);
      for (let t = 4; t < 12; t += 0.125) expect(evaluatePoints(after, t)).toBeCloseTo(evaluatePoints(before, t)! + 0.1, 9);
    });
  });

  it("toggles and deletes the lane", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    const lane = volumeLane(track);
    fireEvent.click(screen.getByRole("button", { name: "lane enabled" }));
    await flush();
    expect(project().automation_lanes[lane.id]!.enabled).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "delete lane" }));
    await flush();
    expect(project().automation_lanes[lane.id]).toBeUndefined();
    // The row stays (shown parameter, no lane yet).
    expect(screen.getByTestId("automation-lane-svg")).toBeInTheDocument();
  });
});

describe("AutomationLanes (detail view)", () => {
  it("edits the selected track's lanes", async () => {
    mock = new MockTransport({ timers: "manual", seed: 1 });
    render(
      <TransportProvider transport={mock}>
        <AutomationLanes />
      </TransportProvider>,
    );
    await waitFor(() => expect(store().project).not.toBeNull());
    expect(screen.getByText(/select a track/)).toBeInTheDocument();
    await act(async () => useSelectionStore.getState().selectTrack(keysTrack().id));
    expect(screen.getByRole("button", { name: /show automation of Keys/ })).toBeInTheDocument();
    expect(screen.getByTestId("playhead-line")).toBeInTheDocument();
  });
});
