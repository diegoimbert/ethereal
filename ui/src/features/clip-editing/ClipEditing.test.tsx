import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AudioContent, Clip, Command } from "@/generated";
import { useProjectStore } from "@/state";
import { cmd, newId, TransportProvider } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { ContextMenuHost, type ContextMenuEntry } from "@/kit";
import { ArrangementView } from "@/features/arrangement";
import { arrangementView, resetArrangementUi, useArrangementUi } from "@/features/arrangement/uiStore";
import { resetAutomationUi } from "@/features/automation";
import { dragCurve, dragFadeLength, tensionForMidGain, withClipEditingEntries } from "./clipEditing";
import { fadeGain } from "./fades";
import { MarkerLane } from "./MarkerLane";
import { addSignatureCommand, addTempoPointCommand } from "@/features/tempo/tempoCommands";
import { useTimeSelection } from "@/features/time-edits/store";

/** Default arrangement zoom: 24 px per beat. */
const PX = 24;
const project = () => useProjectStore.getState().project!;
let mock: MockTransport;
let sent: Command[];

const drumClip = (): Clip => Object.values(project().clips).find((c) => c.content.type === "Audio")!;
const audio = (c: Clip) => c.content as AudioContent;

interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}
/** The arranger's test API (`window.__ether.arrangement`): the canvas draws and hit-tests clips. */
const arranger = () =>
  (
    window as unknown as {
      __ether: { arrangement: { handleRect(id: string, part: string): Rect | null } };
    }
  ).__ether.arrangement;
/** The arranger's lanes: presses on them are hit-tested against the clips. */
const lanes = () => screen.getByTestId("arrangement-content");
/** The middle of a clip part (`header`, `fade-in`, `fade-in-curve`, …) in client px. */
function partAt(c: Clip, part: string): { x: number; y: number } {
  const r = arranger().handleRect(c.id, part);
  if (!r) throw new Error(`no ${part} on ${c.id}`);
  return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
}
/** Hovers a clip part (its fade handles show and hit while the clip is under the pointer). */
function hover(c: Clip, part = "body") {
  const p = partAt(c, part);
  fireEvent.pointerMove(lanes(), { buttons: 0, clientX: p.x, clientY: p.y });
}

async function flush() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

async function drag(el: Element, dx: number, dy = 0, alt = false, at: { x: number; y: number } = { x: 100, y: 20 }) {
  const { x, y } = at;
  await act(async () => {
    fireEvent.pointerDown(el, { button: 0, pointerId: 1, clientX: x, clientY: y });
  });
  await act(async () => {
    fireEvent.pointerMove(window, { pointerId: 1, clientX: x + dx / 2, clientY: y + dy / 2, altKey: alt });
    fireEvent.pointerMove(window, { pointerId: 1, clientX: x + dx, clientY: y + dy, altKey: alt });
  });
  await act(async () => {
    fireEvent.pointerUp(window, { pointerId: 1, clientX: x + dx, clientY: y + dy });
  });
  await flush();
}

async function undo() {
  await act(async () => {
    await mock.send(cmd("Edit", { type: "Undo" }));
  });
}

async function menuItem(target: Element, label: string, at: { x: number; y: number } = { x: 50, y: 50 }) {
  fireEvent.contextMenu(target, { clientX: at.x, clientY: at.y });
  await flush();
  const item = await screen.findByRole("menuitem", { name: new RegExp(label) });
  await act(async () => {
    fireEvent.click(item);
  });
  await flush();
}

beforeEach(async () => {
  resetArrangementUi();
  resetAutomationUi();
  useArrangementUi.getState().setGrid({ type: "Fixed", step: { kind: "beats", beats: 1 }, triplet: false });
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
      <MarkerLane />
      <ArrangementView />
      <ContextMenuHost />
    </TransportProvider>,
  );
  await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
  await flush();
});

afterEach(() => {
  vi.restoreAllMocks();
  mock.dispose();
});

describe("clip fades", () => {
  it("dragging the fade handles sets the fade lengths (one undo step)", async () => {
    const c = drumClip();
    // The handles show while the clip is under the pointer.
    hover(c);
    await drag(lanes(), 2 * PX, 0, false, partAt(c, "fade-in"));
    expect(audio(drumClip()).fade_in).toBeCloseTo(2, 6);
    hover(drumClip());
    await drag(lanes(), -3 * PX, 0, false, partAt(drumClip(), "fade-out"));
    expect(audio(drumClip()).fade_out).toBeCloseTo(3, 6);
    await undo();
    expect(audio(drumClip()).fade_out).toBe(0);
    expect(audio(drumClip()).fade_in).toBeCloseTo(2, 6);
  });

  it("dragging the curve handle bends the fade; double-click resets it", async () => {
    const c = drumClip();
    hover(c);
    await drag(lanes(), 4 * PX, 0, false, partAt(c, "fade-in"));
    hover(drumClip());
    await drag(lanes(), 0, 10, false, partAt(drumClip(), "fade-in-curve"));
    const curve = audio(drumClip()).fade_in_curve;
    expect(curve.type).toBe("Curve");
    hover(drumClip());
    const mid = partAt(drumClip(), "fade-in-curve");
    await act(async () => {
      // The double-click's second press, where it's released.
      fireEvent.pointerDown(lanes(), { button: 0, pointerId: 1, clientX: mid.x, clientY: mid.y });
      fireEvent.pointerUp(window, { pointerId: 1, clientX: mid.x, clientY: mid.y });
      fireEvent.doubleClick(lanes(), { clientX: mid.x, clientY: mid.y });
    });
    await flush();
    expect(audio(drumClip()).fade_in_curve).toEqual({ type: "Linear" });
  });

  it("context menu: reverse, equal-power fades, crossfade with the next clip", async () => {
    const c = drumClip();
    const title = () => partAt(drumClip(), "header");
    await menuItem(lanes(), "^reverse$", title());
    expect(audio(drumClip()).reversed).toBe(true);
    await menuItem(lanes(), "unreverse", title());
    expect(audio(drumClip()).reversed).toBe(false);
    await menuItem(lanes(), "equal-power fades", title());
    expect(audio(drumClip())).toMatchObject({ fade_in_curve: { type: "EqualPower" }, fade_out_curve: { type: "EqualPower" } });

    const second = newId();
    await act(async () => {
      await mock.send(cmd("Clip", { type: "Split", id: c.id, at: 8, new_id: second }));
    });
    await flush();
    await menuItem(lanes(), "crossfade with next clip", title());
    const [a, b] = [project().clips[c.id]!, project().clips[second]!];
    expect(a.start + a.length).toBeCloseTo(8.25, 6);
    expect(b.start).toBeCloseTo(7.75, 6);
    expect(audio(a).fade_out).toBeCloseTo(0.5, 6);
    expect(audio(b).fade_in).toBeCloseTo(0.5, 6);
  });
});

describe("MarkerLane", () => {
  const lane = () => screen.getByTestId("marker-lane-area");
  const markers = () => Object.values(project().markers);

  it("adds (double-click, snapped), jumps to, moves, renames and deletes markers", async () => {
    await act(async () => {
      fireEvent.doubleClick(lane(), { clientX: 4.4 * PX });
    });
    await flush();
    expect(markers()).toHaveLength(1);
    expect(markers()[0]).toMatchObject({ position: 4, name: "marker 1" });

    // Click = jump.
    sent = [];
    const el = () => screen.getByTestId("marker");
    await drag(el(), 0);
    expect(sent).toContainEqual(cmd("Transport", { type: "Locate", position: 4 }));

    // Drag = move (grid-snapped), one undo step.
    await drag(el(), 2.2 * PX);
    expect(markers()[0]!.position).toBe(6);
    await undo();
    expect(markers()[0]!.position).toBe(4);

    // Rename.
    await act(async () => {
      fireEvent.doubleClick(el());
    });
    const input = await screen.findByLabelText("marker name");
    await act(async () => {
      fireEvent.change(input, { target: { value: "Chorus" } });
      fireEvent.keyDown(input, { key: "Enter" });
    });
    await flush();
    expect(markers()[0]!.name).toBe("Chorus");

    // Delete from the context menu, then undo brings it back.
    await menuItem(el(), "delete marker");
    expect(markers()).toHaveLength(0);
    await undo();
    expect(markers()[0]!.name).toBe("Chorus");
  });

  it("its right-click is the ruler's menu: tempo and signature changes, then a marker; a tempo change adds there", async () => {
    fireEvent.contextMenu(lane(), { clientX: 8.2 * PX, clientY: 5 });
    await flush();
    const labels = (await screen.findAllByRole("menuitem")).map((el) => el.textContent);
    expect(labels).toEqual(["add tempo change here", "add time signature change here", "add marker here"]);
    const before = Object.keys(project().tempo_points).length;
    await act(async () => {
      fireEvent.click(screen.getByRole("menuitem", { name: "add tempo change here" }));
    });
    await flush();
    expect(Object.keys(project().tempo_points)).toHaveLength(before + 1);
  });

  it("drags time signature and tempo changes on the arrangement grid; Alt drags freely", async () => {
    // A half-beat grid: finer than a beat, so a signature change snaps to it.
    useArrangementUi.getState().setGrid({ type: "Fixed", step: { kind: "beats", beats: 0.5 }, triplet: false });
    const sigId = newId();
    const tempoId = newId();
    await act(async () => {
      await mock.send(addSignatureCommand(8, { numerator: 3, denominator: 4 }, sigId));
      await mock.send(addTempoPointCommand(4, 140, "Step", tempoId));
    });
    // jsdom lays nothing out: give the view a width so the chips render.
    act(() => arrangementView.getState().setWidth(2000));
    await flush();
    const sigEl = () => document.querySelector(`[data-signature="${sigId}"]`)!;
    const tempoEl = () => document.querySelector(`[data-tempo-point="${tempoId}"]`)!;
    const sigTime = () => project().time_signatures[sigId]!.time;

    // Snapped by default (was free: the lane passed no grid step).
    await drag(sigEl(), 2.3 * PX);
    expect(sigTime()).toBe(10.5);
    // Alt: free.
    await drag(sigEl(), 1.15 * PX, 0, true);
    expect(sigTime()).toBeCloseTo(11.65, 6);

    // The same for a tempo change (before the signature change: the grid is bar-relative).
    await drag(tempoEl(), 1.3 * PX);
    expect(project().tempo_points[tempoId]!.time).toBe(5.5);
    await drag(tempoEl(), 0.3 * PX, 0, true);
    expect(project().tempo_points[tempoId]!.time).toBeCloseTo(5.8, 6);
  });

  it("adds a marker at the selection cursor with +, and from the lane menu", async () => {
    act(() => useTimeSelection.setState({ cursor: 6 }));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /^add marker at the cursor/ }));
    });
    await flush();
    expect(markers().map((m) => m.position)).toEqual([6]);
    await menuItem(lane(), "add marker here");
    expect(markers()).toHaveLength(2);
  });
});

describe("pure helpers", () => {
  it("fade drags clamp so fades don't cross; curve drags map to tension", () => {
    expect(dragFadeLength("in", 1, 48, 24, 8, 0)).toBe(3);
    expect(dragFadeLength("in", 1, -480, 24, 8, 0)).toBe(0);
    expect(dragFadeLength("in", 1, 4800, 24, 8, 2)).toBe(6);
    expect(dragFadeLength("out", 1, -48, 24, 8, 0)).toBe(3);
    for (const t of [-0.8, -0.3, 0, 0.4, 0.9]) {
      expect(tensionForMidGain(fadeGain({ type: "Curve", tension: t }, 0.5))).toBeCloseTo(t, 5);
    }
    expect(dragCurve({ type: "Linear" }, 0.25)).toEqual({ type: "Curve", tension: 0.5 });
    expect(dragCurve({ type: "Linear" }, 5)).toEqual({ type: "Curve", tension: 1 });
  });

  it("context menu: a MIDI clip has Reverse, which mirrors its notes", async () => {
    const midi = Object.values(project().clips).find((c) => c.content.type === "Midi")!;
    const notes = () =>
      Object.values(project().notes)
        .filter((n) => n.clip === midi.id)
        .map((n) => n.start);
    const before = notes();
    await menuItem(lanes(), "^reverse$", partAt(midi, "header"));
    expect(sent.some((c) => c.domain === "Note" && c.command.type === "Reverse" && c.command.clip === midi.id)).toBe(true);
    expect(notes()).not.toEqual(before);
    await undo();
    expect(notes()).toEqual(before);
  });

  it("menu entries go before the trailing Delete group", () => {
    const c = drumClip();
    const base: ContextMenuEntry[] = [{ label: "Copy", onSelect: () => {} }, "separator", { label: "Delete", onSelect: () => {} }];
    const out = withClipEditingEntries(base, mock, c);
    const labels = out.map((e) => (e === "separator" ? "-" : e.label));
    expect(labels[0]).toBe("Copy");
    expect(labels.slice(-2)).toEqual(["-", "Delete"]);
    expect(labels).toContain("reverse");
  });
});
