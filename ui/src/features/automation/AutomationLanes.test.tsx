import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { AutomationLane, AutomationPoint, Track } from "@/generated";
import { useProjectStore, useSelectionStore } from "@/state";
import { createTimelineViewStore, DEFAULT_GRID, itemSelection, resolveGrid, snapToGrid, TempoMap, type TimelineViewStore } from "@/timeline";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { AutomationLanes, TrackAutomationLanes } from "./index";
import { LANE_PAD } from "./geometry";
import { AUTOMATION_BAR_HEIGHT, LANE_HEIGHT, resetAutomationUi, useAutomationUi } from "./uiStore";
import { pickOption } from "@/kit/testing";

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
    return <TrackAutomationLanes trackId={t.id} view={view} />;
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

async function drag(el: Element, from: [number, number], to: [number, number], opts: { altKey?: boolean; shiftKey?: boolean } = {}) {
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
  fireEvent.click(screen.getByRole("button", { name: /Show automation of/ }));
}

beforeEach(() => {
  resetAutomationUi();
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
  it("is a collapsed bar until opened, then shows the existing lanes", async () => {
    const track = await renderLanes();
    expect(document.querySelector(".eth-auto-track")).toHaveStyle({ height: `${AUTOMATION_BAR_HEIGHT}px` });
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
    expect(second.querySelector("title")?.textContent).toMatch(/dB$/);
  });

  it("shows and hides lanes per parameter", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    pickOption(screen.getByRole("combobox", { name: "Show parameter" }), { value: `pan:${track.id}` });
    await flush();
    expect(screen.getAllByTestId("automation-lane-svg")).toHaveLength(2);
    fireEvent.click(screen.getAllByRole("button", { name: "Hide lane" })[0]!);
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
    expect(project().automation_points[mid!.id]!.time).toBeCloseTo(8.3, 6);
  });

  it("adds a point on double-click, creating the lane in the same undo step", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    pickOption(screen.getByRole("combobox", { name: "Show parameter" }), { value: `pan:${track.id}` });
    await flush();
    const panSvg = screen.getAllByTestId("automation-lane-svg")[1]!;
    fireEvent.doubleClick(panSvg, { clientX: 40, clientY: y(0.25) });
    await flush();
    const lane = Object.values(project().automation_lanes).find((l) => l.target.type === "TrackPan")!;
    expect(lane).toBeDefined();
    const pts = lanePoints(lane.id);
    expect(pts).toHaveLength(1);
    expect(pts[0]).toMatchObject({ time: 4, curve: { type: "Linear" } });
    expect(pts[0]!.value).toBeCloseTo(0.25, 6);
    expect(itemSelection.getState().selected.automationPoint.has(pts[0]!.id)).toBe(true);

    await send(cmd("Edit", { type: "Undo" }));
    expect(Object.values(project().automation_lanes).some((l) => l.target.type === "TrackPan")).toBe(false);
  });

  it("marquee-selects points, deletes them with Delete, and removes one with double-click", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    const lane = volumeLane(track);
    const svg = screen.getByTestId("automation-lane-svg");
    await drag(svg, [-5, 0], [90, LANE_HEIGHT]);
    const [a, b, c] = lanePoints(lane.id);
    const sel = itemSelection.getState().selected.automationPoint;
    expect(sel.has(a!.id) && sel.has(b!.id) && !sel.has(c!.id)).toBe(true);

    const group = screen.getByRole("group", { name: "Volume automation" });
    fireEvent.keyDown(group, { key: "Delete" });
    await flush();
    expect(lanePoints(lane.id).map((p) => p.id)).toEqual([c!.id]);

    fireEvent.doubleClick(document.querySelector(`circle[data-point="${c!.id}"]`)!);
    await flush();
    expect(lanePoints(lane.id)).toHaveLength(0);
  });

  it("sets the curve type of the selected points", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    const lane = volumeLane(track);
    const [a] = lanePoints(lane.id);
    const select = screen.getByRole("combobox", { name: "Curve of selected points" });
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

  it("toggles and deletes the lane", async () => {
    const track = await renderLanes();
    openTrack();
    await flush();
    const lane = volumeLane(track);
    fireEvent.click(screen.getByRole("button", { name: "Lane enabled" }));
    await flush();
    expect(project().automation_lanes[lane.id]!.enabled).toBe(false);
    fireEvent.click(screen.getByRole("button", { name: "Delete lane" }));
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
    expect(screen.getByText(/Select a track/)).toBeInTheDocument();
    await act(async () => useSelectionStore.getState().selectTrack(keysTrack().id));
    expect(screen.getByRole("button", { name: /Show automation of Keys/ })).toBeInTheDocument();
    expect(screen.getByTestId("playhead-line")).toBeInTheDocument();
  });
});
