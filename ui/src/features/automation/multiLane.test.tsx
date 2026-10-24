// Selections across lanes and tracks (laneRegistry.ts, laneMarquee.ts): a box started in one
// lane selects the points of the others it crosses; dragging, nudging, Delete, copy / paste
// and duplicate then act on every lane holding selected points, as one undo step.
import { act, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AutomationPoint, Track } from "@/generated";
import { useProjectStore, useSelectionStore } from "@/state";
import { DEFAULT_GRID, itemSelection, resolveGrid, stepLength, TempoMap, type TimelineViewStore } from "@/timeline";
import { cmd, newId, TransportProvider } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { TrackAutomationLanes } from "./index";
import { LANE_PAD } from "./geometry";
import { targetKey } from "./params";
import { LANE_HEIGHT, resetAutomationUi, useAutomationUi } from "./uiStore";
import { clearPointClipboard } from "./clipboard";
import { resetTimeSelection } from "@/features/time-edits/store";
import { arrangementView, resetArrangementUi, useArrangementUi } from "@/features/arrangement/uiStore";
import { clearClipboard, pasteClips } from "@/features/arrangement/clipboard";
import { runClipAction } from "@/features/arrangement/actions";
import { isArrangementClip } from "@/features/arrangement/clipTime";

const PX_PER_BEAT = 10;
/** Client top of each track's lane (jsdom has no layout: the test lays the two lanes out). */
const TOP = { Keys: 0, Bass: 100 } as const;
const y = (v: number) => LANE_PAD + (1 - v) * (LANE_HEIGHT - 2 * LANE_PAD);

const project = () => useProjectStore.getState().project!;
const trackNamed = (name: string): Track => Object.values(project().tracks).find((t) => t.name === name)!;
const volumePoints = (track: Track): AutomationPoint[] => {
  const lane = Object.values(project().automation_lanes).find(
    (l) => l.target.type === "TrackVolume" && l.owner.type === "Track" && l.owner.track === track.id,
  )!;
  return Object.values(project().automation_points)
    .filter((p) => p.lane === lane.id)
    .sort((a, b) => a.time - b.time);
};
const selected = () => itemSelection.getState().selected.automationPoint;

let mock: MockTransport | undefined;
let view: TimelineViewStore;

async function flush(): Promise<void> {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

async function send(command: Parameters<MockTransport["send"]>[0]) {
  await act(async () => {
    await mock!.send(command);
  });
}

/** Keys (demo volume lane: 0.7 @ 0, 0.9 @ 8, 0.6 @ 16) over Bass (volume: 0.2 @ 2, 0.4 @ 6). */
async function renderTwoTracks(): Promise<{ keys: Track; bass: Track }> {
  mock = new MockTransport({ timers: "manual", seed: 1 });
  const Lanes = () => {
    const p = useProjectStore((s) => s.project);
    if (!p) return null;
    return (
      <>
        <TrackAutomationLanes trackId={trackNamed("Keys").id} view={view} />
        <TrackAutomationLanes trackId={trackNamed("Bass").id} view={view} />
      </>
    );
  };
  render(
    <TransportProvider transport={mock}>
      <Lanes />
    </TransportProvider>,
  );
  await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
  const keys = trackNamed("Keys");
  const bass = trackNamed("Bass");
  const lane = newId();
  await send(cmd("Automation", { type: "CreateLane", id: lane, owner: { type: "Track", track: bass.id }, target: { type: "TrackVolume", track: bass.id } }));
  await send(
    cmd("Automation", {
      type: "AddPoints",
      lane,
      points: [
        { id: newId(), time: 2, value: 0.2, curve: { type: "Linear" } },
        { id: newId(), time: 6, value: 0.4, curve: { type: "Linear" } },
      ],
    }),
  );
  for (const t of [keys, bass]) {
    act(() => useAutomationUi.getState().show(t.id, targetKey({ type: "TrackVolume", track: t.id })));
  }
  await flush();
  return { keys, bass };
}

/** The lane roots laid out one under the other (client px), by the track they belong to. */
function layOut(keys: Track, bass: Track) {
  const tops = new Map<string, number>([
    [keys.id, TOP.Keys],
    [bass.id, TOP.Bass],
  ]);
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    const lane = this.closest(".eth-auto-lane");
    const track = this.closest("[data-automation-track]")?.getAttribute("data-automation-track");
    const top = lane && track ? tops.get(track) : undefined;
    if (top === undefined) return DOMRect.fromRect({ x: 0, y: 0, width: 0, height: 0 });
    return DOMRect.fromRect({ x: 0, y: top, width: 400, height: LANE_HEIGHT });
  });
}

const laneSvg = (track: Track) => document.querySelector(`[data-automation-track="${track.id}"] [data-testid="automation-lane-svg"]`)!;
const laneGroup = (track: Track) => document.querySelector(`[data-automation-track="${track.id}"] .eth-auto-lane`)!;

async function drag(el: Element, from: [number, number], to: [number, number], opts: { ctrlKey?: boolean } = {}) {
  await act(async () => {
    fireEvent.pointerDown(el, { button: 0, clientX: from[0], clientY: from[1], ...opts });
  });
  for (let i = 1; i <= 4; i++) {
    await act(async () => {
      fireEvent.pointerMove(window, { clientX: from[0] + ((to[0] - from[0]) * i) / 4, clientY: from[1] + ((to[1] - from[1]) * i) / 4, ...opts });
    });
    await flush();
  }
  await act(async () => {
    fireEvent.pointerUp(window, { clientX: to[0], clientY: to[1], ...opts });
  });
  await flush();
}

/** Box from Keys' lane down into Bass': Keys' first point and both of Bass'. */
async function boxAcross(keys: Track) {
  await drag(laneSvg(keys), [-5, 2], [65, TOP.Bass + LANE_HEIGHT - 2]);
}

beforeEach(() => {
  resetAutomationUi();
  resetTimeSelection();
  resetArrangementUi();
  clearPointClipboard();
  clearClipboard();
  // The arrangement's view: clips selected with the points act on its lanes (`mixedSelection.ts`).
  view = arrangementView;
  view.getState().zoomTo(PX_PER_BEAT, 0);
  view.getState().setWidth(400);
});

afterEach(() => {
  vi.restoreAllMocks();
  mock?.dispose();
  mock = undefined;
  useProjectStore.getState().reset();
  itemSelection.getState().clear();
  useSelectionStore.getState().selectTrack(null);
});

describe("selection across automation lanes", () => {
  it("a box started in one lane selects the points of every lane it crosses", async () => {
    const { keys, bass } = await renderTwoTracks();
    layOut(keys, bass);
    await boxAcross(keys);
    const [k0, k1] = volumePoints(keys);
    const [b0, b1] = volumePoints(bass);
    expect(selected().has(k0!.id) && selected().has(b0!.id) && selected().has(b1!.id)).toBe(true);
    expect(selected().has(k1!.id)).toBe(false);
    // The box is drawn over the page while dragging, and gone after.
    expect(document.querySelector(".eth-auto-marquee")).toBeNull();
  });

  it("Delete in one lane deletes the selected points of all of them, as one undo step", async () => {
    const { keys, bass } = await renderTwoTracks();
    layOut(keys, bass);
    await boxAcross(keys);
    fireEvent.keyDown(laneGroup(keys), { key: "Delete" });
    await flush();
    expect(volumePoints(keys).map((p) => p.time)).toEqual([8, 16]);
    expect(volumePoints(bass)).toHaveLength(0);
    await send(cmd("Edit", { type: "Undo" }));
    expect(volumePoints(keys)).toHaveLength(3);
    expect(volumePoints(bass)).toHaveLength(2);
  });

  it("dragging one selected point moves the others in time with it, each lane on its own scale", async () => {
    const { keys, bass } = await renderTwoTracks();
    layOut(keys, bass);
    await boxAcross(keys);
    const step = stepLength(resolveGrid(DEFAULT_GRID, PX_PER_BEAT, TempoMap.constant().signatureAt(0))!, TempoMap.constant().signatureAt(0));
    // Grab Keys' first point (on the grid, at x 0) and drag it one grid step right, 10 px up.
    const circle = document.querySelector(`circle[data-point="${volumePoints(keys)[0]!.id}"]`)!;
    await drag(circle, [0, TOP.Keys + y(0.7)], [step * PX_PER_BEAT, TOP.Keys + y(0.7) - 10]);
    expect(volumePoints(keys).map((p) => p.time)).toEqual([step, 8, 16]);
    expect(volumePoints(bass).map((p) => p.time)).toEqual([2 + step, 6 + step]);
    const lift = 10 / (LANE_HEIGHT - 2 * LANE_PAD);
    expect(volumePoints(keys)[0]!.value).toBeCloseTo(0.7 + lift, 6);
    expect(volumePoints(bass).map((p) => p.value)).toEqual([expect.closeTo(0.2 + lift, 6), expect.closeTo(0.4 + lift, 6)]);
    // One undo step.
    await send(cmd("Edit", { type: "Undo" }));
    expect(volumePoints(bass).map((p) => p.time)).toEqual([2, 6]);
    expect(volumePoints(keys)[0]!.time).toBe(0);
  });

  it("arrow keys nudge every selected point", async () => {
    const { keys, bass } = await renderTwoTracks();
    layOut(keys, bass);
    await boxAcross(keys);
    fireEvent.keyDown(laneGroup(keys), { key: "ArrowUp" });
    await flush();
    expect(volumePoints(keys)[0]!.value).toBeGreaterThan(0.7);
    expect(volumePoints(bass)[0]!.value).toBeGreaterThan(0.2);
    expect(volumePoints(keys)[1]!.value).toBe(0.9);
  });

  it("copy then paste puts each lane's points back into its own lane, in step", async () => {
    const { keys, bass } = await renderTwoTracks();
    layOut(keys, bass);
    await boxAcross(keys);
    fireEvent.keyDown(laneGroup(keys), { key: "c", ctrlKey: true });
    // Duplicate: right after the selection (Keys' 0 .. Bass' 6), so at 6.
    fireEvent.keyDown(laneGroup(bass), { key: "d", ctrlKey: true });
    await flush();
    expect(volumePoints(keys).map((p) => p.time)).toEqual([0, 6, 8, 16]);
    expect(volumePoints(bass).map((p) => p.time)).toEqual([2, 6, 8, 12]);
    // The copies are the selection now.
    expect(selected().size).toBe(3);
  });

  it("a plain click on a lane clears the point selection of every lane", async () => {
    const { keys, bass } = await renderTwoTracks();
    layOut(keys, bass);
    await boxAcross(keys);
    expect(selected().size).toBe(3);
    await drag(laneSvg(bass), [300, TOP.Bass + 10], [300, TOP.Bass + 10]);
    expect(selected().size).toBe(0);
  });
});

describe("selection across clips and automation lanes", () => {
  /** One of Keys' clips selected with the box's points (Keys' first, both of Bass'). */
  async function mixed() {
    const tracks = await renderTwoTracks();
    layOut(tracks.keys, tracks.bass);
    await boxAcross(tracks.keys);
    const clip = Object.values(project().clips)
      .filter((c) => isArrangementClip(c) && c.track === tracks.keys.id)
      .sort((a, b) => a.start - b.start)[0]!;
    act(() => itemSelection.getState().selectKinds({ clip: [clip.id], automationPoint: selected() }));
    return { ...tracks, clip };
  }

  it("Delete in a lane deletes the clips and the points, as one undo step", async () => {
    const { keys, bass, clip } = await mixed();
    fireEvent.keyDown(laneGroup(keys), { key: "Delete" });
    await flush();
    expect(project().clips[clip.id]).toBeUndefined();
    expect(volumePoints(keys).map((p) => p.time)).toEqual([8, 16]);
    expect(volumePoints(bass)).toHaveLength(0);
    await send(cmd("Edit", { type: "Undo" }));
    expect(project().clips[clip.id]).toBeDefined();
    expect(volumePoints(keys)).toHaveLength(3);
    expect(volumePoints(bass)).toHaveLength(2);
  });

  it("copy then paste puts back the clips and the points, in step, as one undo step", async () => {
    const { keys, bass, clip } = await mixed();
    await act(() => runClipAction(mock!, "copy"));
    const before = Object.keys(project().clips).length;
    // The earliest copied item (Keys' point at 0, or the clip) lands at 32.
    const origin = Math.min(clip.start, 0);
    await act(async () => {
      await pasteClips(mock!, 32);
    });
    await flush();
    expect(Object.keys(project().clips)).toHaveLength(before + 1);
    const copy = Object.values(project().clips).find((c) => c.track === keys.id && c.start === 32 + clip.start - origin);
    expect(copy).toBeDefined();
    expect(volumePoints(keys).map((p) => p.time)).toEqual([0, 8, 16, 32 - origin]);
    expect(volumePoints(bass).map((p) => p.time)).toEqual([2, 6, 34 - origin, 38 - origin]);
    // Both copies are the selection.
    expect(itemSelection.getState().selected.clip.has(copy!.id)).toBe(true);
    expect(selected().size).toBe(3);
    await send(cmd("Edit", { type: "Undo" }));
    expect(Object.keys(project().clips)).toHaveLength(before);
    expect(volumePoints(bass)).toHaveLength(2);
  });

  it("dragging a selected point moves the selected clips with it, as one undo step", async () => {
    const { keys, bass, clip } = await mixed();
    const step = stepLength(resolveGrid(DEFAULT_GRID, PX_PER_BEAT, TempoMap.constant().signatureAt(0))!, TempoMap.constant().signatureAt(0));
    const circle = document.querySelector(`circle[data-point="${volumePoints(keys)[0]!.id}"]`)!;
    await drag(circle, [0, TOP.Keys + y(0.7)], [step * PX_PER_BEAT, TOP.Keys + y(0.7)]);
    await flush();
    expect(volumePoints(bass).map((p) => p.time)).toEqual([2 + step, 6 + step]);
    expect(project().clips[clip.id]!.start).toBe(clip.start + step);
    expect(useArrangementUi.getState().preview).toBeNull();
    await send(cmd("Edit", { type: "Undo" }));
    expect(project().clips[clip.id]!.start).toBe(clip.start);
    expect(volumePoints(bass).map((p) => p.time)).toEqual([2, 6]);
  });
});
