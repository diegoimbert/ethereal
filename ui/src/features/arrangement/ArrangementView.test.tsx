import { act, createEvent, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Clip, Command, Track } from "@/generated";
import { playheadStore, tracksOrdered, useEditorStore, useProjectStore, useSelectionStore } from "@/state";
import { itemSelection, wheelZoomFactor } from "@/timeline";
import { cmd, MockTransport, newId, TransportProvider } from "@/transport";
import { ContextMenuHost } from "@/kit";
import { ArrangementView } from "./ArrangementView";
import { BROWSER_DRAG_MIME } from "./browserDrop";
import { clearClipboard } from "./clipboard";
import { AUTOMATION_BAR_HEIGHT, LANE_HEIGHT, resetAutomationUi } from "@/features/automation";
import { HEADER_WIDTH, MAX_TRACK_HEIGHT, MIN_TRACK_HEIGHT, TRACK_HEIGHT } from "./layout";
import { arrangementView, resetArrangementUi, useArrangementUi } from "./uiStore";

// Default zoom is 24 px/beat; tests use a fixed 1-beat grid.
const PX = 24;
/** Row pitch (closed automation takes no room). */
const ROW = TRACK_HEIGHT;
const store = () => useProjectStore.getState();
const project = () => store().project!;

let mock: MockTransport;
let sent: Command[];

function trackByName(name: string): Track {
  const t = Object.values(project().tracks).find((x) => x.name === name);
  if (!t) throw new Error(`no track ${name}`);
  return t;
}

function clipByName(name: string): Clip {
  const c = Object.values(project().clips).find((x) => x.name === name);
  if (!c) throw new Error(`no clip ${name}`);
  return c;
}

const startOf = (c: Clip) => c.start;
const clipEl = (c: Clip) => document.querySelector<HTMLElement>(`[data-clip-id="${c.id}"]`)!;

async function flush() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

/** Fake 2D context: enough for the clip canvases to draw (and request peaks). */
function stubCanvas() {
  const ctx = new Proxy(
    {},
    {
      get: (target: Record<string, unknown>, key: string) => (key in target ? target[key] : () => {}),
      set: (target: Record<string, unknown>, key: string, value: unknown) => {
        target[key] = value;
        return true;
      },
    },
  );
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(() => ctx as unknown as CanvasRenderingContext2D);
}

async function renderView() {
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
  await waitFor(() => expect(store().project).not.toBeNull());
  await flush();
}

/** Pointer drag on `el` by (dx, dy) px, starting at (x, y). */
async function drag(el: Element, dx: number, dy = 0, opts: { x?: number; y?: number; up?: PointerEventInit } = {}) {
  const x = opts.x ?? 100;
  const y = opts.y ?? 20;
  await act(async () => {
    fireEvent.pointerDown(el, { button: 0, pointerId: 1, clientX: x, clientY: y });
  });
  await act(async () => {
    fireEvent.pointerMove(window, { pointerId: 1, clientX: x + dx / 2, clientY: y + dy / 2 });
    fireEvent.pointerMove(window, { pointerId: 1, clientX: x + dx, clientY: y + dy });
  });
  await act(async () => {
    fireEvent.pointerUp(window, { pointerId: 1, clientX: x + dx, clientY: y + dy, ...opts.up });
  });
  await flush();
}

async function undo() {
  await act(async () => {
    await mock.send(cmd("Edit", { type: "Undo" }));
  });
}

beforeEach(async () => {
  resetArrangementUi();
  resetAutomationUi();
  useArrangementUi.getState().setGrid({ type: "Fixed", step: { kind: "beats", beats: 1 }, triplet: false });
  stubCanvas();
  await renderView();
});

afterEach(() => {
  clearClipboard();
  mock.dispose();
  store().reset();
  useSelectionStore.getState().selectTrack(null);
  itemSelection.getState().clear();
  useEditorStore.setState({ clip: null, request: 0 });
  vi.restoreAllMocks();
});

describe("ArrangementView: tracks", () => {
  it("renders a header and lane per track, returns and master last", () => {
    const names = [...document.querySelectorAll(".eth-arr-header__name")].map((e) => e.textContent);
    expect(names).toEqual(["Keys", "Bass", "Drums", "A Delay", "Master"]);
    expect(document.querySelectorAll('[data-slot="automation"]')).toHaveLength(5);
    expect(clipEl(clipByName("Chords"))).toBeTruthy();
    expect(clipEl(clipByName("Bassline"))).toBeTruthy();
  });

  it("selects the track on header click", () => {
    fireEvent.click(screen.getByRole("group", { name: "Bass track" }));
    expect(useSelectionStore.getState().selectedTrack).toBe(trackByName("Bass").id);
  });

  it("mutes, solos and arms with the existing commands", async () => {
    fireEvent.click(screen.getByRole("button", { name: "Mute Keys" }));
    await flush();
    expect(trackByName("Keys").mixer.mute).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Solo Bass" }));
    await flush();
    expect(trackByName("Bass").mixer.solo).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Arm Drums" }));
    await flush();
    expect(store().armedTracks).toEqual([trackByName("Drums").id]);
    expect(screen.queryByRole("button", { name: "Arm Master" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Solo Master" })).toBeNull();
  });

  it("dragging New track between rows asks for the type there, and creates the track at that spot", async () => {
    const names = () => tracksOrdered(project()).map((t) => t.name);
    const button = screen.getByRole("button", { name: /New track/ });
    // jsdom has no layout: give the lanes a box so the pointer is "over" them.
    vi.spyOn(screen.getByTestId("arrangement-content"), "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 1200, 800));
    await act(async () => {
      fireEvent.pointerDown(button, { button: 0, pointerId: 1, clientX: 10, clientY: -20 });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: 300, clientY: ROW });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: 300, clientY: ROW + 5 });
    });
    expect(screen.getByTestId("track-drop-line").style.top).toBe(`${ROW}px`);
    await act(async () => {
      fireEvent.pointerUp(window, { pointerId: 1, clientX: 300, clientY: ROW + 5 });
      fireEvent.click(button);
    });
    // The draft row sits between Keys and Bass; Escape cancels it, M makes a MIDI track.
    const draft = screen.getByRole("group", { name: "New track" });
    const rowsNow = [...document.querySelectorAll<HTMLElement>(".eth-arr-row")].map((r) => r.dataset.track);
    expect(rowsNow.indexOf("__draft_track__")).toBe(1);
    fireEvent.keyDown(draft, { key: "m" });
    await flush();
    expect(screen.queryByRole("group", { name: "New track" })).toBeNull();
    expect(names().slice(0, 3)).toEqual(["Keys", expect.stringMatching(/MIDI/), "Bass"]);

    fireEvent.click(button);
    fireEvent.keyDown(screen.getByRole("group", { name: "New track" }), { key: "Escape" });
    expect(screen.queryByRole("group", { name: "New track" })).toBeNull();
  });

  it("adds a MIDI track with the built-in synth and an audio track, each one undo step", async () => {
    const before = Object.keys(project().tracks).length;
    fireEvent.click(screen.getByRole("button", { name: /New track/ }));
    fireEvent.click(screen.getByRole("button", { name: "Create MIDI track" }));
    await flush();
    const midi = Object.values(project().tracks).filter((t) => t.kind === "Midi").at(-1)!;
    expect(Object.keys(project().tracks)).toHaveLength(before + 1);
    expect(useSelectionStore.getState().selectedTrack).toBe(midi.id);
    const devices = Object.values(project().devices).filter((d) => d.track === midi.id);
    expect(devices.map((d) => d.kind)).toEqual([{ type: "Builtin", device: { type: "Synth" } }]);

    fireEvent.click(screen.getByRole("button", { name: /New track/ }));
    fireEvent.keyDown(screen.getByRole("group", { name: "New track" }), { key: "a" });
    await flush();
    expect(Object.keys(project().tracks)).toHaveLength(before + 2);
    await undo();
    await undo();
    expect(Object.keys(project().tracks)).toHaveLength(before);
    expect(Object.values(project().devices).filter((d) => d.track === midi.id)).toEqual([]);
  });

  it("nests and folds groups", async () => {
    const group = newId();
    const child = trackByName("Bass").id;
    await act(async () => {
      await mock.send(cmd("Track", { type: "Create", id: group, kind: "Group", name: "Grp", color: null, parent: null, before: null }));
      await mock.send(cmd("Track", { type: "Move", id: child, parent: group, before: null }));
    });
    await flush();
    const header = screen.getByRole("group", { name: "Bass track" });
    expect(header.style.paddingLeft).toBe("26px");
    // The group lane summarizes the child's clip.
    expect(document.querySelectorAll(`[data-lane="${group}"] .eth-arr-lane__summary`)).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "Fold Grp" }));
    expect(screen.queryByRole("group", { name: "Bass track" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Unfold Grp" }));
    expect(screen.getByRole("group", { name: "Bass track" })).toBeTruthy();
  });
});

describe("ArrangementView: clip editing", () => {
  it("click selects a clip and its track; shift adds; cmd toggles", async () => {
    const chords = clipByName("Chords");
    const bass = clipByName("Bassline");
    await drag(clipEl(chords), 0);
    expect([...itemSelection.getState().selected.clip]).toEqual([chords.id]);
    expect(useSelectionStore.getState().selectedTrack).toBe(chords.track);
    await act(async () => {
      fireEvent.pointerDown(clipEl(bass), { button: 0, clientX: 10, clientY: 70, shiftKey: true });
      fireEvent.pointerUp(window, { clientX: 10, clientY: 70 });
    });
    expect(itemSelection.getState().selected.clip.size).toBe(2);
    await act(async () => {
      fireEvent.pointerDown(clipEl(bass), { button: 0, clientX: 10, clientY: 70, metaKey: true });
      fireEvent.pointerUp(window, { clientX: 10, clientY: 70, metaKey: true });
    });
    expect([...itemSelection.getState().selected.clip]).toEqual([chords.id]);
  });

  it("moves a clip with snapping as one undo step", async () => {
    const chords = clipByName("Chords");
    await drag(clipEl(chords), 4.3 * PX);
    expect(startOf(project().clips[chords.id]!)).toBe(4);
    expect(store().history.can_undo).toBe(true);
    await undo();
    expect(startOf(project().clips[chords.id]!)).toBe(0);
  });

  it("bypasses snapping with alt", async () => {
    const chords = clipByName("Chords");
    await drag(clipEl(chords), 1.5 * PX, 0, {});
    expect(startOf(project().clips[chords.id]!)).toBe(2);
    await undo();
    await act(async () => {
      fireEvent.pointerDown(clipEl(chords), { button: 0, clientX: 100, clientY: 20 });
      fireEvent.pointerMove(window, { clientX: 100 + 1.5 * PX, clientY: 20, altKey: true });
      fireEvent.pointerUp(window, { clientX: 100 + 1.5 * PX, clientY: 20, altKey: true });
    });
    await flush();
    expect(startOf(project().clips[chords.id]!)).toBeCloseTo(1.5);
  });

  it("drags a clip to another compatible track", async () => {
    const chords = clipByName("Chords");
    await drag(clipEl(chords), 0, ROW, { x: 100, y: 20 });
    expect(project().clips[chords.id]!.track).toBe(trackByName("Bass").id);
    // Audio track below: MIDI clips don't go there.
    const bass = clipByName("Bassline");
    await drag(clipEl(bass), 0, ROW, { x: 100, y: ROW + 20 });
    expect(project().clips[bass.id]!.track).toBe(trackByName("Bass").id);
  });

  it("lays rows out with the automation lanes (drag targets follow)", async () => {
    const keys = trackByName("Keys");
    const slot = document.querySelector(`[data-slot="automation"][data-track="${keys.id}"]`)!;
    expect(slot.querySelector(`[data-automation-track="${keys.id}"]`)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Show automation of Keys" }));
    await flush();
    const rowEl = (id: string) => document.querySelector<HTMLElement>(`.eth-arr-row[data-track="${id}"]`)!;
    expect(rowEl(keys.id).style.height).toBe(`${ROW + AUTOMATION_BAR_HEIGHT + LANE_HEIGHT}px`);
    // Keys' row is taller now: one row pitch down lands inside Keys' own lanes, not on Bass.
    const chords = clipByName("Chords");
    await drag(clipEl(chords), 0, ROW + AUTOMATION_BAR_HEIGHT + LANE_HEIGHT, { x: 100, y: 20 });
    expect(project().clips[chords.id]!.track).toBe(trackByName("Bass").id);
    await undo();
    await drag(clipEl(chords), 0, ROW, { x: 100, y: 20 });
    expect(project().clips[chords.id]!.track).toBe(keys.id);
  });

  it("Delete in a focused automation lane without selected points keeps the selected clips", async () => {
    const chords = clipByName("Chords");
    act(() => itemSelection.getState().select("clip", [chords.id]));
    fireEvent.click(screen.getByRole("button", { name: "Show automation of Keys" }));
    await flush();
    const lane = screen.getByRole("group", { name: "Volume automation" });
    for (const key of ["Delete", "Backspace"]) {
      await act(async () => {
        fireEvent.keyDown(lane, { key });
      });
      await flush();
    }
    expect(project().clips[chords.id]).toBeDefined();
    expect(sent.some((c) => c.domain === "Clip")).toBe(false);
  });

  it("copies with cmd/ctrl held on release", async () => {
    const chords = clipByName("Chords");
    const before = Object.keys(project().clips).length;
    await drag(clipEl(chords), 16 * PX, 0, { up: { ctrlKey: true } });
    expect(Object.keys(project().clips)).toHaveLength(before + 1);
    expect(startOf(project().clips[chords.id]!)).toBe(0);
    const copies = Object.values(project().clips).filter((c) => c.name === "Chords" && startOf(c) === 16);
    expect(copies).toHaveLength(1);
    await undo();
    expect(Object.keys(project().clips)).toHaveLength(before);
  });

  it("previews a drag locally and commits once on release", async () => {
    const chords = clipByName("Chords");
    await act(async () => {
      fireEvent.pointerDown(clipEl(chords), { button: 0, clientX: 100, clientY: 20 });
      fireEvent.pointerMove(window, { clientX: 100 + 2 * PX, clientY: 20 });
    });
    expect(useArrangementUi.getState().preview?.bounds.get(chords.id)?.start).toBe(2);
    expect(sent.filter((c) => c.domain === "Clip")).toHaveLength(0);
    await act(async () => {
      fireEvent.pointerUp(window, { clientX: 100 + 2 * PX, clientY: 20 });
    });
    await flush();
    expect(sent.filter((c) => c.domain === "Clip")).toHaveLength(1);
    expect(useArrangementUi.getState().preview).toBeNull();
  });

  it("resizes the end and the start", async () => {
    const chords = clipByName("Chords");
    await drag(clipEl(chords).querySelector('[data-handle="resize-end"]')!, -4 * PX);
    expect(project().clips[chords.id]!.length).toBe(12);
    await drag(clipEl(chords).querySelector('[data-handle="resize-start"]')!, 2 * PX);
    const c = project().clips[chords.id]!;
    expect([startOf(c), c.length, c.offset]).toEqual([2, 10, 2]);
    await undo();
    expect(startOf(project().clips[chords.id]!)).toBe(0);
    expect(project().clips[chords.id]!.length).toBe(12);
  });

  it("moves a multi-selection together", async () => {
    const chords = clipByName("Chords");
    const bass = clipByName("Bassline");
    act(() => itemSelection.getState().select("clip", [chords.id, bass.id]));
    await drag(clipEl(chords), 8 * PX);
    expect(startOf(project().clips[chords.id]!)).toBe(8);
    expect(startOf(project().clips[bass.id]!)).toBe(24);
    await undo();
    expect(startOf(project().clips[bass.id]!)).toBe(16);
  });

  it("splits at the playhead, duplicates, loops and deletes from the keyboard", async () => {
    const chords = clipByName("Chords");
    const root = document.querySelector<HTMLElement>(".eth-arr")!;
    act(() => itemSelection.getState().select("clip", [chords.id]));
    act(() =>
      playheadStore.setPlayhead({ transport: { position: 6, seconds: 3, playing: false, bpm: 120 } }),
    );
    await act(async () => {
      fireEvent.keyDown(root, { key: "e", ctrlKey: true });
    });
    await flush();
    expect(project().clips[chords.id]!.length).toBe(6);
    const right = Object.values(project().clips).find((c) => c.name === "Chords" && startOf(c) === 6);
    expect(right?.offset).toBe(6);

    await act(async () => {
      fireEvent.keyDown(root, { key: "d", ctrlKey: true });
    });
    await flush();
    const dup = Object.values(project().clips).find((c) => c.name === "Chords" && startOf(c) === 6 && c.id !== right?.id);
    expect(dup?.length).toBe(6);
    // The copy is now selected.
    expect([...itemSelection.getState().selected.clip]).toEqual([dup!.id]);

    await act(async () => {
      fireEvent.keyDown(root, { key: "l", ctrlKey: true, shiftKey: true });
    });
    await flush();
    expect(project().clips[dup!.id]!.looping.enabled).toBe(true);

    await act(async () => {
      fireEvent.keyDown(root, { key: "Delete" });
    });
    await flush();
    expect(project().clips[dup!.id]).toBeUndefined();
  });

  it("toggles looping from the toolbar", async () => {
    const chords = clipByName("Chords");
    act(() => itemSelection.getState().select("clip", [chords.id]));
    fireEvent.click(screen.getByRole("button", { name: "Loop" }));
    await flush();
    expect(project().clips[chords.id]!.looping).toEqual({ enabled: true, start: 0, end: 16 });
    expect(clipEl(chords).querySelector(".eth-clip__loop")).toBeTruthy();
  });

  it("selects clips with the marquee", async () => {
    const content = screen.getByTestId("arrangement-content");
    await drag(content, 400, 150, { x: HEADER_WIDTH + 10, y: 5 });
    const sel = itemSelection.getState().selected.clip;
    expect(sel.has(clipByName("Chords").id)).toBe(true);
    expect(sel.has(clipByName("Bassline").id)).toBe(true);
    // A click on the background clears it and selects the row's track.
    await drag(content, 0, 0, { x: HEADER_WIDTH + 10, y: ROW + 5 });
    expect(itemSelection.getState().selected.clip.size).toBe(0);
    expect(useSelectionStore.getState().selectedTrack).toBe(trackByName("Bass").id);
  });

  it("opens MIDI clips in the editor on double-click", () => {
    const chords = clipByName("Chords");
    fireEvent.doubleClick(clipEl(chords));
    expect(useEditorStore.getState().clip).toBe(chords.id);
    expect(useEditorStore.getState().request).toBe(1);
  });

  it("moves the playhead to a clip's start when it is pressed while stopped", async () => {
    const bass = clipByName("Bassline");
    await drag(clipEl(bass), 0, 0, { x: 10, y: ROW + 5 });
    await waitFor(() => expect(playheadStore.getPlayhead()?.transport.position).toBe(startOf(bass)));
  });

  it("a click on empty space moves the playhead there (snapped), but not while playing", async () => {
    const content = screen.getByTestId("arrangement-content");
    await drag(content, 0, 0, { x: HEADER_WIDTH + 20.4 * PX, y: ROW * 3 + 5 });
    await waitFor(() => expect(playheadStore.getPlayhead()?.transport.position).toBe(20));

    await act(async () => {
      await mock.send(cmd("Transport", { type: "Play" }));
    });
    await waitFor(() => expect(store().transport?.playing).toBe(true));
    await drag(content, 0, 0, { x: HEADER_WIDTH + 40 * PX, y: ROW * 3 + 5 });
    await drag(clipEl(clipByName("Bassline")), 0, 0, { x: 10, y: ROW + 5 });
    expect(sent.filter((c) => c.domain === "Transport" && c.command.type === "Locate")).toHaveLength(1);
  });

  it("pressing cmd mid-drag copies the clip instead of moving it", async () => {
    const bass = clipByName("Bassline");
    const start = startOf(bass);
    await act(async () => {
      fireEvent.pointerDown(clipEl(bass), { button: 0, pointerId: 1, clientX: 10, clientY: ROW + 5 });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: 10 + 4 * PX, clientY: ROW + 5 });
    });
    expect(useArrangementUi.getState().preview?.copy).toBe(false);
    act(() => {
      fireEvent.keyDown(window, { key: "Meta", metaKey: true });
    });
    expect(useArrangementUi.getState().preview?.copy).toBe(true);
    await act(async () => {
      fireEvent.pointerUp(window, { pointerId: 1, clientX: 10 + 4 * PX, clientY: ROW + 5, metaKey: true });
    });
    await flush();
    expect(project().clips[bass.id]!.start).toBe(start);
    expect(Object.values(project().clips).some((c) => c.track === bass.track && startOf(c) === start + 4)).toBe(true);
  });

  it("renames a track inline by double-clicking its name (Enter commits, Escape cancels)", async () => {
    const keys = trackByName("Keys");
    const header = () => document.querySelector<HTMLElement>(`.eth-arr-row[data-track="${keys.id}"] .eth-arr-header__name`)!;
    fireEvent.doubleClick(header());
    const input = screen.getByRole("textbox", { name: "Track name" });
    fireEvent.change(input, { target: { value: "Piano" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await flush();
    expect(project().tracks[keys.id]!.name).toBe("Piano");

    fireEvent.doubleClick(header());
    const again = screen.getByRole("textbox", { name: "Track name" });
    fireEvent.change(again, { target: { value: "Nope" } });
    fireEvent.keyDown(again, { key: "Escape" });
    await flush();
    expect(project().tracks[keys.id]!.name).toBe("Piano");
    expect(screen.queryByRole("textbox", { name: "Track name" })).toBeNull();
  });

  it("cmd-dragging a MIDI clip onto another track copies it and leaves the original", async () => {
    const chords = clipByName("Chords");
    const bass = clipByName("Bassline");
    const count = Object.keys(project().clips).length;
    // Chords (Keys, row 0) copied straight down onto Bass (row 1), overlapping in time.
    await drag(clipEl(chords), 0, ROW, { x: 100, y: 5, up: { metaKey: true } });
    expect(Object.keys(project().clips)).toHaveLength(count + 1);
    expect(project().clips[chords.id]).toMatchObject({ track: chords.track, start: chords.start, length: chords.length });
    const copy = Object.values(project().clips).find((c) => c.track === bass.track && c.name === chords.name);
    expect(copy).toMatchObject({ start: chords.start, length: chords.length });
    expect(Object.values(project().notes).filter((n) => n.clip === copy!.id).length).toBe(
      Object.values(project().notes).filter((n) => n.clip === chords.id).length,
    );
  });

  it("a header volume fader drag is one undo step; double-click resets to 0 dB", async () => {
    const keys = trackByName("Keys");
    const fader = screen.getByRole("slider", { name: "Keys volume" });
    const before = keys.mixer.volume;
    Object.defineProperty(fader, "clientWidth", { configurable: true, value: 40 });
    await act(async () => {
      fireEvent.pointerDown(fader, { button: 0, pointerId: 1, clientX: 20 });
      fireEvent.pointerMove(fader, { pointerId: 1, clientX: 10 });
      fireEvent.pointerMove(fader, { pointerId: 1, clientX: 4 });
      fireEvent.pointerUp(fader, { pointerId: 1, clientX: 4 });
    });
    await flush();
    const lowered = trackByName("Keys").mixer.volume;
    expect(lowered).toBeLessThan(before);
    expect(fader.getAttribute("aria-valuetext")).toMatch(/dB/);
    // The drag didn't select or move the track.
    expect(useArrangementUi.getState().trackFocus).toBeNull();
    await undo();
    expect(trackByName("Keys").mixer.volume).toBeCloseTo(before);

    fireEvent.doubleClick(fader);
    await flush();
    expect(trackByName("Keys").mixer.volume).toBeCloseTo(0);
  });

  it("zoomed out, tiny clips are painted on the lane canvas and still behave like clips", async () => {
    const keys = trackByName("Keys");
    // 16 one-beat clips back to back on Keys, from beat 32.
    await act(async () => {
      for (let i = 0; i < 16; i++) {
        await mock.send(cmd("Clip", { type: "CreateMidi", id: newId(), track: keys.id, start: 32 + i, length: 1, name: null }));
      }
    });
    const lane = document.querySelector<HTMLElement>(`[data-lane="${keys.id}"]`)!;
    act(() => arrangementView.getState().setViewport({ pxPerBeat: 4, scrollBeats: 0 }));
    await act(async () => {
      await new Promise((r) => setTimeout(r, 200)); // the lanes re-render at the settled zoom
    });
    expect(lane.querySelector('[data-testid="small-clips"]')).not.toBeNull();
    const ids = Object.values(project().clips)
      .filter((c) => c.track === keys.id && c.start >= 32)
      .sort((a, b) => a.start - b.start)
      .map((c) => c.id);
    expect(ids.every((id) => !lane.querySelector(`[data-clip-id="${id}"]`))).toBe(true);

    // Press on the 9th one (beat 40.5): selects just it; dragging moves just it.
    await drag(lane, 8, 0, { x: 40.5 * 4, y: 5 });
    await flush();
    expect([...itemSelection.getState().selected.clip]).toEqual([ids[8]]);
    expect(project().clips[ids[8]!]!.start).toBe(42);
    expect(project().clips[ids[7]!]!.start).toBe(39);
  });

  it("cmd-click toggles tracks, shift-click selects a range; Delete removes them all in one step", async () => {
    const header = (name: string) => screen.getByRole("group", { name: `${name} track` });
    const [keys, bass, drums] = ["Keys", "Bass", "Drums"].map(trackByName);
    fireEvent.click(header("Keys"));
    fireEvent.click(header("Drums"), { metaKey: true });
    expect([...useArrangementUi.getState().selectedTracks].sort()).toEqual([keys!.id, drums!.id].sort());
    fireEvent.click(header("Drums"), { metaKey: true });
    expect([...useArrangementUi.getState().selectedTracks]).toEqual([keys!.id]);
    fireEvent.click(header("Drums"), { shiftKey: true });
    expect(new Set(useArrangementUi.getState().selectedTracks)).toEqual(new Set([keys!.id, bass!.id, drums!.id]));
    expect(document.querySelectorAll(".eth-arr-header--selected")).toHaveLength(3);
    const count = Object.keys(project().tracks).length;
    fireEvent.keyDown(document.querySelector('[data-feature="arrangement"]')!, { key: "Delete" });
    await flush();
    expect(Object.keys(project().tracks)).toHaveLength(count - 3);
    await undo();
    expect(Object.keys(project().tracks)).toHaveLength(count);
  });

  it("the selection box starts only over the lanes, and the header column resizes", async () => {
    const content = screen.getByTestId("arrangement-content");
    await act(async () => {
      fireEvent.pointerDown(content, { button: 0, pointerId: 1, clientX: 50, clientY: 5 });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: 400, clientY: 90 });
    });
    expect(screen.queryByTestId("marquee")).toBeNull();
    await act(async () => {
      fireEvent.pointerUp(window, { pointerId: 1, clientX: 400, clientY: 90 });
    });

    const grip = screen.getByRole("separator", { name: "Resize track headers" });
    await act(async () => {
      fireEvent.pointerDown(grip, { button: 0, clientX: HEADER_WIDTH });
      fireEvent.pointerMove(window, { clientX: HEADER_WIDTH + 60 });
      fireEvent.pointerUp(window, { clientX: HEADER_WIDTH + 60 });
    });
    expect(useArrangementUi.getState().headerWidth).toBe(HEADER_WIDTH + 60);
    expect(screen.getByRole("group", { name: "Keys track" }).style.width).toBe(`${HEADER_WIDTH + 60}px`);
    fireEvent.doubleClick(grip);
    expect(useArrangementUi.getState().headerWidth).toBe(HEADER_WIDTH);
  });

  it("reorders tracks by dragging their headers (one undo step)", async () => {
    const names = () => tracksOrdered(project()).map((t) => t.name);
    expect(names().slice(0, 3)).toEqual(["Keys", "Bass", "Drums"]);
    const drums = screen.getByRole("group", { name: "Drums track" });
    await act(async () => {
      fireEvent.pointerDown(drums, { button: 0, pointerId: 1, clientX: 20, clientY: 2 * ROW + 10 });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: 20, clientY: ROW });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: 20, clientY: 5 });
    });
    expect(screen.getByTestId("track-drop-line").style.top).toBe("0px");
    await act(async () => {
      fireEvent.pointerUp(window, { pointerId: 1, clientX: 20, clientY: 5 });
    });
    await flush();
    expect(names().slice(0, 3)).toEqual(["Drums", "Keys", "Bass"]);
    expect(screen.queryByTestId("track-drop-line")).toBeNull();
    await undo();
    expect(names().slice(0, 3)).toEqual(["Keys", "Bass", "Drums"]);
  });

  it("keeps a single selected entity: a track or clips, and Delete removes that one", async () => {
    const bass = clipByName("Bassline");
    const keys = trackByName("Keys");
    await drag(clipEl(bass), 0, 0, { x: 10, y: ROW + 5 });
    expect([...itemSelection.getState().selected.clip]).toEqual([bass.id]);

    // Selecting a track clears the clips; Delete deletes the track, not the clip.
    fireEvent.click(screen.getByRole("group", { name: "Keys track" }));
    expect(itemSelection.getState().selected.clip.size).toBe(0);
    expect(useArrangementUi.getState().trackFocus).toBe(keys.id);
    fireEvent.keyDown(document.querySelector('[data-feature="arrangement"]')!, { key: "Delete" });
    await flush();
    expect(project().tracks[keys.id]).toBeUndefined();
    expect(project().clips[bass.id]).toBeDefined();

    // Selecting a clip clears the track focus again.
    fireEvent.click(screen.getByRole("group", { name: "Drums track" }));
    await drag(clipEl(bass), 0, 0, { x: 10, y: ROW + 5 });
    expect(useArrangementUi.getState().trackFocus).toBeNull();
    expect(document.querySelector(".eth-arr-header--selected")).toBeNull();
  });

  it("a double-click on a clip's body (passed through to the lane) opens it instead of inserting", async () => {
    const chords = clipByName("Chords");
    const lane = document.querySelector<HTMLElement>(`[data-lane="${chords.track}"]`)!;
    const before = Object.keys(project().clips).length;
    const x = (startOf(chords) + 1) * PX;
    await act(async () => {
      fireEvent.pointerDown(lane, { button: 0, pointerId: 1, clientX: x, clientY: 30 });
      fireEvent.pointerUp(window, { pointerId: 1, clientX: x, clientY: 30 });
      fireEvent.pointerDown(lane, { button: 0, pointerId: 1, clientX: x, clientY: 30 });
      fireEvent.pointerUp(window, { pointerId: 1, clientX: x, clientY: 30 });
    });
    await flush();
    expect(Object.keys(project().clips)).toHaveLength(before);
    expect(useEditorStore.getState().clip).toBe(chords.id);
  });

  describe("copy / cut / paste", () => {
    const root = () => document.querySelector<HTMLElement>('[data-feature="arrangement"]')!;
    const key = (k: string) => fireEvent.keyDown(root(), { key: k, metaKey: true });
    const clipsOn = (track: string) => Object.values(project().clips).filter((c) => c.track === track);
    async function locate(position: number) {
      await act(async () => {
        await mock.send(cmd("Transport", { type: "Locate", position }));
      });
    }

    it("copies the selection and pastes it at the playhead as one undo step", async () => {
      const bass = clipByName("Bassline");
      const notes = (clip: string) => Object.values(project().notes).filter((n) => n.clip === clip).length;
      await drag(clipEl(bass), 0, 0, { x: 10, y: ROW + 5 });
      key("c");
      await locate(32);
      key("v");
      await flush();
      const pasted = clipsOn(bass.track).find((c) => startOf(c) === 32)!;
      expect(pasted).toMatchObject({ length: bass.length, name: bass.name });
      expect(notes(pasted.id)).toBe(notes(bass.id));
      expect([...itemSelection.getState().selected.clip]).toEqual([pasted.id]);
      await waitFor(() => expect(playheadStore.getPlayhead()?.transport.position).toBe(32 + bass.length));
      await undo();
      expect(project().clips[pasted.id]).toBeUndefined();
    });

    it("cut removes the clips and paste rebuilds them (notes included)", async () => {
      const bass = clipByName("Bassline");
      const noteCount = Object.values(project().notes).filter((n) => n.clip === bass.id).length;
      await drag(clipEl(bass), 0, 0, { x: 10, y: ROW + 5 });
      key("x");
      await flush();
      expect(project().clips[bass.id]).toBeUndefined();
      await locate(40);
      key("v");
      await flush();
      const pasted = clipsOn(bass.track).find((c) => startOf(c) === 40)!;
      expect(pasted).toMatchObject({ length: bass.length, offset: bass.offset, name: bass.name });
      expect(Object.values(project().notes).filter((n) => n.clip === pasted.id)).toHaveLength(noteCount);
    });

    it("handles the clipboard events the macOS Edit menu sends", async () => {
      const bass = clipByName("Bassline");
      await drag(clipEl(bass), 0, 0, { x: 10, y: ROW + 5 });
      root().focus();
      act(() => {
        document.dispatchEvent(new Event("copy", { bubbles: true, cancelable: true }));
      });
      await locate(48);
      await act(async () => {
        document.dispatchEvent(new Event("paste", { bubbles: true, cancelable: true }));
      });
      await flush();
      expect(clipsOn(bass.track).some((c) => startOf(c) === 48)).toBe(true);
    });

    it("pastes where an empty lane is right-clicked, onto that track", async () => {
      const chords = clipByName("Chords");
      await drag(clipEl(chords), 0, 0, { x: 100, y: 5 });
      key("c");
      const lane = document.querySelector<HTMLElement>(`[data-lane="${chords.track}"]`)!;
      fireEvent.contextMenu(lane, { clientX: 24.3 * PX, clientY: 20 });
      fireEvent.click(screen.getByRole("menuitem", { name: "Paste" }));
      await flush();
      expect(clipsOn(chords.track).some((c) => startOf(c) === 24)).toBe(true);
    });
  });

  it("offers Delete in the right-click menu of a clip and of a track", async () => {
    const bass = clipByName("Bassline");
    fireEvent.contextMenu(clipEl(bass), { clientX: 10, clientY: 70 });
    expect([...itemSelection.getState().selected.clip]).toEqual([bass.id]);
    fireEvent.click(screen.getByRole("menuitem", { name: "Delete" }));
    await flush();
    expect(project().clips[bass.id]).toBeUndefined();

    const keys = trackByName("Keys");
    fireEvent.contextMenu(screen.getByRole("group", { name: "Keys track" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Delete Track" }));
    await flush();
    expect(project().tracks[keys.id]).toBeUndefined();
  });

  it("resizes one track in steps by dragging its bottom edge, and resets on double-click", async () => {
    const keys = trackByName("Keys");
    const handle = screen.getByRole("separator", { name: "Resize Keys" });
    const row = () => document.querySelector<HTMLElement>(`.eth-arr-row[data-track="${keys.id}"]`)!;
    await drag(handle, 0, 21, { y: 100 });
    expect(useArrangementUi.getState().heights.get(keys.id)).toBe(TRACK_HEIGHT + 24);
    expect(row().style.height).toBe(`${TRACK_HEIGHT + 24}px`);
    await drag(handle, 0, -1000, { y: 100 });
    expect(useArrangementUi.getState().heights.get(keys.id)).toBe(MIN_TRACK_HEIGHT);
    fireEvent.doubleClick(handle);
    expect(useArrangementUi.getState().heights.has(keys.id)).toBe(false);
  });

  it("cmd+wheel zooms around the beat under the pointer (lanes start after the headers)", () => {
    const scroll = document.querySelector<HTMLElement>(".eth-arr__scroll")!;
    const x = 10 * PX; // beat 10 in the lanes
    const beatAt = () => arrangementView.getState().scrollBeats + x / arrangementView.getState().pxPerBeat;
    act(() => arrangementView.getState().scrollTo(4));
    const before = beatAt();
    act(() => {
      fireEvent.wheel(scroll, { deltaY: -100, metaKey: true, clientX: HEADER_WIDTH + x });
    });
    expect(arrangementView.getState().pxPerBeat).not.toBe(PX);
    expect(beatAt()).toBeCloseTo(before, 6);
  });

  it("scales every track with cmd+shift+wheel, within the limits", () => {
    const keys = trackByName("Keys");
    useArrangementUi.getState().setHeight(keys.id, 100);
    const scroll = document.querySelector<HTMLElement>(".eth-arr__scroll")!;
    act(() => {
      fireEvent.wheel(scroll, { deltaY: -100, metaKey: true, shiftKey: true });
    });
    const f = wheelZoomFactor(-100);
    expect(useArrangementUi.getState().defaultHeight).toBeCloseTo(TRACK_HEIGHT * f);
    expect(useArrangementUi.getState().heights.get(keys.id)).toBeCloseTo(100 * f);
    expect(arrangementView.getState().pxPerBeat).toBe(PX); // not a horizontal zoom
    act(() => {
      for (let i = 0; i < 50; i++) fireEvent.wheel(scroll, { deltaY: -200, ctrlKey: true, shiftKey: true });
    });
    expect(useArrangementUi.getState().heights.get(keys.id)).toBe(MAX_TRACK_HEIGHT);
  });

  /** Double-press on `lane` at x, then (still holding) drag to `toX` and release. */
  async function doublePressDrag(lane: Element, x: number, toX = x) {
    await act(async () => {
      fireEvent.pointerDown(lane, { button: 0, pointerId: 1, clientX: x, clientY: 5 });
      fireEvent.pointerUp(window, { pointerId: 1, clientX: x, clientY: 5 });
      fireEvent.pointerDown(lane, { button: 0, pointerId: 1, clientX: x, clientY: 5 });
    });
    if (toX !== x) {
      await act(async () => {
        fireEvent.pointerMove(window, { pointerId: 1, clientX: (x + toX) / 2, clientY: 5 });
        fireEvent.pointerMove(window, { pointerId: 1, clientX: toX, clientY: 5 });
      });
      expect(screen.getByTestId("insert-preview")).toBeInTheDocument();
    }
    await act(async () => {
      fireEvent.pointerUp(window, { pointerId: 1, clientX: toX, clientY: 5 });
    });
    await flush();
  }

  it("creates a one-bar MIDI clip on double-click in an empty MIDI lane", async () => {
    const keys = trackByName("Keys");
    const lane = document.querySelector<HTMLElement>(`[data-lane="${keys.id}"]`)!;
    const before = Object.keys(project().clips).length;
    await doublePressDrag(lane, 17.5 * PX);
    const created = Object.values(project().clips).filter((c) => c.track === keys.id && startOf(c) === 16);
    expect(Object.keys(project().clips)).toHaveLength(before + 1);
    expect(created[0]?.length).toBe(4);
  });

  it("sizes the new MIDI clip by dragging after the double-click", async () => {
    const keys = trackByName("Keys");
    const lane = document.querySelector<HTMLElement>(`[data-lane="${keys.id}"]`)!;
    const before = Object.keys(project().clips).length;
    // Press at beat 16.5, drag to beat 22.5: the 1-beat grid covers [16, 23).
    await doublePressDrag(lane, 16.5 * PX, 22.5 * PX);
    expect(Object.keys(project().clips)).toHaveLength(before + 1);
    const created = Object.values(project().clips).find((c) => c.track === keys.id && startOf(c) === 16);
    expect(created?.length).toBe(7);
    expect(screen.queryByTestId("insert-preview")).toBeNull();

    // Dragging left of the press extends the clip backwards.
    await doublePressDrag(lane, 30.5 * PX, 27.5 * PX);
    const back = Object.values(project().clips).find((c) => c.track === keys.id && startOf(c) === 27);
    expect(back?.length).toBe(4);
  });
});

describe("ArrangementView: audio and drops", () => {
  it("requests peaks for audio clip waveforms", async () => {
    await flush();
    const peaks = sent.filter((c) => c.domain === "Media" && c.command.type === "GetPeaks");
    expect(peaks.length).toBeGreaterThan(0);
    expect(screen.getAllByTestId("clip-waveform").length).toBeGreaterThan(0);
    expect(screen.getAllByTestId("clip-notes").length).toBeGreaterThan(0);
  });

  it("scrolling slides the lane layers instead of re-laying out the clips", async () => {
    const chords = clipEl(clipByName("Chords"));
    const view = arrangementView.getState();
    act(() => view.setWidth(800));
    const left = chords.style.left;
    const layer = chords.closest<HTMLElement>(".eth-arr-lane__layer")!;
    act(() => view.scrollByPx(30));
    expect(chords.style.left).toBe(left);
    expect(layer.style.transform).toBe("translateX(-30px)");
  });

  it("pans and small zooms reuse the clip canvases; they redraw once the zoom settles", async () => {
    await flush();
    const paints = vi.mocked(HTMLCanvasElement.prototype.getContext);
    const view = arrangementView.getState();
    act(() => view.setWidth(800));
    await flush();
    const base = paints.mock.calls.length;
    act(() => view.scrollByPx(12));
    act(() => view.scrollByPx(12));
    expect(paints.mock.calls.length).toBe(base);
    vi.useFakeTimers();
    try {
      act(() => view.zoomBy(1.2, 100));
      expect(paints.mock.calls.length).toBe(base);
      act(() => vi.advanceTimersByTime(200));
      expect(paints.mock.calls.length).toBeGreaterThan(base);
    } finally {
      vi.useRealTimers();
    }
  });

  /** Fire a drag event with a fake DataTransfer (jsdom has no DragEvent, so set clientX/Y by hand). */
  function fireDrag(type: "dragOver" | "drop", el: Element, payload: object, clientX: number, clientY: number) {
    const data = JSON.stringify(payload);
    const ev = createEvent[type](el, {
      dataTransfer: {
        types: [BROWSER_DRAG_MIME, "text/plain"],
        getData: (t: string) => (t === BROWSER_DRAG_MIME ? data : ""),
        dropEffect: "none",
      },
    });
    Object.defineProperty(ev, "clientX", { value: clientX });
    Object.defineProperty(ev, "clientY", { value: clientY });
    fireEvent(el, ev);
  }

  it("creates an audio clip from a browser drop on an audio track", async () => {
    const drums = trackByName("Drums");
    const content = screen.getByTestId("arrangement-content");
    const payload = {
      version: 1,
      kind: "media",
      source: { type: "Location", location: { type: "Library", id: "library" }, path: "Drums/Kick.wav" },
      name: "Kick.wav",
      file_kind: "Audio",
    };
    const y = 2 * ROW + 20;
    fireDrag("dragOver", content, payload, HEADER_WIDTH + 20 * PX, y);
    expect(useArrangementUi.getState().dropHint).toEqual({ track: drums.id, at: 20 });
    fireDrag("drop", content, payload, HEADER_WIDTH + 20.2 * PX, y);
    await flush();
    const created = Object.values(project().clips).find((c) => c.track === drums.id && startOf(c) === 20);
    expect(created?.content.type).toBe("Audio");
    expect(created?.name).toBe("Kick");
    expect(useArrangementUi.getState().dropHint).toBeNull();
  });

  it("shows pending imports in their lane or in the drop area", () => {
    const drums = trackByName("Drums");
    const base = { at: 2, name: "Pad.mp3", progress: 0.25, error: null };
    act(() => {
      useArrangementUi.getState().putImport({ id: "a", track: drums.id, ...base });
      useArrangementUi.getState().putImport({ id: "b", track: null, ...base, error: "timed out" });
    });
    const lane = document.querySelector(`[data-lane="${drums.id}"]`)!;
    expect(lane.querySelector('[data-testid="import-placeholder"]')?.textContent).toBe("Importing Pad.mp3… 25%");
    expect(screen.getByText("Import failed: Pad.mp3")).toBeTruthy();
    act(() => useArrangementUi.getState().removeImport("a"));
    expect(lane.querySelector('[data-testid="import-placeholder"]')).toBeNull();
  });

  it("rejects drops on MIDI tracks and creates a track below the last one", async () => {
    const media = Object.values(project().media)[0]!;
    const payload = { version: 1, kind: "media", source: { type: "Project", media: media.id }, name: media.name, file_kind: "Audio" };
    const content = screen.getByTestId("arrangement-content");
    const idsBefore = new Set(Object.keys(project().tracks));
    const tracksBefore = idsBefore.size;
    fireDrag("drop", content, payload, HEADER_WIDTH + 10, 10);
    await flush();
    expect(Object.keys(project().tracks)).toHaveLength(tracksBefore);

    fireDrag("drop", content, payload, HEADER_WIDTH + 4 * PX, 5 * ROW + 20);
    await flush();
    expect(Object.keys(project().tracks)).toHaveLength(tracksBefore + 1);
    const t = Object.values(project().tracks).find((x) => !idsBefore.has(x.id))!;
    expect(t.kind).toBe("Audio");
    const clip = Object.values(project().clips).find((c) => c.track === t.id);
    expect(clip && startOf(clip)).toBe(4);
    // Track + clip are one undo step (media was already in the project).
    await undo();
    expect(Object.keys(project().tracks)).toHaveLength(tracksBefore);
  });
});

describe("ArrangementView: loop region", () => {
  it("shades the project loop region when enabled", async () => {
    expect(screen.queryByTestId("loop-region")).toBeNull();
    await act(async () => {
      await mock.send(cmd("Transport", { type: "SetLoopRegion", region: { start: 4, end: 8 } }));
      await mock.send(cmd("Transport", { type: "SetLoopEnabled", enabled: true }));
    });
    await flush();
    const loop = screen.getByTestId("loop-region");
    expect(loop.style.left).toBe(`${4 * PX}px`);
    expect(loop.style.width).toBe(`${4 * PX}px`);
  });
});
