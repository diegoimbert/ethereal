/** Take lanes in the arrangement (v0.2, comping), against the MockTransport. */
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Command, Track } from "@/generated";
import { useProjectStore, useSelectionStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { ContextMenuHost } from "@/kit";
import { ArrangementView } from "@/features/arrangement/ArrangementView";
import { layoutRows, mainHeight, TRACK_HEIGHT } from "@/features/arrangement/layout";
import { resetArrangementUi, useArrangementUi } from "@/features/arrangement/uiStore";
import { resetAutomationUi } from "@/features/automation";
import { CommandFailedError } from "@/transport/EngineTransport";
import { compOf, lanesOf } from "./model";
import { useCompingUi } from "./store";
import { TAKE_LANE_HEIGHT } from "./height";

const PX = 24;
const store = () => useProjectStore.getState();
const project = () => store().project!;
let mock: MockTransport;
let sent: Command[];
let n = 0;
const id = () => `01JC${String(++n).padStart(22, "0")}`;

const track = (name: string): Track => Object.values(project().tracks).find((t) => t.name === name)!;

async function flush() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

async function send(c: Command) {
  await act(async () => {
    await mock.send(c);
  });
}

function stubCanvas() {
  const ctx = new Proxy({}, { get: (t: Record<string, unknown>, k: string) => (k in t ? t[k] : () => {}), set: (t, k, v) => ((t[k as string] = v), true) });
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(() => ctx as unknown as CanvasRenderingContext2D);
}

/** Bass gets two takes (4-beat MIDI clips at 0) and a comp of the first. */
async function setup(): Promise<{ bass: Track; a: string; b: string }> {
  const bass = track("Bass");
  const [a, b] = [id(), id()];
  for (const lane of [a, b]) {
    await send(cmd("Take", { type: "CreateLane", id: lane, track: bass.id, name: null, before: null }));
    const clip = id();
    await send(cmd("Clip", { type: "CreateMidi", id: clip, track: bass.id, start: 0, length: 4, name: null }));
    await send(cmd("Take", { type: "MoveToLane", clips: [clip], lane }));
  }
  await send(cmd("Take", { type: "SetComp", id: id(), split_id: id(), track: bass.id, lane: a, start: 0, end: 4 }));
  await flush();
  return { bass, a, b };
}

beforeEach(async () => {
  resetArrangementUi();
  resetAutomationUi();
  useCompingUi.setState({ expanded: new Set(), selected: null, swipe: null, audition: new Map(), error: null });
  useArrangementUi.getState().setGrid({ type: "Fixed", step: { kind: "beats", beats: 1 }, triplet: false });
  stubCanvas();
  mock = new MockTransport({ timers: "manual", seed: 1 });
  sent = [];
  const orig = mock.send.bind(mock);
  vi.spyOn(mock, "send").mockImplementation((c, o) => {
    sent.push(c);
    return orig(c, o);
  });
  render(
    <TransportProvider transport={mock}>
      <ArrangementView />
      <ContextMenuHost />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  await flush();
});

afterEach(() => {
  mock.dispose();
  store().reset();
  useSelectionStore.getState().selectTrack(null);
  itemSelection.getState().clear();
  vi.restoreAllMocks();
});

const takeCommands = (type: string) => sent.filter((c) => c.domain === "Take" && c.command.type === type).map((c) => c.command);

describe("take lanes", () => {
  it("lay out under the track's main lane", () => {
    const tracks = [{ id: "t", kind: "Audio", parent: null } as unknown as Track];
    const [row] = layoutRows(tracks, new Set(), () => 10, () => TRACK_HEIGHT, null, () => 2 * TAKE_LANE_HEIGHT);
    expect(row).toMatchObject({ laneHeight: TRACK_HEIGHT + 2 * TAKE_LANE_HEIGHT, takesHeight: 2 * TAKE_LANE_HEIGHT, height: TRACK_HEIGHT + 2 * TAKE_LANE_HEIGHT + 10 });
    expect(mainHeight(row!)).toBe(TRACK_HEIGHT);
  });

  it("toggle from the header, show the takes and the comp on the main lane", async () => {
    const { bass } = await setup();
    // Take clips are not main-lane clips; the comp shows on the main lane.
    expect(document.querySelectorAll(`[data-lane="${bass.id}"] [data-clip-id]`)).toHaveLength(Object.values(project().clips).filter((c) => c.track === bass.id && c.lane == null).length);
    expect(document.querySelectorAll(`[data-comp="${bass.id}"] [data-comp-region]`)).toHaveLength(1);
    expect(document.querySelectorAll("[data-take-lane]")).toHaveLength(0);
    fireEvent.click(screen.getByRole("button", { name: "Show takes of Bass" }));
    await flush();
    expect([...document.querySelectorAll("[data-take-lane] .eth-takes__name")].map((e) => e.textContent)).toEqual(["Take 1", "Take 2"]);
    fireEvent.click(screen.getByRole("button", { name: "Hide takes of Bass" }));
    await flush();
    expect(document.querySelectorAll("[data-take-lane]")).toHaveLength(0);
  });

  it("swipe across a take lane comps that range (one command), Up/Down pick another take", async () => {
    const { bass, a, b } = await setup();
    act(() => useCompingUi.getState().toggle(bass.id, true));
    await flush();
    const lane = document.querySelector<HTMLElement>(`[data-lane-take="${b}"]`)!;
    await act(async () => {
      fireEvent.pointerDown(lane, { button: 0, pointerId: 1, clientX: PX, clientY: 5 });
    });
    await act(async () => {
      fireEvent.pointerMove(window, { pointerId: 1, clientX: 2 * PX, clientY: 5 });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: 3 * PX, clientY: 5 });
    });
    expect(screen.getByTestId("swipe-preview")).toBeTruthy();
    await act(async () => {
      fireEvent.pointerUp(window, { pointerId: 1, clientX: 3 * PX, clientY: 5 });
    });
    await flush();
    expect(takeCommands("SetComp").at(-1)).toMatchObject({ track: bass.id, lane: b, start: 1, end: 3 });
    const regions = () => compOf(project(), bass.id).map((r) => [r.lane, r.start, r.end]);
    expect(regions()).toEqual([
      [a, 0, 1],
      [b, 1, 3],
      [a, 3, 4],
    ]);
    expect(useCompingUi.getState().selected?.track).toBe(bass.id);
    // Up: the previous take for the selected region.
    fireEvent.keyDown(document.querySelector(".eth-arr")!, { key: "ArrowUp" });
    await flush();
    expect(regions()).toEqual([
      [a, 0, 1],
      [a, 1, 3],
      [a, 3, 4],
    ]);
    fireEvent.keyDown(document.querySelector(".eth-arr")!, { key: "ArrowDown" });
    await flush();
    expect(regions()[1]).toEqual([b, 1, 3]);
    // A click (no drag) on a lane picks that take for the region under the pointer.
    const laneA = document.querySelector<HTMLElement>(`[data-lane-take="${a}"]`)!;
    await act(async () => {
      fireEvent.pointerDown(laneA, { button: 0, pointerId: 1, clientX: 2 * PX, clientY: 5 });
      fireEvent.pointerUp(window, { pointerId: 1, clientX: 2 * PX, clientY: 5 });
    });
    await flush();
    expect(regions()[1]).toEqual([a, 1, 3]);
  });

  it("audition, rename, flatten and delete from the lane header", async () => {
    const { bass, a, b } = await setup();
    act(() => useCompingUi.getState().toggle(bass.id, true));
    await flush();
    const audition = screen.getByRole("button", { name: "Audition Take 2" });
    fireEvent.click(audition);
    await flush();
    expect(takeCommands("Audition").at(-1)).toEqual({ type: "Audition", track: bass.id, lane: b });
    expect(screen.getByRole("button", { name: "Stop auditioning Take 2" }).getAttribute("aria-pressed")).toBe("true");

    fireEvent.doubleClick(screen.getByTitle("Take 1 (double-click to rename)"));
    const input = screen.getByLabelText("Take name");
    fireEvent.change(input, { target: { value: "Best" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await flush();
    expect(project().take_lanes[a]?.name).toBe("Best");

    fireEvent.contextMenu(screen.getByRole("group", { name: "Best take" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Flatten Comp" }));
    await flush();
    expect(takeCommands("Flatten")).toHaveLength(1);
    expect(compOf(project(), bass.id)).toEqual([]);

    fireEvent.contextMenu(screen.getByRole("group", { name: "Take 2 take" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Delete Take" }));
    await flush();
    expect(lanesOf(project(), bass.id).map((l) => l.id)).toEqual([a]);
  });

  it("new take lanes from the track menu; rejected edits (a frozen track) are shown", async () => {
    const bass = track("Bass");
    fireEvent.contextMenu(screen.getByRole("group", { name: "Bass track" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "New Take Lane" }));
    await flush();
    expect(lanesOf(project(), bass.id)).toHaveLength(1);
    const lane = lanesOf(project(), bass.id)[0]!;
    // The controller rejects content edits on frozen tracks (freeze::check_editable).
    const orig = vi.mocked(mock.send).getMockImplementation()!;
    vi.mocked(mock.send).mockImplementation((c, o) =>
      c.domain === "Take" ? Promise.reject(new CommandFailedError({ code: "InvalidState", message: "Bass is frozen: unfreeze it to edit" })) : orig(c, o),
    );
    const el = document.querySelector<HTMLElement>(`[data-lane-take="${lane.id}"]`)!;
    await act(async () => {
      fireEvent.pointerDown(el, { button: 0, pointerId: 1, clientX: PX, clientY: 5 });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: 3 * PX, clientY: 5 });
      fireEvent.pointerUp(window, { pointerId: 1, clientX: 3 * PX, clientY: 5 });
    });
    await flush();
    expect((await screen.findByRole("alert")).textContent).toContain("frozen");
  });
});
