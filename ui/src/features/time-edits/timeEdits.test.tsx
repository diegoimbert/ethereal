/** Time selection and time-edit shortcuts/menu in the arrangement (time-edits). */
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Clip, Command, Project, Track } from "@/generated";
import { useProjectStore, useSelectionStore } from "@/state";
import { itemSelection, playheadBeats } from "@/timeline";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { ContextMenuHost } from "@/kit";
import { ArrangementView } from "@/features/arrangement";
import { HEADER_WIDTH, TRACK_HEIGHT } from "@/features/arrangement/layout";
import { resetArrangementUi, useArrangementUi } from "@/features/arrangement/uiStore";
import { resetAutomationUi } from "@/features/automation";
import { coversWholeSong, expandTracks, pasteTracks, selectionFromRect } from "./commands";
import { resetTimeSelection, useTimeSelection } from "./store";

const PX = 24;
const ROW = TRACK_HEIGHT;
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

async function drag(el: Element, x: number, y: number, dx: number, dy: number) {
  await act(async () => {
    fireEvent.pointerDown(el, {
      button: 0,
      pointerId: 1,
      clientX: x,
      clientY: y,
    });
  });
  await act(async () => {
    fireEvent.pointerMove(window, {
      pointerId: 1,
      clientX: x + dx,
      clientY: y + dy,
    });
  });
  await act(async () => {
    fireEvent.pointerUp(window, {
      pointerId: 1,
      clientX: x + dx,
      clientY: y + dy,
    });
  });
  await flush();
}

/** Select beats `from..to` over the first two rows (Keys, Bass) with a marquee drag. */
async function selectTime(from: number, to: number, rows = 2) {
  await drag(screen.getByTestId("arrangement-content"), HEADER_WIDTH + from * PX, 5, (to - from) * PX, (rows - 1) * ROW);
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
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(
    () => new Proxy({}, { get: () => () => {} }) as unknown as CanvasRenderingContext2D,
  );
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
    // A click clears it.
    await drag(screen.getByTestId("arrangement-content"), HEADER_WIDTH + 30 * PX, 5, 0, 0);
    expect(useTimeSelection.getState().selection).toBeNull();
    expect(screen.queryByTestId("time-selection")).toBeNull();
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
    expect(clipsOf("Keys")).toEqual([
      [0, 4, 0],
      [4, 4, 0],
      [8, 12, 4],
    ]);
    expect(useTimeSelection.getState().selection).toMatchObject({
      start: 4,
      end: 8,
    });
    await key("v", { shift: true });
    expect(sent.find((c) => c.domain === "TimeEdit" && c.command.type === "Paste")).toMatchObject({
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
    for (const label of ["Cut Time", "Copy Time", "Paste Time", "Split at Selection", "Duplicate Time", "Insert Silence", "Delete Time"]) {
      expect(screen.getByText(label)).toBeTruthy();
    }
    // Insert Silence is ⇧⌘I (plain ⌘I is Import audio…).
    expect(screen.getByText("Insert Silence").parentElement!.textContent).toMatch(/⇧(⌘|Ctrl\+)I/);
    await act(async () => fireEvent.click(screen.getByText("Delete Time")));
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
    expect(screen.getByTestId("time-edit-notice").textContent).toContain("frozen");
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

  it("with a time selection, ⌘C copies the section (not the clips) and ⌘D tiles it", async () => {
    await selectTime(0, 4, 1);
    // The marquee also selected the clip it crossed: the section still wins.
    expect(itemSelection.getState().selected.clip.size).toBe(1);
    await plain("c");
    expect(timeEdits()).toMatchObject([{ type: "Copy", selection: { start: 0, end: 4, tracks: [track("Keys").id] } }]);
    expect(useTimeSelection.getState().clipboard).toEqual({ tracks: 1, length: 4 });
    expect(sent.some((c) => c.domain === "Clip")).toBe(false);
    await plain("d");
    await plain("d");
    expect(timeEdits().filter((c) => c.type === "DuplicateTime")).toHaveLength(2);
    // Each copy is exactly 4 beats, placed right after the previous one.
    expect(clipsOf("Keys")).toEqual([
      [0, 4, 0],
      [4, 4, 0],
      [8, 4, 0],
      [12, 12, 4],
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
    // A time copy, then a click elsewhere (no selection): ⌘V pastes the section at the playhead.
    await selectTime(0, 4, 1);
    await plain("c");
    act(() => useTimeSelection.getState().setSelection(null));
    const at = playheadBeats();
    await plain("v");
    expect(timeEdits().at(-1)).toMatchObject({ type: "Paste", at, insert: false });
    // The playhead moves to the end of the pasted section (so ⌘V again continues it).
    expect(playheadBeats()).toBe(at + 4);
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
