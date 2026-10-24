import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AutomationLane, AutomationPoint, Command, Track } from "@/generated";
import { playheadStore, useProjectStore, useSelectionStore } from "@/state";
import { createTimelineViewStore, itemSelection, type TimelineViewStore } from "@/timeline";
import { cmd, TransportProvider } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { paramToPlain } from "@/domain/paramScale";
import { formatKeys } from "@/features/keymap/chords";
import { pickOption } from "@/kit/testing";
import { AutomationToggleButton } from "./AutomationToggleButton";
import { clearPointClipboard, pointClipboard } from "./clipboard";
import { LANE_PAD } from "./geometry";
import { evaluatePoints } from "./curve";
import { TrackAutomationLanes } from "./index";
import { VOLUME_INFO } from "./params";
import { resetTimeSelection, useTimeSelection } from "@/features/time-edits/store";
import { AUTOMATION_BAR_HEIGHT, automationHeight, LANE_HEIGHT, laneUiKey, resetAutomationUi, useAutomationUi } from "./uiStore";

const store = () => useProjectStore.getState();
const project = () => store().project!;

let mock: MockTransport | undefined;
let rendered: ReturnType<typeof render> | undefined;
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
  rendered = render(
    <TransportProvider transport={mock}>
      <Probe />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  await flush();
  fireEvent.click(screen.getByRole("button", { name: /show automation of/ }));
  await flush();
  return t!;
}

async function send(command: Parameters<MockTransport["send"]>[0]) {
  await act(async () => {
    await mock!.send(command);
  });
}

async function key(el: Element, k: string, opts: { ctrlKey?: boolean; shiftKey?: boolean; altKey?: boolean } = {}) {
  await act(async () => {
    fireEvent.keyDown(el, { key: k, ...opts });
  });
  await flush();
}

function setPlayhead(position: number) {
  act(() => playheadStore.setPlayhead({ transport: { position, seconds: 0, playing: false, bpm: 120 } }));
}

/** Where pastes land: the selection cursor, never the playhead. */
function setCursor(position: number) {
  act(() => useTimeSelection.setState({ cursor: position }));
}

/** A double-click as the lane sees it: two presses (it acts on the second pointerdown). */
async function doublePress(el: Element, at: [number, number]) {
  for (let i = 0; i < 2; i++) {
    await act(async () => {
      fireEvent.pointerDown(el, { button: 0, clientX: at[0], clientY: at[1] });
    });
    await flush();
    await act(async () => {
      fireEvent.pointerUp(window, { clientX: at[0], clientY: at[1] });
    });
    await flush();
  }
}

async function drag(
  el: Element,
  from: [number, number],
  to: [number, number],
  opts: { altKey?: boolean; ctrlKey?: boolean; shiftKey?: boolean; release?: boolean } = {},
) {
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
  resetTimeSelection();
});

describe("copy / cut / paste / duplicate", () => {
  it("copies and pastes at the selection cursor (not the playhead) as one undo step, selecting the paste", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const [, b, c] = lanePoints(lane.id);
    act(() => itemSelection.getState().select("automationPoint", [b!.id, c!.id], "replace"));
    await key(lanes()[0]!, "c", { ctrlKey: true });
    expect(pointClipboard()?.points).toHaveLength(2);

    setCursor(32);
    setPlayhead(50);
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
    setCursor(8);
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
    act(() =>
      itemSelection.getState().select(
        "automationPoint",
        lanePoints(lane.id).map((p) => p.id),
        "replace",
      ),
    );
    await key(lanes()[0]!, "x", { ctrlKey: true });
    expect(lanePoints(lane.id)).toHaveLength(0);

    pickOption(screen.getByRole("combobox", { name: "show parameter" }), { value: `pan:${track.id}` });
    await flush();
    setCursor(4);
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

  it("Edit-menu clipboard events reach the focused lane only (one shared listener)", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const [a] = lanePoints(lane.id);
    act(() => itemSelection.getState().select("automationPoint", [a!.id], "replace"));
    // Not focused: left alone (the arrangement's clip handlers get it).
    const outside = new Event("copy", { bubbles: true, cancelable: true });
    window.dispatchEvent(outside);
    expect(outside.defaultPrevented).toBe(false);
    expect(pointClipboard()).toBeNull();
    // Focused: copied here, and stopped.
    act(() => lanes()[0]!.focus());
    const inside = new Event("copy", { bubbles: true, cancelable: true });
    window.dispatchEvent(inside);
    expect(inside.defaultPrevented).toBe(true);
    expect(pointClipboard()?.points).toHaveLength(1);
  });

  it("the value scale takes the wheel only when zoomed in or with ⌘/Ctrl", async () => {
    await renderLanes();
    const scale = screen.getByRole("scrollbar", { name: /volume value range/ });
    const plain = new WheelEvent("wheel", { deltaY: 40, bubbles: true, cancelable: true });
    scale.dispatchEvent(plain);
    expect(plain.defaultPrevented).toBe(false);
    const zoom = new WheelEvent("wheel", { deltaY: -40, ctrlKey: true, bubbles: true, cancelable: true });
    act(() => {
      scale.dispatchEvent(zoom);
    });
    expect(zoom.defaultPrevented).toBe(true);
    // Zoomed in now: a plain wheel scrolls the window.
    const scroll = new WheelEvent("wheel", { deltaY: 40, bubbles: true, cancelable: true });
    act(() => {
      scale.dispatchEvent(scroll);
    });
    expect(scroll.defaultPrevented).toBe(true);
  });

  it("offers the actions in the point menu", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const [, b] = lanePoints(lane.id);
    const circle = document.querySelector(`circle[data-point="${b!.id}"]`)!;
    const { useContextMenuStore } = await import("@/kit/contextMenuStore");
    fireEvent.contextMenu(circle);
    const labels = useContextMenuStore.getState().menu!.items.map((i) => (i === "separator" ? "-" : i.label));
    expect(labels).toEqual(expect.arrayContaining(["cut", "copy", "paste", "duplicate", "set value…", "delete point"]));
  });

  it("the point menu's hints follow the keymap: rebinding copy shows the new chord (#574)", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const [, b] = lanePoints(lane.id);
    const { useContextMenuStore } = await import("@/kit/contextMenuStore");
    const { DEFAULT_KEYMAP, useKeymapStore, withBinding } = await import("@/features/keymap/store");
    const { formatChord } = await import("@/features/keymap/chords");
    const hint = (label: string) => {
      fireEvent.contextMenu(document.querySelector(`circle[data-point="${b!.id}"]`)!);
      const item = useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === label);
      return item && item !== "separator" ? item.shortcut : undefined;
    };
    try {
      expect(hint("copy")).toBe(formatChord("Mod+C"));
      act(() => useKeymapStore.setState({ keymap: withBinding(DEFAULT_KEYMAP, "edit.copy", ["Mod+Shift+K"]) }));
      expect(hint("copy")).toBe(formatChord("Mod+Shift+K"));
      act(() => useKeymapStore.setState({ keymap: withBinding(DEFAULT_KEYMAP, "edit.delete", ["Alt+Backspace"]) }));
      expect(hint("delete point")).toBe(formatChord("Alt+Backspace"));
    } finally {
      act(() => useKeymapStore.setState({ keymap: DEFAULT_KEYMAP }));
    }
  });

  it("set value… opens a popover at the point (no modal); Enter sets every selected point in one undo step", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const [, b, c] = lanePoints(lane.id);
    act(() => itemSelection.getState().select("automationPoint", [b!.id, c!.id], "replace"));
    const { useContextMenuStore } = await import("@/kit/contextMenuStore");
    fireEvent.contextMenu(document.querySelector(`circle[data-point="${b!.id}"]`)!);
    const item = useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === "set value…");
    act(() => {
      if (item && item !== "separator") item.onSelect?.();
    });
    expect(screen.queryByRole("dialog", { name: "set volume" })?.classList.contains("eth-popover")).toBe(true);
    const anchor = document.querySelector<HTMLElement>(".eth-auto-lane__value-anchor")!;
    expect(anchor.style.left).toBe(`${8 * PX_PER_BEAT}px`);
    expect(anchor.style.top).toBe(`${y(0.9)}px`);
    const input = screen.getByRole("textbox", { name: "volume value" }) as HTMLInputElement;
    expect(input.value).toBe(String(Number(paramToPlain(VOLUME_INFO, 0.9).toFixed(2))));
    expect(document.activeElement).toBe(input);

    fireEvent.change(input, { target: { value: "-10 db" } });
    await key(input, "Enter");
    for (const p of [b!, c!]) expect(paramToPlain(VOLUME_INFO, project().automation_points[p.id]!.value)).toBeCloseTo(-10, 6);
    // The lane keeps its points (Enter didn't reach its shortcuts) and gets the focus back.
    expect(lanePoints(lane.id)).toHaveLength(3);
    expect(document.activeElement).toBe(lanes()[0]);

    await send(cmd("Edit", { type: "Undo" }));
    expect(lanePoints(lane.id).map((p) => p.value)).toEqual([0.7, 0.9, 0.6]);
  });
});

describe("value snapping", () => {
  it("shift held mid-drag snaps continuous values to whole increments (1 dB); alt keeps them free", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const [, b] = lanePoints(lane.id);
    const circle = document.querySelector(`circle[data-point="${b!.id}"]`)!;
    const dB = () => paramToPlain(VOLUME_INFO, project().automation_points[b!.id]!.value);
    const press = async (x: number, yy: number) =>
      act(async () => {
        fireEvent.pointerDown(circle, { button: 0, clientX: x, clientY: yy });
      });
    const move = async (x: number, yy: number, mods: { shiftKey?: boolean; altKey?: boolean }) => {
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

    // ⇧ held after the press: whole dB, with the hint in the tooltip.
    await press(80, y(0.9));
    await move(80, y(0.87), { shiftKey: true });
    await move(80, y(0.83), { shiftKey: true });
    const tip = screen.getByTestId("automation-drag-tip");
    expect(tip.textContent).toMatch(/db/);
    expect(tip.textContent).toContain(`${formatKeys("Shift")} 1 db steps`);
    expect(tip.textContent).toContain(`${formatKeys("Alt")} fine, off grid`);
    await release();
    expect(dB()).toBeCloseTo(Math.round(dB()), 6);
    expect(screen.queryByTestId("automation-drag-tip")).toBeNull();

    // ⌥ alone: values stay free (not whole dB). (Pressed 10 px away from the first press: two
    // quick presses in one place are a double-click, which deletes the point.)
    await press(90, y(0.83));
    await move(90, y(0.8), { altKey: true });
    await move(90, y(0.7713), { altKey: true });
    await release();
    expect(Math.abs(dB() - Math.round(dB()))).toBeGreaterThan(1e-3);

    // ⇧↑ / ⇧↓: to the next whole dB.
    const before = dB();
    act(() => itemSelection.getState().select("automationPoint", [b!.id], "replace"));
    await key(lanes()[0]!, "ArrowUp", { shiftKey: true });
    expect(dB()).toBeCloseTo(Math.ceil(before), 6);
    await key(lanes()[0]!, "ArrowDown", { shiftKey: true });
    expect(dB()).toBeCloseTo(Math.ceil(before) - 1, 6);
  });

  it("⌥ held: a precision drag, 1/10 of the pointer's motion and off the grid; no jump on press (#793)", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const [, b] = lanePoints(lane.id);
    const circle = document.querySelector(`circle[data-point="${b!.id}"]`)!;
    const usable = LANE_HEIGHT - 2 * LANE_PAD;
    const point = () => project().automation_points[b!.id]!;

    // 20 px down and 15 px right with ⌥: the value moves 2 px worth, the time 1.5 px (off the grid).
    await drag(circle, [80, y(0.9)], [95, y(0.9) + 20], { altKey: true });
    expect(point().value).toBeCloseTo(0.9 - 2 / usable, 6);
    expect(point().time).toBeCloseTo(8 + 1.5 / PX_PER_BEAT, 6);
    const v0 = point().value;

    // ⌥ pressed mid-drag: the first 10 px count in full, the 10 px after the press a tenth.
    const move = async (x: number, yy: number, altKey: boolean) => {
      await act(async () => {
        fireEvent.pointerMove(window, { clientX: x, clientY: yy, altKey });
      });
      await flush();
    };
    await act(async () => {
      fireEvent.pointerDown(circle, { button: 0, clientX: 120, clientY: 40 });
    });
    await move(120, 45, false);
    await move(120, 50, false);
    expect(point().value).toBeCloseTo(v0 - 10 / usable, 6);
    await move(120, 50, true);
    expect(point().value).toBeCloseTo(v0 - 10 / usable, 6);
    await move(120, 60, true);
    expect(point().value).toBeCloseTo(v0 - 11 / usable, 6);
    // Released: full speed again from where it is.
    await move(120, 60, false);
    expect(point().value).toBeCloseTo(v0 - 11 / usable, 6);
    await act(async () => {
      fireEvent.pointerUp(window, {});
    });
    await flush();

    // ⇧ + ⌥: ⇧'s whole increments win for the value.
    await drag(circle, [140, y(0.5)], [140, y(0.5) + 6], { altKey: true, shiftKey: true });
    const dB = paramToPlain(VOLUME_INFO, point().value);
    expect(dB).toBeCloseTo(Math.round(dB), 6);

    // ⌥↑ / ⌥↓: a tenth of the normal nudge (0.1 % of the range).
    act(() => itemSelection.getState().select("automationPoint", [b!.id], "replace"));
    const before = point().value;
    await key(lanes()[0]!, "ArrowUp", { altKey: true });
    expect(point().value).toBeCloseTo(before + 0.001, 9);
    await key(lanes()[0]!, "ArrowDown", { altKey: true });
    await key(lanes()[0]!, "ArrowDown", { altKey: true });
    expect(point().value).toBeCloseTo(before - 0.001, 9);
  });

  it("⌘/Ctrl held mid-drag locks the drag to one axis", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const [, b] = lanePoints(lane.id);
    const circle = document.querySelector(`circle[data-point="${b!.id}"]`)!;
    // Mostly vertical: the time stays.
    await drag(circle, [80, y(0.9)], [93, y(0.4)], { ctrlKey: true });
    expect(project().automation_points[b!.id]).toMatchObject({ time: 8 });
    expect(project().automation_points[b!.id]!.value).toBeCloseTo(0.4, 6);
  });

  it("stepped params (semitones) snap to whole steps and draw step lines", async () => {
    const track = await renderLanes();
    const synth = Object.values(project().devices).find((d) => d.track === track.id && d.name === "Synth")!;
    const combo = screen.getByRole("combobox", { name: "show parameter" });
    await waitFor(() => expect(combo.querySelector(`option[value="param:${synth.id}:1"]`) ?? combo).toBeTruthy());
    await waitFor(() => {
      pickOption(screen.getByRole("combobox", { name: "show parameter" }), { value: `param:${synth.id}:1` });
      expect(lanes()).toHaveLength(2);
    });
    await flush();
    const svg = screen.getAllByTestId("automation-lane-svg")[1]!;
    // Transpose: -24..24 st. The default 64 px lane opens on ±3 st (9 px per step), one line
    // per step.
    expect(svg.querySelectorAll(".eth-auto-lane__step")).toHaveLength(7);
    const st = (s: number) => LANE_PAD + (1 - (s + 3) / 6) * (LANE_HEIGHT - 2 * LANE_PAD);
    // The empty lane's first point takes the param's value (+1 st), not the pointer's.
    await send(cmd("Device", { type: "SetParam", device: synth.id, param: 1, value: 1 }));
    expect(Number(svg.querySelector('[data-testid="automation-baseline"]')!.getAttribute("y1"))).toBeCloseTo(st(1), 6);
    await doublePress(svg, [40, st(-2.3)]);
    const detune = Object.values(project().automation_lanes).find((l) => l.target.type === "DeviceParam")!;
    const [p] = lanePoints(detune.id);
    const semis = () => project().automation_points[p!.id]!.value * 48 - 24;
    expect(semis()).toBeCloseTo(1, 9);

    // A double-click once it has points: the lane's value there (held after the last point),
    // not the pointer's.
    await doublePress(svg, [200, st(-2.3)]);
    expect(lanePoints(detune.id)[1]!.value * 48 - 24).toBeCloseTo(1, 9);

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
    const grip = screen.getByRole("separator", { name: /resize volume lane/ });
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

    fireEvent.doubleClick(screen.getByRole("separator", { name: /resize volume lane/ }));
    expect(useAutomationUi.getState().laneHeights[key]).toBeUndefined();
    expect(automationHeight(useAutomationUi.getState(), track.id)).toBe(AUTOMATION_BAR_HEIGHT + LANE_HEIGHT);
  });
});

describe("unmount mid-drag (#591)", () => {
  it("ends the drag: later pointer events don't throw or edit, and the gesture closes once", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const [, b] = lanePoints(lane.id);
    const circle = document.querySelector(`circle[data-point="${b!.id}"]`)!;
    const errors: unknown[] = [];
    const onError = (e: ErrorEvent) => {
      errors.push(e.error);
      e.preventDefault();
    };
    window.addEventListener("error", onError);
    const sent = vi.spyOn(mock!, "send");
    const isEnd = (c: Command) => c.domain === "Edit" && c.command.type === "EndGesture";
    try {
      await drag(circle, [80, y(0.9)], [80, y(0.5)], { release: false });
      expect(sent.mock.calls.some(([c]) => c.domain === "Automation")).toBe(true);
      // A collaborator (or an undo) removes the lane's owner: the lane unmounts mid-drag.
      act(() => rendered!.unmount());
      await flush();
      const before = sent.mock.calls.length;
      for (const yy of [y(0.4), y(0.3)]) fireEvent.pointerMove(window, { clientX: 80, clientY: yy });
      fireEvent.pointerUp(window, { clientX: 80, clientY: y(0.3) });
      await flush();
      expect(errors).toEqual([]);
      // Nothing after the unmount: no edits for the dead lane, no second EndGesture.
      expect(sent.mock.calls.slice(before)).toEqual([]);
      expect(sent.mock.calls.filter(([c]) => isEnd(c))).toHaveLength(1);
    } finally {
      window.removeEventListener("error", onError);
    }
  });
});

describe("dragging the line", () => {
  const svgOf = () => lanes()[0]!.querySelector('[data-testid="automation-lane-svg"]')!;
  const lineY = (lane: string, t: number) => y(evaluatePoints(lanePoints(lane), t)!);

  it("a plain drag on a segment moves its two points up or down, times kept, as one undo step", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const [a, b, c] = lanePoints(lane.id);
    const at: [number, number] = [40, lineY(lane.id, 4)];
    // Down (up would clamp the pair at the top: 0.9 is near it).
    await drag(svgOf(), at, [at[0] + 7, at[1] + 10]);
    const lift = -10 / (LANE_HEIGHT - 2 * LANE_PAD);
    expect(project().automation_points[a!.id]).toMatchObject({ time: a!.time });
    expect(project().automation_points[b!.id]).toMatchObject({ time: b!.time });
    expect(project().automation_points[a!.id]!.value).toBeCloseTo(a!.value + lift, 6);
    expect(project().automation_points[b!.id]!.value).toBeCloseTo(b!.value + lift, 6);
    expect(project().automation_points[c!.id]!.value).toBe(c!.value);
    // The segment's points are the selection.
    expect([...itemSelection.getState().selected.automationPoint].sort()).toEqual([a!.id, b!.id].sort());
    await send(cmd("Edit", { type: "Undo" }));
    expect(project().automation_points[a!.id]!.value).toBe(a!.value);
    expect(project().automation_points[b!.id]!.value).toBe(b!.value);
  });

  it("the hold after the last point moves that point; a click on the line changes nothing", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const before = lanePoints(lane.id);
    const last = before[before.length - 1]!;
    await drag(svgOf(), [300, lineY(lane.id, 30)], [300, lineY(lane.id, 30)]);
    expect(lanePoints(lane.id)).toEqual(before);
    // Away from the click (two presses in one spot are a double-click: a new point).
    await drag(svgOf(), [360, lineY(lane.id, 36)], [360, lineY(lane.id, 36) + 8]);
    expect(project().automation_points[last.id]!.value).toBeLessThan(last.value);
    expect(lanePoints(lane.id).slice(0, -1)).toEqual(before.slice(0, -1));
  });

  it("away from the line, a drag is still a selection box", async () => {
    const track = await renderLanes();
    const lane = laneOf(track, "TrackVolume")!;
    const before = lanePoints(lane.id);
    await drag(svgOf(), [40, LANE_HEIGHT - 2], [100, LANE_HEIGHT - 3]);
    expect(lanePoints(lane.id)).toEqual(before);
  });
});
