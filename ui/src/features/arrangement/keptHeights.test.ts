import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Project, ProjectId, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { MockTransport } from "@/transport/mock";
import { bindTrackHeightsToProject, keptHeightsKey, loadKeptHeights } from "./keptHeights";
import { TRACK_HEIGHT } from "./layout";
import { resetArrangementUi, useArrangementUi } from "./uiStore";

let mock: MockTransport;
let a: Project;
let unbind: () => void;
const other = (p: Project): Project => ({ ...structuredClone(p), id: "019b0000-0000-7000-8000-000000000777" as ProjectId });
const firstTrack = (p: Project) => Object.values(p.tracks).find((t) => t.kind !== "Master")!.id as TrackId;

beforeEach(async () => {
  vi.useFakeTimers();
  localStorage.clear();
  mock = new MockTransport({ timers: "manual", seed: 1 });
  a = (await mock.connect())!;
  unbind = bindTrackHeightsToProject();
  useProjectStore.getState().loadProject(a);
});

afterEach(() => {
  unbind();
  mock.dispose();
  useProjectStore.getState().reset();
  resetArrangementUi();
  localStorage.clear();
  vi.useRealTimers();
});

describe("the track heights are kept per project", () => {
  it("saves them as they change, and sets them again when the project opens", () => {
    const track = firstTrack(a);
    useArrangementUi.getState().setHeight(track, 120);
    useArrangementUi.getState().scaleHeights(1.5);
    vi.advanceTimersByTime(500);
    const kept = loadKeptHeights(a.id)!;
    expect(kept.default).toBe(TRACK_HEIGHT * 1.5);
    expect(kept.heights[track]).toBe(useArrangementUi.getState().heights.get(track));

    // Another project: its own (default) heights; back to the first: its heights again.
    useProjectStore.getState().loadProject(other(a));
    expect(useArrangementUi.getState().defaultHeight).toBe(TRACK_HEIGHT);
    expect(useArrangementUi.getState().heights.size).toBe(0);
    useProjectStore.getState().loadProject(a);
    expect(useArrangementUi.getState().defaultHeight).toBe(TRACK_HEIGHT * 1.5);
    expect(useArrangementUi.getState().heights.get(track)).toBe(kept.heights[track]);
  });

  it("saves a change still pending when another project opens", () => {
    useArrangementUi.getState().scaleHeights(2);
    useProjectStore.getState().loadProject(other(a));
    expect(loadKeptHeights(a.id)?.default).toBe(TRACK_HEIGHT * 2);
  });

  it("keeps nothing for a project at the default heights", () => {
    useArrangementUi.getState().scaleHeights(2);
    vi.advanceTimersByTime(500);
    useArrangementUi.getState().scaleHeights(0.5);
    vi.advanceTimersByTime(500);
    expect(localStorage.getItem(keptHeightsKey(a.id))).toBeNull();
  });
});
