/**
 * Automation lanes drawn by the WebGL arranger (`gl`): the SVG keeps the gestures but has no
 * element per point; the lane publishes what it shows (`arrangement/gl/layers`).
 */
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { AutomationLane, AutomationPoint, Track } from "@/generated";
import { useProjectStore } from "@/state";
import { createTimelineViewStore, itemSelection, type TimelineViewStore } from "@/timeline";
import { MockTransport, TransportProvider } from "@/transport";
import { automationLanesOf } from "@/features/arrangement/gl/layers/automationLanes";
import { AutomationToggleButton } from "./AutomationToggleButton";
import { LANE_PAD } from "./geometry";
import { TrackAutomationLanes } from "./index";
import { LANE_HEIGHT, resetAutomationUi } from "./uiStore";

const store = () => useProjectStore.getState();
const project = () => store().project!;
let mock: MockTransport | undefined;
let view: TimelineViewStore;
const PX_PER_BEAT = 10;
// jsdom has no layout: client coords are lane-local px.
const y = (v: number) => LANE_PAD + (1 - v) * (LANE_HEIGHT - 2 * LANE_PAD);

const keys = (): Track => Object.values(project().tracks).find((t) => t.name === "Keys")!;
const volumeLane = (t: Track): AutomationLane =>
  Object.values(project().automation_lanes).find((l) => l.target.type === "TrackVolume" && l.owner.type === "Track" && l.owner.track === t.id)!;
const points = (lane: string): AutomationPoint[] =>
  Object.values(project().automation_points)
    .filter((p) => p.lane === lane)
    .sort((a, b) => a.time - b.time);

async function flush(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

async function renderLanes(): Promise<{ track: Track; unmount: () => void }> {
  mock = new MockTransport({ timers: "manual", seed: 1 });
  let t: Track | undefined;
  const Probe = () => {
    const p = useProjectStore((s) => s.project);
    if (!p) return null;
    t ??= keys();
    return (
      <>
        <AutomationToggleButton trackId={t.id} trackName={t.name} />
        <TrackAutomationLanes trackId={t.id} view={view} gl />
      </>
    );
  };
  const r = render(
    <TransportProvider transport={mock}>
      <Probe />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  await flush();
  fireEvent.click(screen.getByRole("button", { name: /show automation of/ }));
  await flush();
  return { track: t!, unmount: r.unmount };
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
});

describe("automation lanes on the WebGL arranger", () => {
  it("publish their points for the canvas and draw no SVG elements", async () => {
    const { track, unmount } = await renderLanes();
    const svg = screen.getAllByTestId("automation-lane-svg")[0]!;
    expect(svg.dataset.renderer).toBe("webgl");
    expect(svg.querySelectorAll("circle, path, line")).toHaveLength(0);
    const lane = volumeLane(track);
    const published = automationLanesOf(track.id);
    expect(published).toHaveLength(1);
    expect(published[0]!.points.map((p) => p.id)).toEqual(points(lane.id).map((p) => p.id));
    // Selection is republished.
    const [, b] = points(lane.id);
    act(() => itemSelection.getState().select("automationPoint", [b!.id], "replace"));
    expect(automationLanesOf(track.id)[0]!.selected.has(b!.id)).toBe(true);
    unmount();
    expect(automationLanesOf(track.id)).toHaveLength(0);
  });

  it("select and drag points through the SVG's hit test (no circles)", async () => {
    const { track } = await renderLanes();
    const lane = volumeLane(track);
    const [, b] = points(lane.id);
    const svg = screen.getAllByTestId("automation-lane-svg")[0]!;
    const at: [number, number] = [b!.time * PX_PER_BEAT, y(b!.value)];
    await act(async () => {
      fireEvent.pointerDown(svg, { button: 0, clientX: at[0] + 2, clientY: at[1] - 1 });
    });
    expect(itemSelection.getState().isSelected("automationPoint", b!.id)).toBe(true);
    for (let i = 1; i <= 4; i++) {
      await act(async () => {
        fireEvent.pointerMove(window, { clientX: at[0] + 2 + 5 * i, clientY: at[1] - 1 });
      });
      await flush();
    }
    await act(async () => {
      fireEvent.pointerUp(window, { clientX: at[0] + 22, clientY: at[1] - 1 });
    });
    await flush();
    expect(points(lane.id).find((p) => p.id === b!.id)!.time).toBeGreaterThan(b!.time);
  });

  it("double-click on a point deletes it, on empty space adds one", async () => {
    const { track } = await renderLanes();
    const lane = volumeLane(track);
    const [a] = points(lane.id);
    const svg = screen.getAllByTestId("automation-lane-svg")[0]!;
    // Double-clicks are seen on the second press (pointerdown), not on `dblclick`.
    const doublePress = async (x: number, yy: number) => {
      for (let i = 0; i < 2; i++) {
        await act(async () => {
          fireEvent.pointerDown(svg, { button: 0, clientX: x, clientY: yy });
        });
        await flush();
        await act(async () => {
          fireEvent.pointerUp(window, { clientX: x, clientY: yy });
        });
        await flush();
      }
    };
    await doublePress(a!.time * PX_PER_BEAT, y(a!.value));
    expect(points(lane.id).some((p) => p.id === a!.id)).toBe(false);
    await doublePress(300, y(0.2));
    expect(points(lane.id)).toHaveLength(3);
  });
});
