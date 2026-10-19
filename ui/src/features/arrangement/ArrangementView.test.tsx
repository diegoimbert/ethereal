import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, createEvent, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Clip, Command, Track } from "@/generated";
import { playheadStore, tracksOrdered, useEditorStore, useProjectStore, useSelectionStore } from "@/state";
import { itemSelection, wheelZoomFactor } from "@/timeline";
import { cmd, MockTransport, newId, TransportProvider } from "@/transport";
import { ContextMenuHost } from "@/kit";
import { pickOption } from "@/kit/testing";
import { ArrangementView } from "./ArrangementView";
import { clipMenu } from "./actions";
import { BROWSER_DRAG_MIME, BROWSER_MIDI_DRAG_MIME } from "./browserDrop";
import { clearClipboard } from "./clipboard";
import { flushDraws } from "./drawQueue";
import { resetTimeSelection, useTimeSelection } from "@/features/time-edits/store";
import { AUTOMATION_BAR_HEIGHT, LANE_HEIGHT, resetAutomationUi } from "@/features/automation";
import { HEADER_WIDTH, MAX_TRACK_HEIGHT, MIN_TRACK_HEIGHT, TRACK_HEIGHT } from "./layout";
import { glRenderer } from "./gl/active";
import { formatDb } from "@/domain/paramScale";
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

/** The arranger's lanes: the canvas hit-tests presses on them (no element per clip). */
const lanes = () => screen.getByTestId("arrangement-content");
type ClipPartName = "header" | "body" | "resize-start" | "resize-end";
/** The middle of a clip part in client px, from the arranger's test API (jsdom lays nothing out). */
function partAt(c: Clip, part: ClipPartName = "header"): { x: number; y: number } {
  const api = (
    window as unknown as { __ether: { arrangement: { handleRect(id: string, part: string): { x: number; y: number; width: number; height: number } | null } } }
  ).__ether.arrangement;
  const r = api.handleRect(c.id, part);
  if (!r) throw new Error(`no ${part} for ${c.name}`);
  return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
}
/** Drags a clip by its title band (or another part) by (dx, dy) px. */
async function dragClip(c: Clip, dx: number, dy = 0, opts: { part?: ClipPartName; up?: PointerEventInit } = {}) {
  const at = partAt(project().clips[c.id] ?? c, opts.part);
  await drag(lanes(), dx, dy, { x: at.x, y: at.y, up: opts.up });
}

async function flush() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

/**
 * jsdom has no canvas (the arranger's Canvas2D fallback draws nothing but still hit-tests
 * and runs the gestures) and lays nothing out: the lanes and the arranger canvas start after
 * the header column, as in a browser.
 */
function stubCanvas() {
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  const rect = Element.prototype.getBoundingClientRect;
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    if (!this.matches(".eth-arr-lane, .eth-arr-gl__canvas")) return rect.call(this);
    const left = useArrangementUi.getState().headerWidth;
    return { x: left, y: 0, left, top: 0, right: left, bottom: 0, width: 0, height: 0, toJSON: () => ({}) };
  });
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
  resetTimeSelection();
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
    expect(names).toEqual(["Keys", "Bass", "Drums", "A Delay", "master"]);
    // Master is pinned below the scrolling tracks, outside the scroll content.
    const master = screen.getByTestId("arrangement-master");
    expect(master.querySelector(".eth-arr-header__name")?.textContent).toBe("master");
    expect(screen.getByTestId("arrangement-content").textContent).not.toContain("master");
    expect(document.querySelectorAll('[data-slot="automation"]')).toHaveLength(5);
  });

  it("lists each track's clips as text for screen readers (the canvas draws them)", () => {
    const summary = within(screen.getByRole("list", { name: "clips" }));
    const clips = Object.values(project().clips).filter((c) => c.lane == null);
    expect(clips.length).toBeGreaterThan(0);
    for (const track of tracksOrdered(project())) {
      const own = clips.filter((c) => c.track === track.id);
      if (own.length === 0) continue;
      const list = summary.getByRole("list", { name: `${track.name}: ${own.length === 1 ? "1 clip" : `${own.length} clips`}` });
      expect(within(list).getAllByRole("listitem")).toHaveLength(own.length);
    }
    const first = clips[0]!;
    expect(summary.getAllByRole("listitem").some((li) => li.textContent?.includes(first.name) && li.textContent.includes(" to "))).toBe(true);
  });

  it("selects the track on header click", () => {
    fireEvent.click(screen.getByRole("group", { name: "Bass track" }));
    expect(useSelectionStore.getState().selectedTrack).toBe(trackByName("Bass").id);
  });

  it("mutes, solos and arms with the existing commands", async () => {
    fireEvent.click(screen.getByRole("button", { name: "mute Keys" }));
    await flush();
    expect(trackByName("Keys").mixer.mute).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "solo Bass" }));
    await flush();
    expect(trackByName("Bass").mixer.solo).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "arm Drums" }));
    await flush();
    expect(store().armedTracks).toEqual([trackByName("Drums").id]);
    expect(screen.queryByRole("button", { name: "arm Master" })).toBeNull();
    expect(screen.queryByRole("button", { name: "solo Master" })).toBeNull();
  });

  it("exclusive solo: plain click solos only that track, Cmd/Ctrl adds, the only soloed one unsolos", async () => {
    const soloed = () =>
      tracksOrdered(project())
        .filter((t) => t.mixer.solo)
        .map((t) => t.name);
    const click = async (name: string, mods: { metaKey?: boolean; ctrlKey?: boolean } = {}) => {
      fireEvent.click(screen.getByRole("button", { name: `solo ${name}` }), mods);
      await flush();
    };
    await click("Keys", { metaKey: true });
    await click("Bass", { ctrlKey: true });
    await click("Drums", { metaKey: true });
    expect(soloed()).toEqual(["Keys", "Bass", "Drums"]);
    await click("Bass");
    expect(soloed()).toEqual(["Bass"]);
    await click("Drums");
    expect(soloed()).toEqual(["Drums"]);
    await click("Drums");
    expect(soloed()).toEqual([]);
  });

  it("add track opens a menu of the kinds; dragging one between rows creates it at that spot (no draft row)", async () => {
    const names = () => tracksOrdered(project()).map((t) => t.name);
    fireEvent.click(screen.getByRole("button", { name: "add track" }));
    const item = screen.getByRole("menuitem", { name: "create midi track" });
    // jsdom has no layout: give the lanes a box so the pointer is "over" them.
    vi.spyOn(screen.getByTestId("arrangement-content"), "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 1200, 800));
    await act(async () => {
      fireEvent.pointerDown(item, { button: 0, pointerId: 1, clientX: 10, clientY: -20 });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: 300, clientY: ROW });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: 300, clientY: ROW + 5 });
    });
    expect(screen.getByTestId("track-drop-line").style.top).toBe(`${ROW}px`);
    // The menu got out of the way of the drop.
    expect(screen.queryByRole("menu", { name: "add track" })).toBeNull();
    await act(async () => {
      fireEvent.pointerUp(window, { pointerId: 1, clientX: 300, clientY: ROW + 5 });
    });
    await flush();
    expect(names().slice(0, 3)).toEqual(["Keys", expect.stringMatching(/midi/), "Bass"]);
    expect(document.querySelector('[data-track="__draft_track__"]')).toBeNull();
    // A dragged MIDI track offers its instruments right away; a pick replaces its Synth.
    const midi = tracksOrdered(project())[1]!;
    const picker = await screen.findByRole("dialog", { name: `choose an instrument for ${midi.name}` });
    // A real press then click (the header under the portal must not take it).
    const row = within(picker).getByRole("button", { name: "sampler" });
    fireEvent.pointerDown(row, { button: 0, pointerId: 2 });
    fireEvent.pointerUp(row, { button: 0, pointerId: 2 });
    fireEvent.click(row);
    await flush();
    expect(screen.queryByRole("dialog", { name: `choose an instrument for ${midi.name}` })).toBeNull();
    expect(
      Object.values(project().devices)
        .filter((d) => d.track === midi.id)
        .map((d) => d.kind),
    ).toEqual([{ type: "Builtin", device: { type: "Sampler", sample: null, slices: expect.anything() } }]);
  });

  it("adds a MIDI track with the built-in synth and an audio track from the menu, each one undo step", async () => {
    const before = Object.keys(project().tracks).length;
    fireEvent.click(screen.getByRole("button", { name: "add track" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "create midi track" }));
    await flush();
    const midi = Object.values(project().tracks)
      .filter((t) => t.kind === "Midi")
      .at(-1)!;
    expect(Object.keys(project().tracks)).toHaveLength(before + 1);
    expect(useSelectionStore.getState().selectedTrack).toBe(midi.id);
    const devices = Object.values(project().devices).filter((d) => d.track === midi.id);
    expect(devices.map((d) => d.kind)).toEqual([{ type: "Builtin", device: { type: "Synth" } }]);

    fireEvent.click(screen.getByRole("button", { name: "add track" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "create audio track" }));
    await flush();
    expect(Object.keys(project().tracks)).toHaveLength(before + 2);
    await undo();
    await undo();
    expect(Object.keys(project().tracks)).toHaveLength(before);
    expect(Object.values(project().devices).filter((d) => d.track === midi.id)).toEqual([]);
  });

  it("clicking Add track > MIDI track opens the instrument picker for the new track, and a pick lands on it", async () => {
    fireEvent.click(screen.getByRole("button", { name: "add track" }));
    const item = screen.getByRole("menuitem", { name: "create midi track" });
    // A real click: press, release, click (the press must not close what the click opens).
    fireEvent.pointerDown(item, { button: 0, pointerId: 1, clientX: 10, clientY: 10 });
    fireEvent.pointerUp(window, { pointerId: 1, clientX: 10, clientY: 10 });
    fireEvent.click(item);
    await flush();
    const midi = Object.values(project().tracks)
      .filter((t) => t.kind === "Midi")
      .at(-1)!;
    const picker = await screen.findByRole("dialog", { name: `choose an instrument for ${midi.name}` });
    const row = within(picker).getByRole("button", { name: "sampler" });
    fireEvent.pointerDown(row, { button: 0, pointerId: 2 });
    fireEvent.pointerUp(row, { button: 0, pointerId: 2 });
    fireEvent.click(row);
    await flush();
    expect(screen.queryByRole("dialog", { name: `choose an instrument for ${midi.name}` })).toBeNull();
    expect(
      Object.values(project().devices)
        .filter((d) => d.track === midi.id)
        .map((d) => d.kind),
    ).toEqual([{ type: "Builtin", device: { type: "Sampler", sample: null, slices: expect.anything() } }]);
  });

  it("Escape closes the instrument picker of a clicked-in MIDI track (the synth stays)", async () => {
    fireEvent.click(screen.getByRole("button", { name: "add track" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "create midi track" }));
    await flush();
    const midi = Object.values(project().tracks)
      .filter((t) => t.kind === "Midi")
      .at(-1)!;
    const picker = await screen.findByRole("dialog", { name: `choose an instrument for ${midi.name}` });
    fireEvent.keyDown(within(picker).getByRole("button", { name: "sampler" }), { key: "Escape" });
    await flush();
    expect(screen.queryByRole("dialog", { name: `choose an instrument for ${midi.name}` })).toBeNull();
    expect(
      Object.values(project().devices)
        .filter((d) => d.track === midi.id)
        .map((d) => d.kind),
    ).toEqual([{ type: "Builtin", device: { type: "Synth" } }]);
  });

  it("adds a track from the + Add track row below the tracks (the same menu) and from a right-click on empty space", async () => {
    const before = Object.keys(project().tracks).length;
    fireEvent.click(screen.getByRole("button", { name: "add a track at the end" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "create audio track" }));
    await flush();
    expect(Object.keys(project().tracks)).toHaveLength(before + 1);
    expect(screen.getByRole("button", { name: "add a track at the end" })).toBeInTheDocument();

    // Empty space below the tracks: the menu offers both kinds and creates the track at once.
    fireEvent.contextMenu(screen.getByTestId("arrangement-content"), { clientX: 20, clientY: 100_000 });
    expect(screen.getByRole("menuitem", { name: "add audio track" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("menuitem", { name: "add midi track" }));
    await flush();
    const midi = Object.values(project().tracks)
      .filter((t) => t.kind === "Midi")
      .at(-1)!;
    expect(Object.keys(project().tracks)).toHaveLength(before + 2);
    expect(useSelectionStore.getState().selectedTrack).toBe(midi.id);

    // A track's own menu has no audio/MIDI add items, only "add return track".
    fireEvent.contextMenu(screen.getByRole("group", { name: "Keys track" }));
    expect(screen.queryByRole("menuitem", { name: "add audio track" })).toBeNull();
    expect(screen.getByRole("menuitem", { name: "add return track" })).toBeInTheDocument();
  });

  it("adds a return track from the add menu, a track's menu and empty space (top-level, before master, one undo step each)", async () => {
    const returns = () => Object.values(project().tracks).filter((t) => t.kind === "Return");
    const before = returns().length;
    fireEvent.click(screen.getByRole("button", { name: "add a track at the end" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "create return track" }));
    await flush();
    expect(returns()).toHaveLength(before + 1);
    const added = returns().find((t) => t.name !== "A Delay")!;
    expect(added.parent).toBeNull();
    // After the existing return, before master.
    const order = Object.values(project().tracks)
      .filter((t) => t.parent === null)
      .sort((a, b) => (a.order < b.order ? -1 : 1))
      .map((t) => t.kind);
    expect(order.slice(-3)).toEqual(["Return", "Return", "Master"]);
    expect(useSelectionStore.getState().selectedTrack).toBe(added.id);

    fireEvent.contextMenu(screen.getByRole("group", { name: "Keys track" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "add return track" }));
    await flush();
    expect(returns()).toHaveLength(before + 2);

    fireEvent.contextMenu(screen.getByTestId("arrangement-content"), { clientX: 20, clientY: 100_000 });
    fireEvent.click(screen.getByRole("menuitem", { name: "add return track" }));
    await flush();
    expect(returns()).toHaveLength(before + 3);

    await act(async () => void (await mock.send(cmd("Edit", { type: "Undo" }))));
    await flush();
    expect(returns()).toHaveLength(before + 2);
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
    fireEvent.click(screen.getByRole("button", { name: "fold Grp" }));
    expect(screen.queryByRole("group", { name: "Bass track" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "unfold Grp" }));
    expect(screen.getByRole("group", { name: "Bass track" })).toBeTruthy();
  });
});

/** The rows the renderer draws (its row model: what hit testing and drawing read). */
const rowsOf = (r: object) => (r as { deps: { rows(): unknown[] } }).deps.rows();

describe("ArrangementView: track header polish", () => {
  const headerOf = (name: string) => screen.getByRole("group", { name: `${name} track` });
  const buttonLabels = (name: string) => [...headerOf(name).querySelectorAll(".eth-arr-header__buttons > button")].map((b) => b.getAttribute("aria-label"));

  it("puts the takes button first, so the other buttons keep their positions", async () => {
    const bass = trackByName("Bass");
    const before = buttonLabels("Bass");
    expect(before).toEqual(["mute Bass", "solo Bass", "arm Bass", "show automation of Bass"].slice(0, before.length));
    await act(async () => {
      await mock.send(cmd("Take", { type: "CreateLane", id: newId(), track: bass.id, name: null, before: null }));
    });
    await flush();
    const after = buttonLabels("Bass");
    expect(after[0]).toMatch(/takes of Bass/);
    expect(after.slice(1)).toEqual(before);
  });

  it("header buttons never move: each keeps its slot, and hover / state only fade them in", () => {
    const css = readFileSync(resolve(__dirname, "arrangement.css"), "utf8");
    const rule = (sel: string) => css.split("}").find((b) => b.includes(sel)) ?? "";
    const base = rule(".eth-arr-header__toggle {");
    expect(base).toMatch(/width: var\(--eth-size-track-button\)/);
    expect(base).toMatch(/opacity: 0;/);
    expect(base).not.toMatch(/transition:[^;]*\bwidth\b/);
    const shown = rule(".eth-arr-header:hover .eth-arr-header__toggle");
    for (const sel of ['[aria-pressed="true"]', ".eth-takes-toggle", ".eth-freeze__cancel", ".eth-auto-toggle--has"]) {
      expect(shown).toContain(sel);
    }
    // Showing a button changes nothing about the layout.
    expect(shown).toMatch(/opacity: 1;/);
    expect(shown).not.toMatch(/\b(width|margin)/);
  });

  it("a selected clip gives its track's header the hover look, without selecting the track", async () => {
    const chords = clipByName("Chords");
    const keys = headerOf(trackByName("Keys").name);
    expect(keys.hasAttribute("data-clip-selected")).toBe(false);
    act(() => itemSelection.getState().select("clip", [chords.id]));
    expect(keys.getAttribute("data-clip-selected")).toBe("true");
    expect(keys.className).not.toContain("eth-arr-header--selected");
    act(() => itemSelection.getState().clear());
    expect(keys.hasAttribute("data-clip-selected")).toBe(false);
  });

  it("folding a group redraws the lanes at once (no other input)", async () => {
    const group = newId();
    const child = trackByName("Bass").id;
    await act(async () => {
      await mock.send(cmd("Track", { type: "Create", id: group, kind: "Group", name: "Grp", color: null, parent: null, before: null }));
      await mock.send(cmd("Track", { type: "Move", id: child, parent: group, before: null }));
    });
    await flush();
    const r = glRenderer()!;
    expect(r).toBeTruthy();
    const spy = vi.spyOn(r, "requestRender");
    const rowsBefore = rowsOf(r).length;
    fireEvent.click(screen.getByRole("button", { name: "fold Grp" }));
    await flush();
    expect(rowsOf(r).length).toBe(rowsBefore - 1);
    expect(spy).toHaveBeenCalledWith(true);
    spy.mockClear();
    fireEvent.click(screen.getByRole("button", { name: "unfold Grp" }));
    await flush();
    expect(rowsOf(r).length).toBe(rowsBefore);
    expect(spy).toHaveBeenCalledWith(true);
  });
});

describe("ArrangementView: clip editing", () => {
  it("click selects a clip and its track; shift adds; cmd toggles", async () => {
    const chords = clipByName("Chords");
    const bass = clipByName("Bassline");
    await dragClip(chords, 0);
    expect([...itemSelection.getState().selected.clip]).toEqual([chords.id]);
    expect(useSelectionStore.getState().selectedTrack).toBe(chords.track);
    const b = partAt(bass);
    await act(async () => {
      fireEvent.pointerDown(lanes(), { button: 0, clientX: b.x, clientY: b.y, shiftKey: true });
      fireEvent.pointerUp(window, { clientX: b.x, clientY: b.y });
    });
    expect(itemSelection.getState().selected.clip.size).toBe(2);
    await act(async () => {
      fireEvent.pointerDown(lanes(), { button: 0, clientX: b.x, clientY: b.y, metaKey: true });
      fireEvent.pointerUp(window, { clientX: b.x, clientY: b.y, metaKey: true });
    });
    expect([...itemSelection.getState().selected.clip]).toEqual([chords.id]);
  });

  it("moves a clip with snapping as one undo step", async () => {
    const chords = clipByName("Chords");
    await dragClip(chords, 4.3 * PX);
    expect(startOf(project().clips[chords.id]!)).toBe(4);
    expect(store().history.can_undo).toBe(true);
    await undo();
    expect(startOf(project().clips[chords.id]!)).toBe(0);
  });

  it("bypasses snapping with alt", async () => {
    const chords = clipByName("Chords");
    await dragClip(chords, 1.5 * PX);
    expect(startOf(project().clips[chords.id]!)).toBe(2);
    await undo();
    const at = partAt(chords);
    await act(async () => {
      fireEvent.pointerDown(lanes(), { button: 0, clientX: at.x, clientY: at.y });
      fireEvent.pointerMove(window, { clientX: at.x + 1.5 * PX, clientY: at.y, altKey: true });
      fireEvent.pointerUp(window, { clientX: at.x + 1.5 * PX, clientY: at.y, altKey: true });
    });
    await flush();
    expect(startOf(project().clips[chords.id]!)).toBeCloseTo(1.5);
  });

  it("drags a clip to another compatible track", async () => {
    const chords = clipByName("Chords");
    await dragClip(chords, 0, ROW);
    expect(project().clips[chords.id]!.track).toBe(trackByName("Bass").id);
    // Audio track below: MIDI clips don't go there.
    const bass = clipByName("Bassline");
    await dragClip(bass, 0, ROW);
    expect(project().clips[bass.id]!.track).toBe(trackByName("Bass").id);
  });

  it("lays rows out with the automation lanes (drag targets follow)", async () => {
    const keys = trackByName("Keys");
    const slot = document.querySelector(`[data-slot="automation"][data-track="${keys.id}"]`)!;
    expect(slot.querySelector(`[data-automation-track="${keys.id}"]`)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "show automation of Keys" }));
    await flush();
    const rowEl = (id: string) => document.querySelector<HTMLElement>(`.eth-arr-row[data-track="${id}"]`)!;
    expect(rowEl(keys.id).style.height).toBe(`${ROW + AUTOMATION_BAR_HEIGHT + LANE_HEIGHT}px`);
    // Keys' row is taller now: one row pitch down lands inside Keys' own lanes, not on Bass.
    const chords = clipByName("Chords");
    await dragClip(chords, 0, ROW + AUTOMATION_BAR_HEIGHT + LANE_HEIGHT);
    expect(project().clips[chords.id]!.track).toBe(trackByName("Bass").id);
    await undo();
    await dragClip(chords, 0, ROW);
    expect(project().clips[chords.id]!.track).toBe(keys.id);
  });

  it("Delete in a focused automation lane without selected points keeps the selected clips", async () => {
    const chords = clipByName("Chords");
    act(() => itemSelection.getState().select("clip", [chords.id]));
    fireEvent.click(screen.getByRole("button", { name: "show automation of Keys" }));
    await flush();
    const lane = screen.getByRole("group", { name: "volume automation" });
    for (const key of ["Delete", "Backspace"]) {
      await act(async () => {
        fireEvent.keyDown(lane, { key });
      });
      await flush();
    }
    expect(project().clips[chords.id]).toBeDefined();
    expect(sent.some((c) => c.domain === "Clip")).toBe(false);
  });

  it("edits the tempo map on the master's Tempo lane, its first automation lane", async () => {
    const master = screen.getByTestId("arrangement-master");
    const row = master.querySelector<HTMLElement>(".eth-arr-row")!;
    const closed = parseFloat(row.style.height);
    expect(screen.queryByTestId("tempo-automation-lane")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "show automation of master" }));
    await flush();
    const lane = within(master).getByTestId("tempo-automation-lane");
    expect(master.querySelector("[data-target]")).toBe(lane);
    expect(lane.textContent).toContain("tempo");
    expect(within(lane).getByTestId("playhead-bpm").textContent).toBe("120 bpm");
    // The pinned master row grows by the automation bar and the tempo lane.
    expect(parseFloat(row.style.height)).toBe(closed + AUTOMATION_BAR_HEIGHT + LANE_HEIGHT);

    // Double-click the lane: a point on the grid (jsdom: the lane starts at x 0, 24 px per beat).
    const before = Object.keys(project().tempo_points).length;
    fireEvent.doubleClick(within(lane).getByTestId("tempo-lane-svg"), { clientX: 8 * PX + 3, clientY: 10 });
    await flush();
    const added = Object.values(project().tempo_points).filter((p) => p.time !== 0);
    expect(Object.keys(project().tempo_points)).toHaveLength(before + 1);
    expect(added[0]!.time).toBe(8);

    // Its × hides it (the master's row shrinks back to the bar); "+ Parameter…" shows it again.
    fireEvent.click(within(lane).getByRole("button", { name: "hide lane" }));
    await flush();
    await waitFor(() => expect(screen.queryByTestId("tempo-automation-lane")).toBeNull());
    expect(parseFloat(row.style.height)).toBe(closed + AUTOMATION_BAR_HEIGHT);
    pickOption(within(master).getByRole("combobox", { name: "show parameter" }), { value: "tempo" });
    await flush();
    expect(within(master).getByTestId("tempo-automation-lane")).toBeTruthy();

    // Hiding the master's automation hides it.
    fireEvent.click(screen.getByRole("button", { name: "hide automation of master" }));
    await flush();
    await waitFor(() => expect(screen.queryByTestId("tempo-automation-lane")).toBeNull());
    expect(parseFloat(row.style.height)).toBe(closed);
  });

  it("copies with cmd/ctrl held on release", async () => {
    const chords = clipByName("Chords");
    const before = Object.keys(project().clips).length;
    await dragClip(chords, 16 * PX, 0, { up: { ctrlKey: true } });
    expect(Object.keys(project().clips)).toHaveLength(before + 1);
    expect(startOf(project().clips[chords.id]!)).toBe(0);
    const copies = Object.values(project().clips).filter((c) => c.name === "Chords" && startOf(c) === 16);
    expect(copies).toHaveLength(1);
    await undo();
    expect(Object.keys(project().clips)).toHaveLength(before);
  });

  it("previews a drag locally and commits once on release", async () => {
    const chords = clipByName("Chords");
    const at = partAt(chords);
    await act(async () => {
      fireEvent.pointerDown(lanes(), { button: 0, clientX: at.x, clientY: at.y });
      fireEvent.pointerMove(window, { clientX: at.x + 2 * PX, clientY: at.y });
    });
    expect(useArrangementUi.getState().preview?.bounds.get(chords.id)?.start).toBe(2);
    expect(sent.filter((c) => c.domain === "Clip")).toHaveLength(0);
    await act(async () => {
      fireEvent.pointerUp(window, { clientX: at.x + 2 * PX, clientY: at.y });
    });
    await flush();
    expect(sent.filter((c) => c.domain === "Clip")).toHaveLength(1);
    expect(useArrangementUi.getState().preview).toBeNull();
  });

  it("keeps the dragged clip under the pointer when the view scrolls or zooms mid-drag (#592)", async () => {
    const chords = clipByName("Chords");
    const at = partAt(chords);
    const previewStart = () => useArrangementUi.getState().preview?.bounds.get(chords.id)?.start;
    await act(async () => {
      fireEvent.pointerDown(lanes(), { button: 0, clientX: at.x, clientY: at.y });
      fireEvent.pointerMove(window, { clientX: at.x + 2 * PX, clientY: at.y });
    });
    expect(previewStart()).toBe(2);
    // The view pages 16 beats (follow playhead) under a still pointer: the clip follows at once.
    act(() => arrangementView.getState().scrollTo(16));
    expect(previewStart()).toBe(18);
    // ...and a move to the same spot keeps it there.
    await act(async () => {
      fireEvent.pointerMove(window, { clientX: at.x + 2 * PX, clientY: at.y });
    });
    expect(previewStart()).toBe(18);
    // Zooming in 2× around the left edge: the beat under the pointer, and the clip, follow.
    act(() => arrangementView.setState({ pxPerBeat: 2 * PX }));
    const pressBeat = (at.x - useArrangementUi.getState().headerWidth) / PX;
    const pointerBeat = 16 + (at.x + 2 * PX - useArrangementUi.getState().headerWidth) / (2 * PX);
    await act(async () => {
      fireEvent.pointerMove(window, { clientX: at.x + 2 * PX, clientY: at.y });
    });
    expect(previewStart()).toBeCloseTo(Math.round(pointerBeat - pressBeat), 6);
    await act(async () => {
      fireEvent.pointerUp(window, { clientX: at.x + 2 * PX, clientY: at.y });
    });
    await flush();
    expect(startOf(project().clips[chords.id]!)).toBeCloseTo(Math.round(pointerBeat - pressBeat), 6);
  });

  it("Escape cancels a clip drag: nothing sent, preview cleared, the clip stays selected (#593)", async () => {
    const chords = clipByName("Chords");
    for (const part of ["header", "resize-end"] as const) {
      const p = partAt(project().clips[chords.id]!, part);
      sent.length = 0;
      await act(async () => {
        fireEvent.pointerDown(lanes(), { button: 0, clientX: p.x, clientY: p.y });
        fireEvent.pointerMove(window, { clientX: p.x + 20, clientY: p.y });
      });
      expect(useArrangementUi.getState().preview).not.toBeNull();
      await act(async () => {
        fireEvent.keyDown(document.activeElement ?? lanes(), { key: "Escape" });
      });
      expect(useArrangementUi.getState().preview).toBeNull();
      await act(async () => {
        fireEvent.pointerUp(window, { clientX: p.x + 20, clientY: p.y });
      });
      await flush();
      expect(
        sent.filter((c) => c.domain === "Clip"),
        part,
      ).toEqual([]);
      expect(itemSelection.getState().isSelected("clip", chords.id), part).toBe(true);
      expect(startOf(project().clips[chords.id]!)).toBe(0);
    }
  });

  it("edit keys are ignored while a clip is dragged: Delete doesn't delete it under the drag (#593)", async () => {
    const chords = clipByName("Chords");
    const at = partAt(chords);
    await act(async () => {
      fireEvent.pointerDown(lanes(), { button: 0, clientX: at.x, clientY: at.y });
      fireEvent.pointerMove(window, { clientX: at.x + 2 * PX, clientY: at.y });
    });
    await act(async () => {
      fireEvent.keyDown(document.activeElement ?? lanes(), { key: "Delete" });
      fireEvent.keyDown(document.activeElement ?? lanes(), { key: "Backspace" });
    });
    await flush();
    expect(project().clips[chords.id]).toBeDefined();
    await act(async () => {
      fireEvent.pointerUp(window, { clientX: at.x + 2 * PX, clientY: at.y });
    });
    await flush();
    expect(startOf(project().clips[chords.id]!)).toBe(2);
    // After the drop the keys work again.
    await act(async () => {
      fireEvent.keyDown(document.activeElement ?? lanes(), { key: "Delete" });
    });
    await flush();
    expect(project().clips[chords.id]).toBeUndefined();
  });

  it("resizes the end and the start", async () => {
    const chords = clipByName("Chords");
    await dragClip(chords, -4 * PX, 0, { part: "resize-end" });
    expect(project().clips[chords.id]!.length).toBe(12);
    await dragClip(chords, 2 * PX, 0, { part: "resize-start" });
    const c = project().clips[chords.id]!;
    expect([startOf(c), c.length, c.offset]).toEqual([2, 10, 2]);
    await undo();
    expect(startOf(project().clips[chords.id]!)).toBe(0);
    expect(project().clips[chords.id]!.length).toBe(12);
  });

  it("extends a clip left past its start: a gap when unlooped, wrapped loop content when looped", async () => {
    const bass = clipByName("Bassline");
    // Unlooped: 4 beats of empty space before the content; the notes keep their song time.
    await dragClip(bass, -4 * PX, 0, { part: "resize-start" });
    let c = project().clips[bass.id]!;
    expect([startOf(c), c.length, c.offset]).toEqual([12, 20, -4]);
    // Dragging back right trims the gap first.
    await dragClip(c, 2 * PX, 0, { part: "resize-start" });
    c = project().clips[bass.id]!;
    expect([startOf(c), c.length, c.offset]).toEqual([14, 18, -2]);
    await undo();
    await undo();
    expect(project().clips[bass.id]!.offset).toBe(0);

    // Looped (loop = the clip's 16 beats): extending left by 4 starts in the loop's tail.
    await act(async () => {
      await mock.send(cmd("Clip", { type: "SetLoop", id: bass.id, looping: { enabled: true, start: 0, end: 16 } }));
    });
    await flush();
    await dragClip(bass, -4 * PX, 0, { part: "resize-start" });
    c = project().clips[bass.id]!;
    expect([startOf(c), c.length, c.offset]).toEqual([12, 20, 12]);
    expect(c.looping).toEqual({ enabled: true, start: 0, end: 16 });
  });

  it("moves a multi-selection together", async () => {
    const chords = clipByName("Chords");
    const bass = clipByName("Bassline");
    act(() => itemSelection.getState().select("clip", [chords.id, bass.id]));
    await dragClip(chords, 8 * PX);
    expect(startOf(project().clips[chords.id]!)).toBe(8);
    expect(startOf(project().clips[bass.id]!)).toBe(24);
    await undo();
    expect(startOf(project().clips[bass.id]!)).toBe(16);
  });

  it("splits at the selection cursor (not the playhead), duplicates, loops and deletes from the keyboard", async () => {
    const chords = clipByName("Chords");
    const root = document.querySelector<HTMLElement>(".eth-arr")!;
    act(() => itemSelection.getState().select("clip", [chords.id]));
    act(() => useTimeSelection.setState({ cursor: 6 }));
    act(() => playheadStore.setPlayhead({ transport: { position: 10, seconds: 5, playing: false, bpm: 120 } }));
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

  it("toggles looping with cmd+shift+L", async () => {
    const chords = clipByName("Chords");
    act(() => itemSelection.getState().select("clip", [chords.id]));
    fireEvent.keyDown(document.querySelector('[data-feature="arrangement"]')!, { key: "l", metaKey: true, shiftKey: true });
    await flush();
    expect(project().clips[chords.id]!.looping).toEqual({ enabled: true, start: 0, end: 16 });
  });

  it("reverses a MIDI clip with R: the notes mirror in time, one undo step", async () => {
    const chords = clipByName("Chords");
    const notesOfChords = () =>
      Object.values(project().notes)
        .filter((n) => n.clip === chords.id)
        .map((n) => [n.id, n.start, n.duration] as const);
    const before = notesOfChords();
    expect(before.length).toBeGreaterThan(0);
    act(() => itemSelection.getState().select("clip", [chords.id]));
    sent.length = 0;
    fireEvent.keyDown(document.querySelector('[data-feature="arrangement"]')!, { key: "r" });
    await flush();
    expect(sent.some((c) => c.domain === "Note" && c.command.type === "Reverse" && c.command.clip === chords.id && c.command.notes === null)).toBe(true);
    const c = project().clips[chords.id]!;
    const r1 = c.looping.enabled ? c.looping.end : c.offset + c.length;
    const r0 = c.looping.enabled ? c.looping.start : Math.max(c.offset, 0);
    for (const [id, start, duration] of before) {
      const n = project().notes[id]!;
      if (start >= r0 && start + duration <= r1) expect(n.start).toBeCloseTo(r0 + r1 - (start + duration));
      else expect(n.start).toBe(start);
      expect(n.duration).toBe(duration);
    }
    await undo();
    expect(notesOfChords()).toEqual(before);
  });

  it("selects clips with the marquee", async () => {
    const content = screen.getByTestId("arrangement-content");
    // From a clip body: presses there pass through to the lane (the marquee).
    await drag(content, 400, 150, { x: HEADER_WIDTH + 10, y: 30 });
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
    const at = partAt(chords);
    fireEvent.doubleClick(lanes(), { clientX: at.x, clientY: at.y });
    expect(useEditorStore.getState().clip).toBe(chords.id);
    expect(useEditorStore.getState().request).toBe(1);
  });

  it("moves the playhead to a clip's start when it is pressed while stopped", async () => {
    const bass = clipByName("Bassline");
    await dragClip(bass, 0);
    await waitFor(() => expect(playheadStore.getPlayhead()?.transport.position).toBe(startOf(bass)));
  });

  it("a click on a clip's body selects it and moves the playhead to the click (not the clip start)", async () => {
    const bass = clipByName("Bassline");
    const content = screen.getByTestId("arrangement-content");
    // Bassline is on row 1; its body is below the title bar. Click 2 beats into it.
    const x = HEADER_WIDTH + (bass.start + 2) * PX;
    await drag(content, 0, 0, { x, y: ROW + 30 });
    expect([...itemSelection.getState().selected.clip]).toEqual([bass.id]);
    await waitFor(() => expect(playheadStore.getPlayhead()?.transport.position).toBe(bass.start + 2));
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
    await dragClip(clipByName("Bassline"), 0);
    expect(sent.filter((c) => c.domain === "Transport" && c.command.type === "Locate")).toHaveLength(1);
  });

  it("a click on a lane's empty space selects its track like a header click (inspector focus, marker kept)", async () => {
    const content = screen.getByTestId("arrangement-content");
    const [keys, bass, drums] = ["Keys", "Bass", "Drums"].map(trackByName);
    const at = (row: number, beat: number) => ({ x: HEADER_WIDTH + beat * PX, y: ROW * row + 30 });
    const ui = () => useArrangementUi.getState();

    // Select a clip first: the empty-space click replaces it with the track.
    await dragClip(clipByName("Bassline"), 0);
    expect(itemSelection.getState().selected.clip.size).toBe(1);

    await drag(content, 0, 0, at(2, 40.4));
    expect(ui().trackFocus).toBe(drums!.id);
    expect([...ui().selectedTracks]).toEqual([drums!.id]);
    expect(useSelectionStore.getState().selectedTrack).toBe(drums!.id);
    expect(itemSelection.getState().selected.clip.size).toBe(0);
    expect(document.querySelectorAll(".eth-arr-header--selected")).toHaveLength(1);
    // The insert marker still lands there (selecting the track doesn't clear it).
    expect(useTimeSelection.getState().selection).toMatchObject({ start: 40, end: 40, tracks: [drums!.id] });

    // Modifiers act like on the header: cmd toggles, shift selects the range.
    await drag(content, 0, 0, { ...at(0, 40), up: { metaKey: true } });
    expect(new Set(ui().selectedTracks)).toEqual(new Set([keys!.id, drums!.id]));
    await drag(content, 0, 0, { ...at(0, 40), up: { metaKey: true } });
    expect([...ui().selectedTracks]).toEqual([drums!.id]);
    await drag(content, 0, 0, { ...at(0, 40), up: { shiftKey: true } });
    expect(new Set(ui().selectedTracks)).toEqual(new Set([keys!.id, bass!.id, drums!.id]));

    // A drag over empty space is still a selection box: it leaves the tracks alone.
    await drag(content, 4 * PX, 10, at(1, 40));
    expect(ui().trackFocus).toBe(drums!.id);
    expect(ui().selectedTracks.size).toBe(3);

    // A click on a clip's body selects the clip (no track focus); below the tracks clears it.
    await drag(content, 0, 0, at(1, startOf(clipByName("Bassline")) + 2));
    expect(ui().trackFocus).toBeNull();
    expect([...itemSelection.getState().selected.clip]).toEqual([clipByName("Bassline").id]);
    await drag(content, 0, 0, at(2, 40));
    expect(ui().trackFocus).toBe(drums!.id);
    await drag(content, 0, 0, { x: HEADER_WIDTH + 40 * PX, y: ROW * 4 + 200 });
    expect(ui().trackFocus).toBeNull();
  });

  it("selecting a clip on the selected track drops the track's selected look and glow", async () => {
    const content = screen.getByTestId("arrangement-content");
    const at = (row: number, beat: number) => ({ x: HEADER_WIDTH + beat * PX, y: ROW * row + 30 });
    await drag(content, 0, 0, at(1, 40.4));
    expect(document.querySelectorAll(".eth-arr-header--selected")).toHaveLength(1);
    expect(document.querySelectorAll(".eth-arr-header__glow")).toHaveLength(1);
    await drag(content, 0, 0, at(1, startOf(clipByName("Bassline")) + 2));
    expect([...itemSelection.getState().selected.clip]).toEqual([clipByName("Bassline").id]);
    expect(document.querySelectorAll(".eth-arr-header--selected")).toHaveLength(0);
    expect(document.querySelectorAll(".eth-arr-header__glow")).toHaveLength(0);
    expect(document.querySelectorAll(".eth-arr-row--selected")).toHaveLength(0);
  });

  it("pressing cmd mid-drag copies the clip instead of moving it", async () => {
    const bass = clipByName("Bassline");
    const start = startOf(bass);
    const at = partAt(bass);
    await act(async () => {
      fireEvent.pointerDown(lanes(), { button: 0, pointerId: 1, clientX: at.x, clientY: at.y });
      fireEvent.pointerMove(window, { pointerId: 1, clientX: at.x + 4 * PX, clientY: at.y });
    });
    expect(useArrangementUi.getState().preview?.copy).toBe(false);
    act(() => {
      fireEvent.keyDown(window, { key: "Meta", metaKey: true });
    });
    expect(useArrangementUi.getState().preview?.copy).toBe(true);
    await act(async () => {
      fireEvent.pointerUp(window, { pointerId: 1, clientX: at.x + 4 * PX, clientY: at.y, metaKey: true });
    });
    await flush();
    expect(project().clips[bass.id]!.start).toBe(start);
    expect(Object.values(project().clips).some((c) => c.track === bass.track && startOf(c) === start + 4)).toBe(true);
  });

  it("renames a track inline by double-clicking its name (Enter commits, Escape cancels)", async () => {
    const keys = trackByName("Keys");
    const header = () => document.querySelector<HTMLElement>(`.eth-arr-row[data-track="${keys.id}"] .eth-arr-header__name`)!;
    fireEvent.doubleClick(header());
    const input = screen.getByRole("textbox", { name: "track name" });
    fireEvent.change(input, { target: { value: "Piano" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await flush();
    expect(project().tracks[keys.id]!.name).toBe("Piano");

    fireEvent.doubleClick(header());
    const again = screen.getByRole("textbox", { name: "track name" });
    fireEvent.change(again, { target: { value: "Nope" } });
    fireEvent.keyDown(again, { key: "Escape" });
    await flush();
    expect(project().tracks[keys.id]!.name).toBe("Piano");
    expect(screen.queryByRole("textbox", { name: "track name" })).toBeNull();
  });

  it("cmd-dragging a MIDI clip onto another track copies it and leaves the original", async () => {
    const chords = clipByName("Chords");
    const bass = clipByName("Bassline");
    const count = Object.keys(project().clips).length;
    // Chords (Keys, row 0) copied straight down onto Bass (row 1), overlapping in time.
    await dragClip(chords, 0, ROW, { up: { metaKey: true } });
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
    expect(fader.getAttribute("aria-valuetext")).toMatch(/db/);
    // The drag didn't select or move the track.
    expect(useArrangementUi.getState().trackFocus).toBeNull();
    await undo();
    expect(trackByName("Keys").mixer.volume).toBeCloseTo(before);

    fireEvent.doubleClick(fader);
    await flush();
    expect(trackByName("Keys").mixer.volume).toBeCloseTo(0);
  });

  it("#911: a volume drag over 32 selected tracks sends one batch per frame, not one command per track", async () => {
    await act(async () => {
      for (let i = 0; i < 29; i++) {
        await mock.send(cmd("Track", { type: "Create", id: newId(), kind: "Midi", name: `T${i}`, color: null, parent: null, before: null }));
      }
    });
    await flush();
    const ids = Object.keys(project().tracks);
    expect(ids.length).toBeGreaterThanOrEqual(32);
    act(() => useArrangementUi.getState().setTrackSelection(ids, ids[0]!));
    const before = new Map(ids.map((id) => [id, project().tracks[id]!.mixer.volume]));
    const keys = trackByName("Keys");
    const fader = screen.getByRole("slider", { name: "Keys volume" });
    Object.defineProperty(fader, "clientWidth", { configurable: true, value: 100 });
    sent.length = 0;
    const MOVES = 20;
    await act(async () => {
      fireEvent.pointerDown(fader, { button: 0, pointerId: 1, clientX: 80 });
      for (let i = 1; i <= MOVES; i++) fireEvent.pointerMove(fader, { pointerId: 1, clientX: 80 - i });
      fireEvent.pointerUp(fader, { pointerId: 1, clientX: 80 - MOVES });
    });
    await flush();
    // No per-track commands: batches of the whole selection, at most one per move.
    expect(sent.filter((c) => c.domain === "Mixer")).toHaveLength(0);
    const batches = sent.filter((c) => c.domain === "Edit" && c.command.type === "Batch");
    expect(batches.length).toBeGreaterThan(0);
    expect(batches.length).toBeLessThanOrEqual(MOVES);
    expect(sent.length).toBeLessThanOrEqual(batches.length + 1); // + EndGesture
    // Every track moved by the same dB amount as the dragged one (their balance is kept).
    const delta = project().tracks[keys.id]!.mixer.volume - before.get(keys.id)!;
    expect(delta).toBeLessThan(0);
    for (const id of ids) expect(project().tracks[id]!.mixer.volume - before.get(id)!).toBeCloseTo(delta);
    // The other faders follow (their rows don't re-render for a volume change: the fader reads the store).
    const bass = screen.getByRole("slider", { name: "Bass volume" });
    expect(bass.getAttribute("aria-valuetext")).toBe(formatDb(trackByName("Bass").mixer.volume));
    // One undo step for the whole drag.
    await undo();
    for (const id of ids) expect(project().tracks[id]!.mixer.volume).toBeCloseTo(before.get(id)!);
  });

  it("a header pan knob (right of the volume): a drag is one undo step; double-click centers it", async () => {
    const pan = screen.getByRole("slider", { name: "Keys pan" });
    const fader = screen.getByRole("slider", { name: "Keys volume" });
    // Right of the volume fader, in the same header.
    expect(fader.compareDocumentPosition(pan) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    const before = trackByName("Keys").mixer.pan;
    // jsdom has no pointer capture.
    Object.assign(pan, { setPointerCapture: () => {}, releasePointerCapture: () => {}, hasPointerCapture: () => true });
    await act(async () => {
      fireEvent.pointerDown(pan, { button: 0, pointerId: 1, clientY: 100 });
      fireEvent.pointerMove(pan, { pointerId: 1, clientY: 70 });
      fireEvent.pointerMove(pan, { pointerId: 1, clientY: 40 });
      fireEvent.pointerUp(pan, { pointerId: 1, clientY: 40 });
    });
    await flush();
    const right = trackByName("Keys").mixer.pan;
    expect(right).toBeGreaterThan(before);
    expect(pan.getAttribute("aria-valuetext")).toMatch(/R$/);
    // The drag didn't select or move the track.
    expect(useArrangementUi.getState().trackFocus).toBeNull();
    await undo();
    expect(trackByName("Keys").mixer.pan).toBeCloseTo(before);

    fireEvent.doubleClick(pan);
    await flush();
    expect(trackByName("Keys").mixer.pan).toBe(0);
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

    const grip = screen.getByRole("separator", { name: "resize track headers" });
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

  describe("return tracks (#791)", () => {
    const returnNames = () =>
      tracksOrdered(project())
        .filter((t) => t.kind === "Return")
        .map((t) => t.name);
    // Rows: Keys, Bass, Drums, then the returns from 3 * ROW.
    async function addReturns() {
      for (const name of ["B Rev", "C Echo"]) {
        await act(async () => {
          await mock.send(cmd("Track", { type: "Create", id: newId(), kind: "Return", name, color: null, parent: null, before: null }));
        });
      }
      await flush();
      expect(returnNames()).toEqual(["A Delay", "B Rev", "C Echo"]);
    }
    async function pressAndMove(name: string, fromY: number, toY: number) {
      const header = screen.getByRole("group", { name: `${name} track` });
      await act(async () => {
        fireEvent.pointerDown(header, { button: 0, pointerId: 1, clientX: 20, clientY: fromY });
        fireEvent.pointerMove(window, { pointerId: 1, clientX: 20, clientY: (fromY + toY) / 2 });
        fireEvent.pointerMove(window, { pointerId: 1, clientX: 20, clientY: toY });
      });
      return header;
    }
    async function release(y: number) {
      await act(async () => {
        fireEvent.pointerUp(window, { pointerId: 1, clientX: 20, clientY: y });
      });
      await flush();
    }

    it("reorders returns by dragging a header between the others; sends keep their return; one undo step", async () => {
      await addReturns();
      const keys = trackByName("Keys").id;
      for (const name of returnNames()) {
        await act(async () => {
          await mock.send(cmd("Mixer", { type: "CreateSend", id: newId(), from: keys, to: trackByName(name).id, level: -6, pre_fader: false }));
        });
      }
      // Keys sends to every return (the demo's send to A Delay was already there).
      const sends = Object.fromEntries(Object.values(project().sends).map((s) => [s.id, s.to]));
      expect(
        new Set(
          Object.values(project().sends)
            .filter((s) => s.from === keys)
            .map((s) => s.to),
        ).size,
      ).toBe(3);
      // C Echo (row 5) between A Delay and B Rev (the gap at row 4).
      await pressAndMove("C Echo", 5 * ROW + 10, 4 * ROW + 3);
      expect(screen.getByTestId("track-drop-line").style.top).toBe(`${4 * ROW}px`);
      expect(screen.queryByTestId("track-drop-blocked")).toBeNull();
      await release(4 * ROW + 3);
      expect(returnNames()).toEqual(["A Delay", "C Echo", "B Rev"]);
      expect(sent).toContainEqual(
        expect.objectContaining({ domain: "Track", command: expect.objectContaining({ type: "Move", before: trackByName("B Rev").id }) }),
      );
      for (const [id, to] of Object.entries(sends)) expect(project().sends[id]?.to).toBe(to);
      // The regular tracks stay where they were.
      expect(
        tracksOrdered(project())
          .map((t) => t.name)
          .slice(0, 3),
      ).toEqual(["Keys", "Bass", "Drums"]);
      await undo();
      expect(returnNames()).toEqual(["A Delay", "B Rev", "C Echo"]);
    });

    it("shows the not-allowed zone over the regular tracks, and dropping there does nothing", async () => {
      await addReturns();
      const header = await pressAndMove("B Rev", 4 * ROW + 10, ROW + 5);
      expect(screen.queryByTestId("track-drop-line")).toBeNull();
      const zone = screen.getByTestId("track-drop-blocked");
      expect(zone.style.top).toBe("0px");
      expect(zone.style.height).toBe(`${3 * ROW}px`);
      expect(header).toHaveClass("eth-arr-header--drop-blocked");
      expect(document.documentElement.style.getPropertyValue("--eth-drag-cursor")).toBe("not-allowed");
      const count = sent.length;
      await release(ROW + 5);
      expect(sent.slice(count).some((c) => c.domain === "Track")).toBe(false);
      expect(returnNames()).toEqual(["A Delay", "B Rev", "C Echo"]);
      expect(screen.queryByTestId("track-drop-blocked")).toBeNull();
      expect(header).not.toHaveClass("eth-arr-header--drop-blocked");
    });

    it("Escape cancels a return drag", async () => {
      await addReturns();
      await pressAndMove("C Echo", 5 * ROW + 10, 3 * ROW + 3);
      expect(screen.getByTestId("track-drop-line").style.top).toBe(`${3 * ROW}px`);
      fireEvent.keyDown(window, { key: "Escape" });
      expect(screen.queryByTestId("track-drop-line")).toBeNull();
      const count = sent.length;
      await release(3 * ROW + 3);
      expect(sent.slice(count).some((c) => c.domain === "Track")).toBe(false);
      expect(returnNames()).toEqual(["A Delay", "B Rev", "C Echo"]);
    });

    it("adds a return after the selected return, and the add menu's return drags between the returns", async () => {
      await addReturns();
      fireEvent.click(screen.getByRole("group", { name: "A Delay track" }));
      fireEvent.click(screen.getByRole("button", { name: "add a track at the end" }));
      fireEvent.click(screen.getByRole("menuitem", { name: "create return track" }));
      await flush();
      const order = tracksOrdered(project()).filter((t) => t.kind === "Return");
      expect(order.map((t) => t.name).filter((n) => n !== "A Delay" && n !== "B Rev" && n !== "C Echo")).toHaveLength(1);
      expect(order[0]!.name).toBe("A Delay");
      expect(order.slice(2).map((t) => t.name)).toEqual(["B Rev", "C Echo"]);
      const added = order[1]!.id;
      await undo();
      expect(project().tracks[added]).toBeUndefined();

      // Drag "create return track" from the menu: over the regular tracks it can't go...
      fireEvent.click(screen.getByRole("button", { name: "add a track at the end" }));
      const item = screen.getByRole("menuitem", { name: "create return track" });
      vi.spyOn(screen.getByTestId("arrangement-content"), "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 1200, 800));
      await act(async () => {
        fireEvent.pointerDown(item, { button: 0, pointerId: 1, clientX: 10, clientY: -20 });
        fireEvent.pointerMove(window, { pointerId: 1, clientX: 300, clientY: ROW });
        fireEvent.pointerMove(window, { pointerId: 1, clientX: 300, clientY: ROW + 5 });
      });
      expect(screen.getByTestId("track-drop-blocked")).toBeInTheDocument();
      // ...between B Rev and C Echo it goes.
      await act(async () => {
        fireEvent.pointerMove(window, { pointerId: 1, clientX: 300, clientY: 5 * ROW + 3 });
      });
      expect(screen.getByTestId("track-drop-line").style.top).toBe(`${5 * ROW}px`);
      await act(async () => {
        fireEvent.pointerUp(window, { pointerId: 1, clientX: 300, clientY: 5 * ROW + 3 });
      });
      await flush();
      const names = returnNames();
      expect(names).toHaveLength(4);
      expect([names[0], names[1], names[3]]).toEqual(["A Delay", "B Rev", "C Echo"]);
    });
  });

  it("keeps a single selected entity: a track or clips, and Delete removes that one", async () => {
    const bass = clipByName("Bassline");
    const keys = trackByName("Keys");
    await dragClip(bass, 0);
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
    await dragClip(bass, 0);
    expect(useArrangementUi.getState().trackFocus).toBeNull();
    expect(document.querySelector(".eth-arr-header--selected")).toBeNull();
  });

  it("a double-click on a clip's body (passed through to the lane) opens it instead of inserting", async () => {
    const chords = clipByName("Chords");
    const lane = document.querySelector<HTMLElement>(`[data-lane="${chords.track}"]`)!;
    const before = Object.keys(project().clips).length;
    const x = HEADER_WIDTH + (startOf(chords) + 1) * PX;
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

  it("renames a clip in place from its menu (Enter renames, Escape cancels)", async () => {
    const bass = clipByName("Bassline");
    const menuAt = () => partAt(project().clips[bass.id]!);
    fireEvent.contextMenu(lanes(), { clientX: menuAt().x, clientY: menuAt().y });
    fireEvent.click(await screen.findByRole("menuitem", { name: "rename" }));
    const field = await screen.findByRole("textbox", { name: "rename Bassline" });
    expect(document.activeElement).toBe(field);
    fireEvent.change(field, { target: { value: "Low End" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await flush();
    expect(project().clips[bass.id]!.name).toBe("Low End");
    expect(screen.queryByRole("textbox", { name: /^rename/ })).toBeNull();

    fireEvent.contextMenu(lanes(), { clientX: menuAt().x, clientY: menuAt().y });
    fireEvent.click(await screen.findByRole("menuitem", { name: "rename" }));
    const again = await screen.findByRole("textbox", { name: "rename Low End" });
    fireEvent.change(again, { target: { value: "Nope" } });
    fireEvent.keyDown(again, { key: "Escape" });
    await flush();
    expect(project().clips[bass.id]!.name).toBe("Low End");
  });

  describe("copy / cut / paste", () => {
    const root = () => document.querySelector<HTMLElement>('[data-feature="arrangement"]')!;
    const key = (k: string) => fireEvent.keyDown(root(), { key: k, metaKey: true });
    const clipsOn = (track: string) => Object.values(project().clips).filter((c) => c.track === track);
    /** Move the selection cursor (not the playhead) where the next paste lands (no marker). */
    async function setCursor(position: number) {
      await act(async () => {
        useTimeSelection.setState({ cursor: position, selection: null });
      });
    }

    it("copies the selection and pastes it at the selection cursor as one undo step", async () => {
      const bass = clipByName("Bassline");
      const notes = (clip: string) => Object.values(project().notes).filter((n) => n.clip === clip).length;
      await dragClip(bass, 0);
      key("c");
      await setCursor(32);
      const playheadBefore = playheadStore.getPlayhead()?.transport.position;
      key("v");
      await flush();
      const pasted = clipsOn(bass.track).find((c) => startOf(c) === 32)!;
      expect(pasted).toMatchObject({ length: bass.length, name: bass.name });
      expect(notes(pasted.id)).toBe(notes(bass.id));
      expect([...itemSelection.getState().selected.clip]).toEqual([pasted.id]);
      // section-edit: the insert marker (not the playhead) moves to the pasted end.
      expect(useTimeSelection.getState().selection).toMatchObject({ start: 32 + bass.length, end: 32 + bass.length });
      expect(playheadStore.getPlayhead()?.transport.position).toBe(playheadBefore);
      expect(useTimeSelection.getState().cursor).toBe(32 + bass.length);
      await undo();
      expect(project().clips[pasted.id]).toBeUndefined();
    });

    it("cut removes the clips and paste rebuilds them (notes included)", async () => {
      const bass = clipByName("Bassline");
      const noteCount = Object.values(project().notes).filter((n) => n.clip === bass.id).length;
      await dragClip(bass, 0);
      key("x");
      await flush();
      expect(project().clips[bass.id]).toBeUndefined();
      await setCursor(40);
      key("v");
      await flush();
      const pasted = clipsOn(bass.track).find((c) => startOf(c) === 40)!;
      expect(pasted).toMatchObject({ length: bass.length, offset: bass.offset, name: bass.name });
      expect(Object.values(project().notes).filter((n) => n.clip === pasted.id)).toHaveLength(noteCount);
    });

    it("handles the clipboard events the macOS Edit menu sends", async () => {
      const bass = clipByName("Bassline");
      await dragClip(bass, 0);
      root().focus();
      act(() => {
        document.dispatchEvent(new Event("copy", { bubbles: true, cancelable: true }));
      });
      await setCursor(48);
      await act(async () => {
        document.dispatchEvent(new Event("paste", { bubbles: true, cancelable: true }));
      });
      await flush();
      expect(clipsOn(bass.track).some((c) => startOf(c) === 48)).toBe(true);
    });

    it("pastes where an empty lane is right-clicked, onto that track", async () => {
      const chords = clipByName("Chords");
      await dragClip(chords, 0);
      key("c");
      const lane = document.querySelector<HTMLElement>(`[data-lane="${chords.track}"]`)!;
      fireEvent.contextMenu(lane, { clientX: HEADER_WIDTH + 24.3 * PX, clientY: 30 });
      fireEvent.click(screen.getByRole("menuitem", { name: "paste" }));
      await flush();
      expect(clipsOn(chords.track).some((c) => startOf(c) === 24)).toBe(true);
    });
  });

  it("offers Delete in the right-click menu of a clip and of a track", async () => {
    const bass = clipByName("Bassline");
    const at = partAt(bass);
    fireEvent.contextMenu(lanes(), { clientX: at.x, clientY: at.y });
    expect([...itemSelection.getState().selected.clip]).toEqual([bass.id]);
    fireEvent.click(screen.getByRole("menuitem", { name: "delete" }));
    await flush();
    expect(project().clips[bass.id]).toBeUndefined();

    const keys = trackByName("Keys");
    fireEvent.contextMenu(screen.getByRole("group", { name: "Keys track" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "delete track" }));
    await flush();
    expect(project().tracks[keys.id]).toBeUndefined();
  });

  it("resizes one track in steps by dragging its bottom edge, and resets on double-click", async () => {
    const keys = trackByName("Keys");
    const handle = screen.getByRole("separator", { name: "resize Keys" });
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
  async function doublePressDrag(lane: Element, laneX: number, laneToX = laneX) {
    // Lane px from the lanes' left edge (after the header column).
    const [x, toX] = [HEADER_WIDTH + laneX, HEADER_WIDTH + laneToX];
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

describe("ArrangementView: track multi-selection (base-151)", () => {
  const root = () => document.querySelector<HTMLElement>('[data-feature="arrangement"]')!;
  const key = async (k: string) => {
    await act(async () => {
      fireEvent.keyDown(root(), { key: k, metaKey: true });
    });
    await flush();
  };
  const header = (name: string) => screen.getByRole("group", { name: `${name} track` });
  const order = () => tracksOrdered(project()).map((t) => t.id);
  const clipsOf = (track: string) => Object.values(project().clips).filter((c) => c.track === track && !c.lane).length;
  /** Select Keys, then shift-click Bass (a range of two). */
  const selectKeysAndBass = () => {
    fireEvent.click(header("Keys"));
    fireEvent.click(header("Bass"), { shiftKey: true });
  };

  it("Cmd+D duplicates the selected tracks after the last one, in order, as one undo step; the copies get selected", async () => {
    const [keys, bass] = [trackByName("Keys"), trackByName("Bass")];
    const before = order();
    selectKeysAndBass();
    expect([...useArrangementUi.getState().selectedTracks]).toEqual([keys.id, bass.id]);
    await key("d");
    const now = order();
    expect(now).toHaveLength(before.length + 2);
    const [k2, b2] = [now[2]!, now[3]!];
    expect(now.slice(0, 2)).toEqual([keys.id, bass.id]);
    expect(project().tracks[k2]!.name).toBe("Keys");
    expect(project().tracks[b2]!.name).toBe("Bass");
    expect(now.slice(4)).toEqual(before.slice(2));
    // With their clips.
    expect(clipsOf(k2)).toBe(clipsOf(keys.id));
    expect(clipsOf(b2)).toBe(clipsOf(bass.id));
    expect([...useArrangementUi.getState().selectedTracks]).toEqual([k2, b2]);
    // One undo step.
    expect(sent.filter((c) => c.domain === "Edit" && c.command.type === "Batch")).toHaveLength(1);
    await undo();
    expect(order()).toEqual(before);
  });

  it("Cmd+C / Cmd+V pastes copies of the tracks after the selected track, one undo step", async () => {
    const before = order();
    selectKeysAndBass();
    await key("c");
    fireEvent.click(header("Drums"));
    await key("v");
    const now = order();
    const drums = trackByName("Drums").id;
    const at = now.indexOf(drums);
    expect(now).toHaveLength(before.length + 2);
    expect(now.slice(at + 1, at + 3).map((id) => project().tracks[id]!.name)).toEqual(["Keys", "Bass"]);
    expect(useArrangementUi.getState().selectedTracks.size).toBe(2);
    await undo();
    expect(order()).toEqual(before);
  });

  it("Cmd+X deletes the tracks; Cmd+V brings them back (clips kept) at the paste position", async () => {
    const [keys, bass] = [trackByName("Keys"), trackByName("Bass")];
    const clips = clipsOf(keys.id);
    const before = order();
    selectKeysAndBass();
    await key("x");
    expect(project().tracks[keys.id]).toBeUndefined();
    expect(project().tracks[bass.id]).toBeUndefined();
    fireEvent.click(header("Drums"));
    await key("v");
    const now = order();
    const drums = trackByName("Drums").id;
    expect(now.slice(now.indexOf(drums), now.indexOf(drums) + 3)).toEqual([drums, keys.id, bass.id]);
    expect(clipsOf(keys.id)).toBe(clips);
    // Undo puts them back where they were.
    await undo();
    expect(order()).toEqual(before);
  });

  it("right-click on a selected track acts on the whole selection, with counted labels", async () => {
    const [keys, bass] = [trackByName("Keys"), trackByName("Bass")];
    selectKeysAndBass();
    fireEvent.contextMenu(header("Bass"));
    const labels = screen.getAllByRole("menuitem").map((m) => m.textContent);
    expect(labels.join("|")).toContain("duplicate 2 tracks");
    expect(labels.join("|")).toContain("copy 2 tracks");
    expect(screen.queryByRole("menuitem", { name: /Take Lane/ })).toBeNull();
    fireEvent.click(screen.getByRole("menuitem", { name: /delete 2 tracks/ }));
    await flush();
    expect(project().tracks[keys.id]).toBeUndefined();
    expect(project().tracks[bass.id]).toBeUndefined();
    await undo();
    expect(project().tracks[keys.id]).toBeDefined();
    expect(project().tracks[bass.id]).toBeDefined();
  });

  it("right-click on an unselected track selects only it; Duplicate Track duplicates just it", async () => {
    const drums = trackByName("Drums");
    selectKeysAndBass();
    const before = order();
    fireEvent.contextMenu(header("Drums"));
    expect([...useArrangementUi.getState().selectedTracks]).toEqual([drums.id]);
    fireEvent.click(screen.getByRole("menuitem", { name: /duplicate track/ }));
    await flush();
    const now = order();
    expect(now).toHaveLength(before.length + 1);
    expect(project().tracks[now[now.indexOf(drums.id) + 1]!]!.name).toBe("Drums");
  });

  it("duplicating a selected group copies its children once (a selected child goes with it)", async () => {
    const [keys, bass] = [trackByName("Keys"), trackByName("Bass")];
    const group = newId();
    await act(async () => {
      await mock.send(cmd("Track", { type: "GroupSelected", ids: [keys.id, bass.id], group, name: "Grp" }));
    });
    await flush();
    const before = order().length;
    act(() => useArrangementUi.getState().setTrackSelection([group, keys.id], group));
    await key("d");
    expect(order()).toHaveLength(before + 3);
    const copy = [...useArrangementUi.getState().selectedTracks];
    expect(copy).toHaveLength(1);
    expect(project().tracks[copy[0]!]!.kind).toBe("Group");
    expect(Object.values(project().tracks).filter((t) => t.parent === copy[0])).toHaveLength(2);
  });

  it("clip vs track focus: the most recently selected kind is the edit target", async () => {
    const { editTarget } = await import("./actions");
    const chords = clipByName("Chords");
    fireEvent.click(header("Keys"));
    expect(editTarget()).toBe("tracks");
    act(() => itemSelection.getState().select("clip", [chords.id]));
    // Selecting clips cleared the track selection.
    expect(useArrangementUi.getState().selectedTracks.size).toBe(0);
    expect(editTarget()).toBe("clips");
    // Both set (tracks by code, after the clips): the later one wins.
    act(() => useArrangementUi.getState().setTrackSelection([trackByName("Bass").id], trackByName("Bass").id));
    expect(itemSelection.getState().selected.clip.size).toBe(1);
    expect(editTarget()).toBe("tracks");
  });

  it("Mod+A with a track selected selects every track (not the master), no clips", async () => {
    fireEvent.click(header("Keys"));
    await key("a");
    const expected = tracksOrdered(project())
      .filter((t) => t.kind !== "Master")
      .map((t) => t.id);
    expect(new Set(useArrangementUi.getState().selectedTracks)).toEqual(new Set(expected));
    expect(itemSelection.getState().selected.clip.size).toBe(0);
    // The focus (shift-click anchor) stays on the clicked track.
    expect(useArrangementUi.getState().trackFocus).toBe(trackByName("Keys").id);
    // Follow-up edits act on them all, like a manual multi-selection: delete removes every
    // track but the master.
    await act(async () => {
      fireEvent.keyDown(root(), { key: "Delete" });
    });
    await flush();
    expect(tracksOrdered(project()).map((t) => t.kind)).toEqual(["Master"]);
  });

  it("Mod+A with a clip selected (or nothing) selects every clip, no tracks", async () => {
    act(() => itemSelection.getState().select("clip", [clipByName("Chords").id]));
    await key("a");
    const arrangementClips = Object.values(project().clips).filter((c) => !c.lane);
    expect(itemSelection.getState().selected.clip.size).toBe(arrangementClips.length);
    expect(useArrangementUi.getState().selectedTracks.size).toBe(0);
    act(() => itemSelection.getState().clear("clip"));
    await key("a");
    expect(itemSelection.getState().selected.clip.size).toBe(arrangementClips.length);
  });

  it("allSelectableTracks: the shown rows in display order, children of folded groups and the master left out", async () => {
    const { allSelectableTracks } = await import("./actions");
    const [keys, bass] = [trackByName("Keys"), trackByName("Bass")];
    const group = newId();
    await act(async () => {
      await mock.send(cmd("Track", { type: "GroupSelected", ids: [keys.id, bass.id], group, name: "Grp" }));
    });
    const all = allSelectableTracks(project(), new Set());
    expect(all).toContain(group);
    expect(all).toContain(keys.id);
    expect(all.some((id) => project().tracks[id]!.kind === "Master")).toBe(false);
    const folded = allSelectableTracks(project(), new Set([group]));
    expect(folded).toContain(group);
    expect(folded).not.toContain(keys.id);
    expect(folded).not.toContain(bass.id);
  });
});

describe("ArrangementView: audio and drops", () => {
  it("zooming out stops a little past the song's end (at least 32 bars)", async () => {
    const view = arrangementView.getState();
    act(() => view.setWidth(800));
    await flush();
    const p = useProjectStore.getState().project!;
    const end = Math.max(...Object.values(p.clips).map((c) => c.start + c.length));
    const widest = Math.max(128, end * 1.5);
    act(() => arrangementView.getState().zoomTo(0.001));
    expect(arrangementView.getState().pxPerBeat).toBeCloseTo(800 / widest, 6);
  });

  it("zooming scales the lane layers (compositor only); --ppb follows once the zoom settles", async () => {
    const layer = document.querySelector<HTMLElement>(`[data-lane="${trackByName("Keys").id}"] .eth-arr-lane__layer`)!;
    const view = arrangementView.getState();
    act(() => view.setWidth(800));
    await flush();
    const ppb = layer.style.getPropertyValue("--ppb");
    vi.useFakeTimers();
    try {
      act(() => view.zoomBy(1.2, 0));
      // Mid-zoom: --ppb (and so every overlay's style) unchanged, the layer stretched.
      expect(layer.style.getPropertyValue("--ppb")).toBe(ppb);
      expect(layer.style.transform).toMatch(/scaleX\(1\.2\d*\)/);
      act(() => vi.advanceTimersByTime(200));
      // Lanes restyle over the next frames (drawQueue.ts).
      act(() => flushDraws());
      // Settled: laid out at the new zoom, no stretch.
      expect(parseFloat(layer.style.getPropertyValue("--ppb"))).toBeCloseTo(parseFloat(ppb) * 1.2, 6);
      expect(layer.style.transform).not.toContain("scaleX");
    } finally {
      vi.useRealTimers();
    }
  });

  /** Fire a drag event with a fake DataTransfer (jsdom has no DragEvent, so set clientX/Y by hand). */
  function fireDrag(type: "dragOver" | "drop", el: Element, payload: object, clientX: number, clientY: number, extraTypes: string[] = []) {
    const data = JSON.stringify(payload);
    const ev = createEvent[type](el, {
      dataTransfer: {
        types: [BROWSER_DRAG_MIME, ...extraTypes, "text/plain"],
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
    expect(lane.querySelector('[data-testid="import-placeholder"]')?.textContent).toBe("importing Pad.mp3… 25%");
    expect(screen.getAllByTestId("import-placeholder").some((e) => e.textContent === "import failed: Pad.mp3")).toBe(true);
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

describe("ArrangementView: MIDI files (#717)", () => {
  function fireMidiDrag(type: "dragOver" | "drop", el: Element, clientX: number, clientY: number) {
    const payload = {
      version: 1,
      kind: "media",
      source: { type: "Location", location: { type: "Library", id: "library" }, path: "MIDI/Groove 1.mid" },
      name: "Groove 1.mid",
      file_kind: "Midi",
    };
    const data = JSON.stringify(payload);
    const ev = createEvent[type](el, {
      dataTransfer: {
        types: [BROWSER_DRAG_MIME, BROWSER_MIDI_DRAG_MIME, "text/plain"],
        getData: (t: string) => (t === BROWSER_DRAG_MIME ? data : ""),
        dropEffect: "none",
      },
    });
    Object.defineProperty(ev, "clientX", { value: clientX });
    Object.defineProperty(ev, "clientY", { value: clientY });
    fireEvent(el, ev);
  }

  it("a browser MIDI file dropped on a MIDI track: its first track there, the others on new MIDI tracks after it, one undo step", async () => {
    const keys = trackByName("Keys");
    const content = screen.getByTestId("arrangement-content");
    const tracksBefore = Object.keys(project().tracks).length;
    // The hint follows MIDI tracks; an audio track refuses.
    fireMidiDrag("dragOver", content, HEADER_WIDTH + 8 * PX, 20);
    expect(useArrangementUi.getState().dropHint).toEqual({ track: keys.id, at: 8 });
    fireMidiDrag("dragOver", content, HEADER_WIDTH + 8 * PX, 2 * ROW + 20);
    expect(useArrangementUi.getState().dropHint).toBeNull();

    fireMidiDrag("drop", content, HEADER_WIDTH + 8 * PX, 20);
    await flush();
    const onKeys = Object.values(project().clips).find((c) => c.track === keys.id && startOf(c) === 8)!;
    expect(onKeys.name).toBe("drums");
    // One bar long (the file is 3.5 beats).
    expect(onKeys.length).toBe(4);
    expect(Object.values(project().notes).filter((n) => n.clip === onKeys.id)).toHaveLength(4);
    const bass = Object.values(project().tracks).find((t) => t.name === "bass")!;
    expect(bass.kind).toBe("Midi");
    const order = tracksOrdered(project()).map((t) => t.id);
    expect(order.indexOf(bass.id)).toBe(order.indexOf(keys.id) + 1);
    expect(Object.values(project().devices).some((d) => d.track === bass.id)).toBe(true);
    // The new clips are selected.
    expect(itemSelection.getState().selected.clip.has(onKeys.id)).toBe(true);
    // The project had clips: its tempo is kept.
    expect(Object.values(project().tempo_points)[0]!.bpm).not.toBe(100);

    await undo();
    expect(Object.keys(project().tracks)).toHaveLength(tracksBefore);
    expect(Object.values(project().clips).some((c) => c.track === keys.id && startOf(c) === 8)).toBe(false);
  });

  it("dropped below the tracks: one new MIDI track per file track", async () => {
    const content = screen.getByTestId("arrangement-content");
    const before = new Set(Object.keys(project().tracks));
    fireMidiDrag("drop", content, HEADER_WIDTH + 4 * PX, 5 * ROW + 20);
    await flush();
    const added = Object.values(project().tracks).filter((t) => !before.has(t.id));
    expect(added.map((t) => [t.name, t.kind])).toEqual(
      expect.arrayContaining([
        ["drums", "Midi"],
        ["bass", "Midi"],
      ]),
    );
    expect(added).toHaveLength(2);
  });

  it("export MIDI… on a MIDI clip and on a track writes a MIDI file", async () => {
    const urls = vi.fn(() => "blob:x");
    vi.stubGlobal("URL", Object.assign(URL, { createObjectURL: urls, revokeObjectURL: vi.fn() }));
    const chords = Object.values(project().clips).find((c) => c.name === "Chords")!;
    const entry = clipMenu(mock, chords).find((e) => e !== "separator" && e.label === "export midi…");
    if (!entry || entry === "separator") throw new Error("no export midi… on a midi clip");
    await act(async () => entry.onSelect());
    await flush();
    const first = sent.find((c) => c.domain === "Media" && c.command.type === "WriteMidiFile");
    expect(first?.domain === "Media" && first.command.type === "WriteMidiFile" && first.command.request).toMatchObject({
      clips: [chords.id],
      name: "Chords",
      from_project_start: false,
    });
    // An audio clip has none.
    const audio = Object.values(project().clips).find((c) => c.content.type === "Audio")!;
    expect(clipMenu(mock, audio).some((e) => e !== "separator" && e.label === "export midi…")).toBe(false);
    fireEvent.contextMenu(screen.getByRole("group", { name: "Keys track" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "export midi…" }));
    await flush();
    const writes = sent.filter((c) => c.domain === "Media" && c.command.type === "WriteMidiFile");
    expect(writes.length).toBeGreaterThan(0);
    const last = writes.at(-1)!;
    expect(last.domain === "Media" && last.command.type === "WriteMidiFile" && last.command.request).toMatchObject({
      name: "Keys",
      from_project_start: true,
      path: null,
    });
    expect(urls).toHaveBeenCalled();
    // An audio track has no MIDI clips: disabled.
    fireEvent.contextMenu(screen.getByRole("group", { name: "Drums track" }));
    expect(screen.getByRole("menuitem", { name: "export midi…" })).toBeDisabled();
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
    // Placed by a transform (a 1px strip moved and scaled): x = 4 beats, width = 4 beats.
    expect(loop.style.transform).toBe(`translateX(${4 * PX}px) scaleX(${4 * PX})`);
  });
});
