import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { AutomationLane, AutomationPoint, Track } from "@/generated";
import { playheadStore, useProjectStore, useSelectionStore } from "@/state";
import { createTimelineViewStore, itemSelection, type TimelineViewStore } from "@/timeline";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { paramToPlain } from "@/features/devices/paramScale";
import { pickOption } from "@/kit/testing";
import { AutomationToggleButton } from "./AutomationToggleButton";
import { clearPointClipboard, pointClipboard } from "./clipboard";
import { LANE_PAD } from "./geometry";
import { TrackAutomationLanes } from "./index";
import { VOLUME_INFO } from "./params";
import { AUTOMATION_BAR_HEIGHT, automationHeight, LANE_HEIGHT, laneUiKey, resetAutomationUi, useAutomationUi } from "./uiStore";

const store = () => useProjectStore.getState();
const project = () => store().project!;

let mock: MockTransport | undefined;
let view: TimelineViewStore;

// jsdom has no layout: element rects are all zero, so client coords are lane-local px.
const PX_PER_BEAT = 10;
const y = (v: number, h = LANE_HEIGHT) => LANE_PAD + (1 - v) * (h - 2 * LANE_PAD);

function keysTrack(): Track {
  return Object.values(project().tracks).find((t) => t.name === "Keys")!;
}

function laneOf(track: Track, type: string): AutomationLane | undefined {
  return Object.values(project().automation_lanes).find((l) => l.target.type === type && l.owner.type === "Track" && l.owner.track === track.id);
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

async function renderLanes(): Promise<Track> {
  mock = new MockTransport({ timers: "manual", seed: 1 });
  let t: Track | undefined;
  const Probe = () => {
    const p = useProjectStore((s) => s.project);
    if (!p) return null;
    t ??= keysTrack();
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
  fireEvent.click(screen.getByRole("button", { name: /Show automation of/ }));
  await flush();
  return t!;
}

async function send(command: Parameters<MockTransport["send"]>[0]) {
  await act(async () => {
    await mock!.send(command);
  });
}

async function key(el: Element, k: string, opts: { ctrlKey?: boolean; shiftKey?: boolean } = {}) {
  await act(async () => {
    fireEvent.keyDown(el, { key: k, ...opts });
  });
  await flush();
}

function setPlayhead(position: number) {
  act(() => playheadStore.setPlayhead({ transport: { position, seconds: 0, playing: false, bpm: 120 } }));
}

async function drag(el: Element, from: [number, number], to: [number, number], opts: { altKey?: boolean; release?: boolean } = {}) {
  const { release = true, ...mods } = opts;
  await act(async () => {
    fireEvent.pointerDown(el, { button: 0, clientX: from[0], clientY: from[1], ...mods });
  });
  for (let i = 1; i <= 4; i++) {
    await act(async () => {
      fireEvent.pointerMove(window, { clientX: from[0] + ((to[0] - from[0]) * i) / 4, clientY: from[1] + ((to[1] - from[1]) * i) / 4, ...mods });
    });
    await flush();
  }
  if (!release) return;
  await act(async () => {
    fireEvent.pointerUp(window, { clientX: to[0], clientY: to[1], ...mods });
  });
  await flush();
}

const lanes = () => screen.getAllByRole("group", { name: /automation$/ });

beforeEach(() => {
  resetAutomationUi();
  clearPointClipboard();
  view = createTimelineViewStore({ pxPerBeat: PX_PER_BEAT });
  view.getState().setWidth(400);
});

afterEach(() => {
  mock?.dispose();
  mock = undefined;
  store().reset();
  itemSelection.getState().clear();
  useSelectionStore.getState().selectTrack(null);
  setPlayhead(0);
});

describe("copy / cut / paste / duplicate", () => {
  it("copies and pastes at the playhead as one undo step, selecting the paste", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const [, b, c] = lanePoints(lane.id);
    act(() => itemSelection.getState().select("automationPoint", [b!.id, c!.id], "replace"));
    await key(lanes()[0]!, "c", { ctrlKey: true });
    expect(pointClipboard()?.points).toHaveLength(2);

    setPlayhead(32);
    await key(lanes()[0]!, "v", { ctrlKey: true });
    const pts = lanePoints(lane.id);
    expect(pts.map((p) => [p.time, p.value])).toEqual([
      [0, 0.7],
      [8, 0.9],
      [16, 0.6],
      [32, 0.9],
      [40, 0.6],
    ]);
    const pasted = pts.slice(3).map((p) => p.id);
    expect([...itemSelection.getState().selected.automationPoint].sort()).toEqual(pasted.sort());

    await send(cmd("Edit", { type: "Undo" }));
    expect(lanePoints(lane.id)).toHaveLength(3);
  });

  it("paste replaces the points in the pasted span", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const [a, b] = lanePoints(lane.id);
    act(() => itemSelection.getState().select("automationPoint", [a!.id, b!.id], "replace"));
    await key(lanes()[0]!, "c", { ctrlKey: true });
    setPlayhead(8);
    await key(lanes()[0]!, "v", { ctrlKey: true });
    // [8, 16] now holds the copy (0.7 → 0.9); the old points at 8 and 16 are gone.
    expect(lanePoints(lane.id).map((p) => [p.time, p.value])).toEqual([
      [0, 0.7],
      [8, 0.7],
      [16, 0.9],
    ]);
  });

  it("cut removes the points; paste into another parameter remaps and creates its lane in one step", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    act(() => itemSelection.getState().select("automationPoint", lanePoints(lane.id).map((p) => p.id), "replace"));
    await key(lanes()[0]!, "x", { ctrlKey: true });
    expect(lanePoints(lane.id)).toHaveLength(0);

    pickOption(screen.getByRole("combobox", { name: "Show parameter" }), { value: `pan:${track.id}` });
    await flush();
    setPlayhead(4);
    await key(lanes()[1]!, "v", { ctrlKey: true });
    const pan = laneOf(track, "TrackPan")!;
    // Volume (dB) → pan: no shared unit, so by normalized value.
    expect(lanePoints(pan.id).map((p) => [p.time, p.value])).toEqual([
      [4, 0.7],
      [12, 0.9],
      [20, 0.6],
    ]);
    await send(cmd("Edit", { type: "Undo" }));
    expect(laneOf(track, "TrackPan")).toBeUndefined();
  });

  it("duplicates the selection right after itself", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const [, b, c] = lanePoints(lane.id);
    act(() => itemSelection.getState().select("automationPoint", [b!.id, c!.id], "replace"));
    await key(lanes()[0]!, "d", { ctrlKey: true });
    expect(lanePoints(lane.id).map((p) => [p.time, p.value])).toEqual([
      [0, 0.7],
      [8, 0.9],
      [16, 0.6],
      [16, 0.9],
      [24, 0.6],
    ]);
    await send(cmd("Edit", { type: "Undo" }));
    expect(lanePoints(lane.id)).toHaveLength(3);
  });

  it("offers the actions in the point menu", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const [, b] = lanePoints(lane.id);
    const circle = document.querySelector(`circle[data-point="${b!.id}"]`)!;
    const { useContextMenuStore } = await import("@/kit/contextMenuStore");
    fireEvent.contextMenu(circle);
    const labels = useContextMenuStore.getState().menu!.items.map((i) => (i === "separator" ? "-" : i.label));
    expect(labels).toEqual(expect.arrayContaining(["Cut", "Copy", "Paste at Playhead", "Duplicate", "Set Value…", "Delete Point"]));
  });
});

describe("value snapping", () => {
  it("cmd/ctrl held mid-drag snaps continuous values to whole increments (1 dB); alt only frees time", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const [, b] = lanePoints(lane.id);
    const circle = document.querySelector(`circle[data-point="${b!.id}"]`)!;
    const dB = () => paramToPlain(VOLUME_INFO, project().automation_points[b!.id]!.value);
    const press = async (x: number, yy: number) =>
      act(async () => {
        fireEvent.pointerDown(circle, { button: 0, clientX: x, clientY: yy });
      });
    const move = async (x: number, yy: number, mods: { ctrlKey?: boolean; altKey?: boolean }) => {
      await act(async () => {
        fireEvent.pointerMove(window, { clientX: x, clientY: yy, ...mods });
      });
      await flush();
    };
    const release = async () => {
      await act(async () => {
        fireEvent.pointerUp(window, {});
      });
      await flush();
    };

    // ⌘/Ctrl held after the press: whole dB, with the hint in the tooltip.
    await press(80, y(0.9));
    await move(80, y(0.87), { ctrlKey: true });
    await move(80, y(0.83), { ctrlKey: true });
    const tip = screen.getByTestId("automation-drag-tip");
    expect(tip.textContent).toMatch(/dB/);
    expect(tip.textContent).toContain("1 dB steps");
    expect(tip.textContent).toContain("⌥ off grid");
    await release();
    expect(dB()).toBeCloseTo(Math.round(dB()), 6);
    expect(screen.queryByTestId("automation-drag-tip")).toBeNull();

    // ⌥ alone: values stay free (not whole dB).
    await press(80, y(0.83));
    await move(80, y(0.8), { altKey: true });
    await move(80, y(0.7713), { altKey: true });
    await release();
    expect(Math.abs(dB() - Math.round(dB()))).toBeGreaterThan(1e-3);
  });

  it("stepped params (semitones) snap to whole steps and draw step lines", async () => {
    const track = await renderLanes();
    const synth = Object.values(project().devices).find((d) => d.track === track.id && d.name === "Synth")!;
    const combo = screen.getByRole("combobox", { name: "Show parameter" });
    await waitFor(() => expect(combo.querySelector(`option[value="param:${synth.id}:1"]`) ?? combo).toBeTruthy());
    await waitFor(() => {
      pickOption(screen.getByRole("combobox", { name: "Show parameter" }), { value: `param:${synth.id}:1` });
      expect(lanes()).toHaveLength(2);
    });
    await flush();
    const svg = screen.getAllByTestId("automation-lane-svg")[1]!;
    // Detune: -12..12 st. The default 64 px lane opens on ±3 st (9 px per step), one line
    // per step.
    expect(svg.querySelectorAll(".eth-auto-lane__step")).toHaveLength(7);
    const st = (s: number) => LANE_PAD + (1 - (s + 3) / 6) * (LANE_HEIGHT - 2 * LANE_PAD);
    // Double-click a bit above +1 st.
    fireEvent.doubleClick(svg, { clientX: 40, clientY: st(1.3) });
    await flush();
    const detune = Object.values(project().automation_lanes).find((l) => l.target.type === "DeviceParam")!;
    const [p] = lanePoints(detune.id);
    const semis = () => project().automation_points[p!.id]!.value * 24 - 12;
    expect(semis()).toBeCloseTo(1, 9);

    // A coarse 24 px drag up moves exactly 3 st (8 px per step, whatever the lane height).
    const circle = document.querySelector(`circle[data-point="${p!.id}"]`)!;
    await drag(circle, [40, st(1)], [41, st(1) - 24]);
    expect(semis()).toBeCloseTo(4, 9);

    // Nudge up one semitone with the arrow key.
    act(() => itemSelection.getState().select("automationPoint", [p!.id], "replace"));
    await key(lanes()[1]!, "ArrowUp");
    expect(semis()).toBeCloseTo(5, 9);
  });
});

describe("lane resizing", () => {
  it("drags the lane's bottom edge in steps, feeds the layout height, and resets on double-click", async () => {
    const track = await renderLanes();
    const grip = screen.getByRole("separator", { name: /Resize Volume lane/ });
    await act(async () => {
      fireEvent.pointerDown(grip, { button: 0, clientY: 100 });
    });
    await act(async () => {
      fireEvent.pointerMove(window, { clientY: 141 });
    });
    await act(async () => {
      fireEvent.pointerUp(window, { clientY: 141 });
    });
    const key = laneUiKey(track.id, `volume:${track.id}`);
    expect(useAutomationUi.getState().laneHeights[key]).toBe(104);
    expect(automationHeight(useAutomationUi.getState(), track.id)).toBe(AUTOMATION_BAR_HEIGHT + 104);
    expect(document.querySelector<HTMLElement>(".eth-auto-row")!.style.height).toBe("104px");
    expect(document.querySelector(".eth-auto-lane__svg")!.getAttribute("height")).toBe("104");

    // Survives closing and reopening (UI state, like track heights).
    fireEvent.click(screen.getByRole("button", { name: /automation of/ }));
    fireEvent.click(screen.getByRole("button", { name: /automation of/ }));
    await flush();
    expect(document.querySelector<HTMLElement>(".eth-auto-row")!.style.height).toBe("104px");

    fireEvent.doubleClick(screen.getByRole("separator", { name: /Resize Volume lane/ }));
    expect(useAutomationUi.getState().laneHeights[key]).toBeUndefined();
    expect(automationHeight(useAutomationUi.getState(), track.id)).toBe(AUTOMATION_BAR_HEIGHT + LANE_HEIGHT);
  });
});
