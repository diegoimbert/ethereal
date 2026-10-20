/** Time selection and time-edit shortcuts/menu in the arrangement (time-edits). */
import { useNotices } from "@/features/notifications";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Clip, Command, Project, Track } from "@/generated";
import { useProjectStore, useSelectionStore } from "@/state";
import { itemSelection, playheadBeats } from "@/timeline";
import { cmd, TransportProvider } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { ContextMenuHost } from "@/kit";
import { ArrangementView } from "@/features/arrangement";
import { HEADER_WIDTH, TRACK_HEIGHT } from "@/features/arrangement/layout";
import { resetArrangementUi, useArrangementUi } from "@/features/arrangement/uiStore";
import { resetAutomationUi } from "@/features/automation";
import { coversWholeSong, expandTracks, pasteTracks, selectionFromRect } from "./commands";
import { resetTimeSelection, useTimeSelection } from "./store";

const PX = 24;
const ROW = TRACK_HEIGHT;
/** A y in a row's clip bodies: presses there pass through the clips to the lane (the title band would grab the clip). */
const BODY_Y = 30;
/** Off a clip's edge (its resize handles take the first px): snapping keeps the beat. */
const EDGE_PAD = 7;
const project = () => useProjectStore.getState().project!;
const view = () => document.querySelector<HTMLElement>('[data-feature="arrangement"]')!;

let mock: MockTransport;
let sent: Command[];

const track = (name: string): Track => Object.values(project().tracks).find((t) => t.name === name)!;
const clipsOf = (name: string): Array<[number, number, number]> =>
  Object.values(project().clips)
    .filter((c: Clip) => c.track === track(name).id)
    .map((c): [number, number, number] => [c.start, c.length, c.offset])
    .sort((a, b) => a[0] - b[0]);

async function flush() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

async function drag(el: Element, x: number, y: number, dx: number, dy: number, shiftKey = false) {
  await act(async () => {
    fireEvent.pointerDown(el, {
      button: 0,
      pointerId: 1,
      clientX: x,
      clientY: y,
      shiftKey,
    });
  });
  await act(async () => {
    fireEvent.pointerMove(window, {
      pointerId: 1,
      clientX: x + dx,
      clientY: y + dy,
      shiftKey,
    });
  });
  await act(async () => {
    fireEvent.pointerUp(window, {
      pointerId: 1,
      clientX: x + dx,
      clientY: y + dy,
      shiftKey,
    });
  });
  await flush();
}

/** Select beats `from..to` over the first two rows (Keys, Bass) with a Shift-drag (a section). */
async function selectTime(from: number, to: number, rows = 2) {
  await drag(screen.getByTestId("arrangement-content"), HEADER_WIDTH + from * PX + EDGE_PAD, BODY_Y, (to - from) * PX, (rows - 1) * ROW, true);
}

async function key(k: string, mods: { shift?: boolean } = {}) {
  fireEvent.keyDown(view(), { key: k, metaKey: true, shiftKey: !!mods.shift });
  await flush();
}

beforeEach(async () => {
  resetArrangementUi();
  resetAutomationUi();
  resetTimeSelection();
  useArrangementUi.getState().setGrid({
    type: "Fixed",
    step: { kind: "beats", beats: 1 },
    triplet: false,
  });
  // jsdom has no canvas: the arranger canvas (Canvas2D fallback) draws nothing, but still
  // hit-tests and runs the gestures.
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  mock = new MockTransport({ timers: "manual", seed: 1 });
  sent = [];
  const send = mock.send.bind(mock);
  vi.spyOn(mock, "send").mockImplementation((c, o) => {
    sent.push(c);
    return send(c, o);
  });
  render(
    <TransportProvider transport={mock}>
      <ArrangementView />
      <ContextMenuHost />
    </TransportProvider>,
  );
  await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
  await flush();
});

afterEach(() => {
  mock.dispose();
  useProjectStore.getState().reset();
  useSelectionStore.getState().selectTrack(null);
  itemSelection.getState().clear();
  vi.restoreAllMocks();
});

describe("time selection", () => {
  it("a drag over the lanes selects time on the rows it spans (snapped), drawn over them", async () => {
    await selectTime(4.2, 19.8);
    expect(useTimeSelection.getState().selection).toEqual({
      start: 4,
      end: 20,
      tracks: [track("Keys").id, track("Bass").id],
    });
    expect(itemSelection.getState().timeRange).toEqual({ start: 4, end: 20 });
    expect(screen.getAllByTestId("time-selection")).toHaveLength(2);
    // A click replaces it with the insert marker (a zero-length selection, not a range).
    await drag(screen.getByTestId("arrangement-content"), HEADER_WIDTH + 30 * PX, BODY_Y, 0, 0);
    const sel = useTimeSelection.getState().selection;
    expect(sel && sel.end - sel.start).toBe(0);
    expect(itemSelection.getState().timeRange).toBeNull();
    expect(screen.queryByTestId("time-selection")).toBeNull();
    expect(screen.getAllByTestId("insert-marker")).toHaveLength(1);
  });

  it("a Shift-click extends the section outside it, retracts the nearer edge inside it, and spans the clicked track", async () => {
    const content = screen.getByTestId("arrangement-content");
    const shiftClick = (beat: number, row = 0) => drag(content, HEADER_WIDTH + beat * PX + EDGE_PAD, BODY_Y + row * ROW, 0, 0, true);
    const keys = track("Keys").id;
    await selectTime(4, 12, 1);
    expect(useTimeSelection.getState().selection).toEqual({ start: 4, end: 12, tracks: [keys] });
    await shiftClick(16);
    expect(useTimeSelection.getState().selection).toEqual({ start: 4, end: 16, tracks: [keys] });
    await shiftClick(2);
    expect(useTimeSelection.getState().selection).toEqual({ start: 2, end: 16, tracks: [keys] });
    // Inside, nearer the start / the end.
    await shiftClick(5);
    expect(useTimeSelection.getState().selection).toEqual({ start: 5, end: 16, tracks: [keys] });
    await shiftClick(14);
    expect(useTimeSelection.getState().selection).toEqual({ start: 5, end: 14, tracks: [keys] });
    await shiftClick(13);
    expect(useTimeSelection.getState().selection).toEqual({ start: 5, end: 13, tracks: [keys] });
    // Exactly in the middle: the end moves.
    await shiftClick(9);
    expect(useTimeSelection.getState().selection).toEqual({ start: 5, end: 9, tracks: [keys] });
    // On another track: the section spans it too; the clips stay unselected.
    await shiftClick(8, 1);
    expect(useTimeSelection.getState().selection).toEqual({ start: 5, end: 8, tracks: [keys, track("Bass").id] });
    expect(itemSelection.getState().selected.clip.size).toBe(0);
    expect(screen.getAllByTestId("time-selection")).toHaveLength(2);
  });

  it("a Shift-click extends from the insert marker; without a selection it is a plain click", async () => {
    const content = screen.getByTestId("arrangement-content");
    const click = (beat: number, shift: boolean) => drag(content, HEADER_WIDTH + beat * PX + EDGE_PAD, BODY_Y, 0, 0, shift);
    await click(3, true);
    expect(useTimeSelection.getState().selection).toEqual({ start: 3, end: 3, tracks: [track("Keys").id] });
    await click(7, true);
    expect(useTimeSelection.getState().selection).toEqual({ start: 3, end: 7, tracks: [track("Keys").id] });
  });

  it("⌘E splits every selected track at both edges, one undo step", async () => {
    await selectTime(4, 20);
    await key("e");
    expect(clipsOf("Keys")).toEqual([
      [0, 4, 0],
      [4, 12, 4],
    ]);
    expect(clipsOf("Bass")).toEqual([
      [16, 4, 0],
      [20, 12, 4],
    ]);
    // Split both edges in one gesture.
    const gestures = sent.filter((c) => c.domain === "TimeEdit");
    expect(gestures).toHaveLength(2);
    await act(async () => void (await mock.send(cmd("Edit", { type: "Undo" }))));
    // (The mock records each command; the engine merges the gesture.)
    expect(clipsOf("Bass")).toEqual([[16, 16, 0]]);
  });

  it("⇧⌘⌫ deletes the selected time and ⇧⌘I inserts silence (plain ⌘I is left to import)", async () => {
    await selectTime(4, 20);
    // Plain ⌘I is not taken (nor default-prevented), so the import shortcut still sees it.
    expect(fireEvent.keyDown(view(), { key: "i", metaKey: true })).toBe(true);
    await flush();
    expect(sent.some((c) => c.domain === "TimeEdit")).toBe(false);
    await key("i", { shift: true });
    expect(clipsOf("Bass")).toEqual([[32, 16, 0]]);
    expect(useTimeSelection.getState().selection).not.toBeNull();
    await key("Backspace", { shift: true });
    expect(clipsOf("Bass")).toEqual([[16, 16, 0]]);
    expect(clipsOf("Keys")).toEqual([
      [0, 4, 0],
      [4, 12, 4],
    ]);
    expect(useTimeSelection.getState().selection).toBeNull();
    // Only the selected tracks moved; not the whole song, so markers stay (none here).
    const del = sent.find((c) => c.domain === "TimeEdit" && c.command.type === "DeleteTime");
    expect(del).toMatchObject({
      command: { selection: { start: 4, end: 20, global: false } },
    });
  });

  it("copy and paste time; duplicate moves the selection to the copy", async () => {
    await selectTime(0, 4, 1);
    await key("c", { shift: true });
    expect(useTimeSelection.getState().clipboard).toEqual({
      tracks: 1,
      length: 4,
    });
    await key("d", { shift: true });
    // Over what follows (the clip after the copy is trimmed, not shifted).
    expect(clipsOf("Keys")).toEqual([
      [0, 4, 0],
      [4, 4, 0],
      [8, 8, 8],
    ]);
    expect(useTimeSelection.getState().selection).toMatchObject({
      start: 4,
      end: 8,
    });
    await key("v", { shift: true });
    expect(sent.findLast((c) => c.domain === "TimeEdit" && c.command.type === "Paste")).toMatchObject({
      command: {
        at: 4,
        insert: true,
        tracks: [track("Keys").id, track("Bass").id, track("Drums").id],
      },
    });
  });

  it("right-click inside the selection opens the time menu", async () => {
    await selectTime(4, 20);
    fireEvent.contextMenu(screen.getByTestId("arrangement-content"), {
      clientX: HEADER_WIDTH + 8 * PX,
      clientY: 5,
    });
    await flush();
    for (const label of ["cut time", "copy time", "paste time", "split at selection", "duplicate time", "insert silence", "delete time"]) {
      expect(screen.getByText(label)).toBeTruthy();
    }
    // Insert Silence is ⇧⌘I (plain ⌘I is Import audio…).
    expect(screen.getByText("insert silence").parentElement!.textContent).toMatch(/⇧(⌘|Ctrl\+)I/);
    await act(async () => fireEvent.click(screen.getByText("delete time")));
    await flush();
    expect(clipsOf("Bass")).toEqual([[4, 12, 4]]);
  });

  it("shows the engine's message when an edit is refused (frozen track)", async () => {
    const send = vi.mocked(mock.send).getMockImplementation()!;
    vi.spyOn(mock, "send").mockImplementation((c, o) =>
      c.domain === "TimeEdit"
        ? Promise.reject({
            code: "InvalidState",
            message: 'track "Keys" is frozen: unfreeze it to edit its time line',
          })
        : send(c, o),
    );
    await selectTime(4, 20);
    await key("Backspace", { shift: true });
    // The engine's refusal is a warning toast.
    expect(useNotices.getState().notices.at(-1)).toMatchObject({ level: "Warning", message: expect.stringContaining("frozen") });
    // The selection stays (nothing happened).
    expect(useTimeSelection.getState().selection).not.toBeNull();
  });

  it("⌘E without a time selection splits the selected tracks at the playhead", async () => {
    act(() => useArrangementUi.getState().setTrackSelection([track("Keys").id], track("Keys").id));
    await key("e");
    expect(sent.find((c) => c.domain === "TimeEdit")).toMatchObject({
      command: { type: "Split", tracks: [track("Keys").id] },
    });
  });
});

describe("section-edit: plain ⌘C/X/V/D and ⌫ act on the time selection", () => {
  const timeEdits = () => sent.filter((c) => c.domain === "TimeEdit").map((c) => (c as Extract<Command, { domain: "TimeEdit" }>).command);
  const plain = async (k: string) => {
    fireEvent.keyDown(view(), { key: k, metaKey: !["Backspace", "Delete"].includes(k) });
    await flush();
  };

  it("a plain drag selects clips (no section); Shift switches a drag to a live section and back", async () => {
    const content = screen.getByTestId("arrangement-content");
    await drag(content, HEADER_WIDTH + 0 * PX + EDGE_PAD, BODY_Y, 4 * PX, 0);
    expect(itemSelection.getState().selected.clip.size).toBe(1);
    expect(useTimeSelection.getState().selection).toBeNull();
    // Mid-drag: Shift pressed shows the section (no clips), released goes back.
    await act(async () => {
      fireEvent.pointerDown(content, { button: 0, pointerId: 1, clientX: HEADER_WIDTH + 1 * PX, clientY: BODY_Y });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: HEADER_WIDTH + 3 * PX, clientY: BODY_Y });
    });
    expect(itemSelection.getState().selected.clip.size).toBe(1);
    await act(async () => void fireEvent.keyDown(window, { key: "Shift", shiftKey: true }));
    expect(itemSelection.getState().selected.clip.size).toBe(0);
    expect(useTimeSelection.getState().selection).toMatchObject({ start: 1, end: 3 });
    await act(async () => void fireEvent.keyUp(window, { key: "Shift", shiftKey: false }));
    expect(useTimeSelection.getState().selection).toBeNull();
    expect(itemSelection.getState().selected.clip.size).toBe(1);
    await act(async () => void fireEvent.pointerUp(window, { pointerId: 1, clientX: HEADER_WIDTH + 3 * PX, clientY: BODY_Y }));
    await flush();
    expect(useTimeSelection.getState().selection).toBeNull();
  });

  it("with a time selection, ⌘C copies the section (not the clips) and ⌘D tiles it", async () => {
    await selectTime(0, 4, 1);
    // A Shift-drag selects no clips (selection and section are separate).
    expect(itemSelection.getState().selected.clip.size).toBe(0);
    await plain("c");
    expect(timeEdits()).toMatchObject([{ type: "Copy", selection: { start: 0, end: 4, tracks: [track("Keys").id] } }]);
    expect(useTimeSelection.getState().clipboard).toEqual({ tracks: 1, length: 4 });
    expect(sent.some((c) => c.domain === "Clip")).toBe(false);
    await plain("d");
    await plain("d");
    // Each duplicate pastes over what follows (copy + overwrite paste), never inserting.
    expect(timeEdits().filter((c) => c.type === "Paste" && !c.insert)).toHaveLength(2);
    expect(timeEdits().filter((c) => c.type === "DuplicateTime")).toHaveLength(0);
    // Each copy is exactly 4 beats, placed right after the previous one; the rest keeps its place.
    expect(clipsOf("Keys")).toEqual([
      [0, 4, 0],
      [4, 4, 0],
      [8, 4, 0],
      [12, 4, 12],
    ]);
    expect(useTimeSelection.getState().selection).toMatchObject({ start: 8, end: 12 });
  });

  it("⌘V pastes the section right after the selection; repeated ⌘V tiles it", async () => {
    await selectTime(0, 4, 1);
    await plain("c");
    await plain("v");
    await plain("v");
    const pastes = timeEdits().filter((c) => c.type === "Paste");
    expect(pastes).toMatchObject([
      { at: 4, insert: false },
      { at: 8, insert: false },
    ]);
    expect(useTimeSelection.getState().selection).toMatchObject({ start: 8, end: 12 });
  });

  it("⌘X cuts the section and ⌫ deletes the selected time", async () => {
    await selectTime(0, 4, 1);
    await plain("x");
    expect(timeEdits()).toMatchObject([{ type: "Cut", selection: { start: 0, end: 4 } }]);
    expect(useTimeSelection.getState().selection).toBeNull();
    await selectTime(0, 4, 1);
    await plain("Backspace");
    expect(timeEdits().at(-1)).toMatchObject({ type: "DeleteTime", selection: { start: 0, end: 4 } });
    // Nothing went through the clip actions.
    expect(sent.some((c) => c.domain === "Clip")).toBe(false);
  });

  it("the desktop Edit menu's copy/paste events take the section too", async () => {
    await selectTime(0, 4, 1);
    view().focus();
    await act(async () => void document.dispatchEvent(new Event("copy", { bubbles: true, cancelable: true })));
    await flush();
    expect(timeEdits()).toMatchObject([{ type: "Copy" }]);
    await act(async () => void document.dispatchEvent(new Event("paste", { bubbles: true, cancelable: true })));
    await flush();
    expect(timeEdits().at(-1)).toMatchObject({ type: "Paste", at: 4 });
  });

  it("without a time selection the clip shortcuts keep working; the latest copy wins ⌘V", async () => {
    const keys = Object.values(project().clips).find((c) => c.track === track("Keys").id)!;
    act(() => itemSelection.getState().select("clip", [keys.id], "replace"));
    await plain("c");
    await plain("d");
    expect(timeEdits()).toEqual([]);
    expect(clipsOf("Keys")).toHaveLength(2);
    // A time copy, then no selection: ⌘V pastes the section at the selection cursor (the
    // start of the last selection), never at the playhead.
    await selectTime(0, 4, 1);
    await plain("c");
    act(() => useTimeSelection.getState().setSelection(null));
    act(() => useTimeSelection.setState({ cursor: 16 }));
    const playhead = playheadBeats();
    await plain("v");
    expect(timeEdits().at(-1)).toMatchObject({ type: "Paste", at: 16, insert: false });
    // Pasting never moves the playhead (insert marker).
    expect(playheadBeats()).toBe(playhead);
    // A clip copy after that: ⌘V is the clip paste again.
    act(() => itemSelection.getState().select("clip", [keys.id], "replace"));
    await plain("c");
    expect(useTimeSelection.getState().clipboard).toBeNull();
    const before = timeEdits().length;
    await plain("v");
    expect(timeEdits()).toHaveLength(before);
  });
});

describe("time selection helpers", () => {
  const rows = (p: Project) =>
    Object.values(p.tracks)
      .filter((t) => t.kind !== "Master")
      .map((t, i) => ({ track: t, y: i * ROW, height: ROW }));

  it("selectionFromRect keeps audio/MIDI/group rows and needs a range", () => {
    const p = project();
    const toBeats = (px: number) => Math.round(px / PX);
    const r = rows(p);
    const sel = selectionFromRect({ x0: HEADER_WIDTH, y0: 0, x1: HEADER_WIDTH + 4 * PX, y1: 10 * ROW }, r, HEADER_WIDTH, toBeats)!;
    expect(sel.start).toBe(0);
    expect(sel.end).toBe(4);
    expect(sel.tracks.every((id) => ["Audio", "Midi", "Group"].includes(p.tracks[id]!.kind))).toBe(true);
    expect(selectionFromRect({ x0: HEADER_WIDTH, y0: 0, x1: HEADER_WIDTH + 2, y1: ROW }, r, HEADER_WIDTH, toBeats)).toBeNull();
  });

  it("the whole song is every audio/MIDI/group track (groups bring their children)", async () => {
    const group = "01J0000000000000000000GRP0";
    await act(async () => {
      await mock.send(
        cmd("Track", {
          type: "Create",
          id: group,
          kind: "Group",
          name: "Bus",
          color: null,
          parent: null,
          before: null,
        }),
      );
      await mock.send(
        cmd("Track", {
          type: "Move",
          id: track("Keys").id,
          parent: group,
          before: null,
        }),
      );
    });
    const p = project();
    const top = Object.values(p.tracks).filter((t) => t.parent === null && ["Audio", "Midi", "Group"].includes(t.kind));
    expect(
      coversWholeSong(
        p,
        top.map((t) => t.id),
      ),
    ).toBe(true);
    expect(coversWholeSong(p, [track("Bass").id])).toBe(false);
    expect(expandTracks(p, [group])).toEqual([group, track("Keys").id]);
    expect(pasteTracks(p, null)).toEqual([]);
    expect(pasteTracks(p, track("Keys").id)[0]).toBe(track("Keys").id);
  });
});

describe("section-edit: the insert marker (Ableton's edit cursor, separate from the playhead)", () => {
  const timeEdits = () => sent.filter((c) => c.domain === "TimeEdit").map((c) => (c as Extract<Command, { domain: "TimeEdit" }>).command);
  const locates = () => sent.filter((c) => c.domain === "Transport" && c.command.type === "Locate").map((c) => (c.command as { position: number }).position);
  const plain = async (k: string) => {
    fireEvent.keyDown(view(), { key: k, metaKey: true });
    await flush();
  };
  const click = (beats: number, row: number) => drag(screen.getByTestId("arrangement-content"), HEADER_WIDTH + beats * PX, row * ROW + BODY_Y, 0, 0);
  const transport = async (type: "Play" | "Stop") => {
    await act(async () => {
      await mock.send(cmd("Transport", { type }));
    });
    await waitFor(() => expect(useProjectStore.getState().transport?.playing).toBe(type === "Play"));
    await flush();
  };

  it("a click on a lane places it there (snapped, on that track) and draws it; while stopped Play starts from it", async () => {
    await click(10.3, 1);
    expect(useTimeSelection.getState().selection).toEqual({ start: 10, end: 10, tracks: [track("Bass").id] });
    const marker = screen.getByTestId("insert-marker");
    expect(marker.dataset.track).toBe(track("Bass").id);
    // A zero-length selection is not a range: no time range, no time menu.
    expect(itemSelection.getState().timeRange).toBeNull();
    // Stopped: the playhead goes there, so Play starts from the marker.
    expect(locates()).toEqual([10]);
    await waitFor(() => expect(playheadBeats()).toBe(10));
  });

  it("a range selection sets the selection cursor at its start: stopped, the playhead follows; playing, the next stop goes there", async () => {
    await selectTime(4, 8, 1);
    expect(useTimeSelection.getState().cursor).toBe(4);
    expect(locates()).toEqual([4]);
    await waitFor(() => expect(playheadBeats()).toBe(4));
    await transport("Play");
    const before = locates().length;
    await selectTime(12, 16, 1);
    expect(useTimeSelection.getState().cursor).toBe(12);
    // Playing: no jump.
    expect(locates()).toHaveLength(before);
    await transport("Stop");
    expect(locates().at(-1)).toBe(12);
  });

  it("a click on a clip's title sets the cursor at the clip start (the playhead follows while stopped)", async () => {
    const keys = Object.values(project().clips).find((c) => c.track === track("Keys").id)!;
    // The arranger canvas hit-tests the press (its test API places the title band).
    const api = (window as unknown as { __ether: { arrangement: { handleRect(id: string, part: string): { x: number; y: number } | null } } }).__ether
      .arrangement;
    const title = api.handleRect(keys.id, "header")!;
    await drag(screen.getByTestId("arrangement-content"), title.x + 4, title.y + 4, 0, 0);
    expect(useTimeSelection.getState().cursor).toBe(keys.start);
    expect(locates().at(-1)).toBe(keys.start);
  });

  it("while playing, a click and a paste never move the playhead; ⌘V targets the marker; Stop returns to it", async () => {
    await selectTime(0, 4, 1);
    await plain("c");
    await transport("Play");
    const before = locates().length;
    await click(10.3, 1);
    expect(useTimeSelection.getState().selection).toMatchObject({ start: 10, end: 10 });
    await plain("v");
    expect(timeEdits().at(-1)).toMatchObject({ type: "Paste", at: 10, insert: false });
    expect(timeEdits().at(-1)).toMatchObject({ tracks: expect.arrayContaining([track("Bass").id]) });
    expect(locates()).toHaveLength(before);
    // The pasted range is selected, so ⌘V again tiles after it.
    expect(useTimeSelection.getState().selection).toMatchObject({ start: 10, end: 14 });
    await plain("v");
    expect(timeEdits().at(-1)).toMatchObject({ type: "Paste", at: 14 });
    expect(locates()).toHaveLength(before);
    // On Stop the playhead goes to the marker placed while playing (Play then starts there).
    await transport("Stop");
    expect(locates().at(-1)).toBe(10);
  });

  it("⌘C needs a range: with only the marker it acts on clips; ⌘E splits at the marker, not the playhead", async () => {
    // The click lands on the Keys clip's body: it selects the clip and places the marker.
    await click(2.2, 0);
    expect(useTimeSelection.getState().selection).toMatchObject({ start: 2, end: 2 });
    await plain("c");
    expect(timeEdits()).toEqual([]);
    act(() => useTimeSelection.setState({ clipboard: null }));
    await act(async () => {
      await mock.send(cmd("Transport", { type: "Locate", position: 7 }));
    });
    await plain("e");
    expect(clipsOf("Keys").map(([start]) => start)).toContain(2);
    expect(clipsOf("Keys").map(([start]) => start)).not.toContain(7);
  });

  it("⌘E with only the marker on empty lane space splits that track at the marker", async () => {
    act(() => useTimeSelection.getState().setSelection({ start: 3, end: 3, tracks: [track("Bass").id] }));
    await plain("e");
    expect(timeEdits()).toMatchObject([{ type: "Split", at: 3, tracks: [track("Bass").id] }]);
  });

  it("clip paste lands at the marker and moves the marker (not the playhead) to the pasted end", async () => {
    const keys = Object.values(project().clips).find((c) => c.track === track("Keys").id)!;
    act(() => itemSelection.getState().select("clip", [keys.id], "replace"));
    await plain("c");
    await click(40, 0);
    const playhead = playheadBeats();
    const locatesBefore = locates().length;
    await plain("v");
    expect(clipsOf("Keys").some(([start]) => start === 40)).toBe(true);
    expect(useTimeSelection.getState().selection).toMatchObject({ start: 40 + keys.length, end: 40 + keys.length });
    expect(locates()).toHaveLength(locatesBefore);
    expect(playheadBeats()).toBe(playhead);
  });
});
