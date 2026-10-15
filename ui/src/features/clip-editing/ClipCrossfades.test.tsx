/** Crossfade gestures on arrangement clips (`crossfades` node), against the MockTransport. */
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AudioContent, Clip, Command } from "@/generated";
import { useProjectStore } from "@/state";
import { cmd, MockTransport, newId, TransportProvider } from "@/transport";
import { ContextMenuHost } from "@/kit";
import { ArrangementView } from "@/features/arrangement";
import { resetArrangementUi, useArrangementUi } from "@/features/arrangement/uiStore";
import { resetAutomationUi } from "@/features/automation";
import { crossfadeDragLength, crossfadeSide, isCrossfade } from "./clipEditing";

/** Default arrangement zoom: 24 px per beat. */
const PX = 24;
const project = () => useProjectStore.getState().project!;
let mock: MockTransport;
let sent: Command[];

const audio = (c: Clip) => c.content as AudioContent;
type Point = { x: number; y: number };
/** The arranger's lanes: presses on them are hit-tested against the clips (the canvas). */
const lanes = () => screen.getByTestId("arrangement-content");
/** The middle of a clip part (`header`, `body`, `fade-in`, `fade-out-curve`, …) in client px. */
function partAt(id: string, part: string): Point {
  const api = (
    window as unknown as { __ether: { arrangement: { handleRect(id: string, part: string): { x: number; y: number; width: number; height: number } | null } } }
  ).__ether.arrangement;
  const r = api.handleRect(id, part);
  if (!r) throw new Error(`no ${part} on ${id}`);
  return { x: r.x + r.width / 2, y: r.y + r.height / 2 };
}
/** A fade handle's point (handles show while the clip is hovered or selected: hovers it first). */
const handle = (id: string, which: "fade-in" | "fade-out" | "fade-in-curve" | "fade-out-curve"): Point => {
  const body = partAt(id, "body");
  fireEvent.pointerMove(lanes(), { buttons: 0, clientX: body.x, clientY: body.y });
  return partAt(id, which);
};

async function flush() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

/**
 * Press a handle at `at`, drag through `steps` (px offsets from beat `anchor`, where the
 * handle's fade or crossfade edge is: the drag follows the beat under the pointer), release.
 */
async function drag(at: Point, anchor: number, steps: number[]) {
  const { x, y } = at;
  const x0 = useArrangementUi.getState().headerWidth + anchor * PX;
  await act(async () => {
    fireEvent.pointerMove(lanes(), { buttons: 0, clientX: x, clientY: y });
    fireEvent.pointerDown(lanes(), { button: 0, pointerId: 1, clientX: x, clientY: y });
  });
  for (const dx of steps) {
    await act(async () => {
      fireEvent.pointerMove(window, { pointerId: 1, clientX: x0 + dx, clientY: y });
    });
    await flush();
  }
  await act(async () => {
    fireEvent.pointerUp(window, { pointerId: 1, clientX: x0 + steps[steps.length - 1]!, clientY: y });
  });
  await flush();
}

async function undo() {
  await act(async () => {
    await mock.send(cmd("Edit", { type: "Undo" }));
  });
  await flush();
}

async function menuItem(at: Point, label: string) {
  fireEvent.contextMenu(lanes(), { clientX: at.x, clientY: at.y });
  await flush();
  const item = await screen.findByRole("menuitem", { name: new RegExp(label) });
  await act(async () => {
    fireEvent.click(item);
  });
  await flush();
}

/** The demo drum clip split at beat 8: two touching audio clips `[a, b)`. */
async function splitDrums(): Promise<[string, string]> {
  const a = Object.values(project().clips).find((c) => c.content.type === "Audio")!;
  const b = newId();
  await act(async () => {
    await mock.send(cmd("Clip", { type: "SetBounds", id: a.id, start: 0, length: 16, offset: 0 }));
    await mock.send(cmd("Clip", { type: "Split", id: a.id, at: 8, new_id: b }));
  });
  await flush();
  return [a.id, b];
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
      <ArrangementView />
      <ContextMenuHost />
    </TransportProvider>,
  );
  await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
  await flush();
  // jsdom lays nothing out: the arranger canvas starts after the header column, as in a browser.
  const canvas = document.querySelector<HTMLCanvasElement>(".eth-arr-gl__canvas")!;
  vi.spyOn(canvas, "getBoundingClientRect").mockImplementation(() => {
    const left = useArrangementUi.getState().headerWidth;
    return { x: left, y: 0, left, top: 0, right: left, bottom: 0, width: 0, height: 0, toJSON: () => ({}) };
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  mock.dispose();
});

describe("crossfade gestures", () => {
  it("dragging a fade-out handle across the boundary creates a crossfade; one undo removes it", async () => {
    const [a, b] = await splitDrums();
    // From the end of A, 1 beat past the boundary: an equal-power crossfade of 2 beats.
    await drag(handle(a, "fade-out"), 8, [0.5 * PX, 1 * PX]);
    const [ca, cb] = [project().clips[a]!, project().clips[b]!];
    expect(ca.start + ca.length).toBeCloseTo(9, 6);
    expect(cb.start).toBeCloseTo(7, 6);
    expect(audio(ca)).toMatchObject({ fade_out: 2, fade_out_curve: { type: "EqualPower" } });
    expect(audio(cb)).toMatchObject({ fade_in: 2, fade_in_curve: { type: "EqualPower" } });
    expect(isCrossfade(ca, cb)).toBe(true);
    // Every command of the drag shared one gesture: one undo restores the split.
    expect(sent.filter((c) => c.domain === "Clip" && c.command.type === "SetCrossfade").length).toBeGreaterThanOrEqual(1);
    await undo();
    expect(project().clips[a]!.length).toBeCloseTo(8, 6);
    expect(project().clips[b]!.start).toBeCloseTo(8, 6);
    expect(audio(project().clips[a]!).fade_out).toBe(0);
  });

  it("the fade-in handle of the later clip dragged left also creates one", async () => {
    const [a, b] = await splitDrums();
    await drag(handle(b, "fade-in"), 8, [-0.5 * PX, -1.5 * PX]);
    const [ca, cb] = [project().clips[a]!, project().clips[b]!];
    expect(cb.start).toBeCloseTo(6.5, 6);
    expect(ca.start + ca.length).toBeCloseTo(9.5, 6);
  });

  it("resizes symmetrically around the boundary, and dragging back to it removes the crossfade", async () => {
    const [a, b] = await splitDrums();
    await act(async () => {
      await mock.send(cmd("Clip", { type: "SetCrossfade", first: a, second: b, at: 8, length: 2, curve: { type: "EqualPower" } }));
    });
    await flush();
    // B's fade-in handle is the crossfade's right edge (beat 9): drag it to 9.5 → 3 beats.
    await drag(handle(b, "fade-in"), 9, [0.5 * PX]);
    expect(project().clips[b]!.start).toBeCloseTo(6.5, 6);
    expect(project().clips[a]!.start + project().clips[a]!.length).toBeCloseTo(9.5, 6);
    // Dragged back to the boundary (8): removed. (A's fade-out handle, the left edge, is under
    // B there: B's is the one to grab.)
    await drag(handle(b, "fade-in"), 9.5, [-0.75 * PX, -1.5 * PX]);
    const [ca, cb] = [project().clips[a]!, project().clips[b]!];
    expect(ca.length).toBeCloseTo(8, 6);
    expect(cb.start).toBeCloseTo(8, 6);
    expect(audio(ca).fade_out).toBe(0);
    expect(audio(cb).fade_in).toBe(0);
    // One undo per gesture: back to the 3-beat crossfade.
    await undo();
    expect(project().clips[b]!.start).toBeCloseTo(6.5, 6);
  });

  it("the curve handle bends both fades of a crossfade", async () => {
    const [a, b] = await splitDrums();
    await act(async () => {
      await mock.send(cmd("Clip", { type: "SetCrossfade", first: a, second: b, at: 8, length: 2, curve: { type: "Linear" } }));
    });
    await flush();
    const curve = handle(b, "fade-in-curve");
    await act(async () => {
      fireEvent.doubleClick(lanes(), { clientX: curve.x, clientY: curve.y });
    });
    await flush();
    await act(async () => {
      fireEvent.pointerDown(lanes(), { button: 0, pointerId: 1, clientX: curve.x, clientY: curve.y });
    });
    await act(async () => {
      fireEvent.pointerMove(window, { pointerId: 1, clientX: curve.x, clientY: curve.y + 10 });
      fireEvent.pointerUp(window, { pointerId: 1, clientX: curve.x, clientY: curve.y + 10 });
    });
    await flush();
    const [ca, cb] = [project().clips[a]!, project().clips[b]!];
    expect(audio(ca).fade_out_curve.type).toBe("Curve");
    expect(audio(cb).fade_in_curve).toEqual(audio(ca).fade_out_curve);
    // The crossfade's bounds are untouched.
    expect(cb.start).toBeCloseTo(7, 6);
  });

  it("context menu: create and remove the crossfade", async () => {
    const [a, b] = await splitDrums();
    const title = (id: string) => partAt(id, "header");
    await menuItem(title(a), "crossfade with next clip");
    expect(isCrossfade(project().clips[a]!, project().clips[b]!)).toBe(true);
    await menuItem(title(b), "remove crossfade with previous clip");
    expect(project().clips[a]!.length).toBeCloseTo(8, 6);
    expect(project().clips[b]!.start).toBeCloseTo(8, 6);
  });
});

describe("crossfade helpers", () => {
  it("drag length: twice the pointer's distance to the centre on the grabbed side, 0 near it", () => {
    expect(crossfadeDragLength(8, 9, false, PX)).toBe(2);
    expect(crossfadeDragLength(8, 7, true, PX)).toBe(2);
    expect(crossfadeDragLength(8, 9, true, PX)).toBe(0);
    expect(crossfadeDragLength(8, 8.05, false, PX)).toBe(0); // 2.4 px < 4 px
  });

  it("finds touching and crossfaded neighbours", async () => {
    const [a, b] = await splitDrums();
    const p = () => project();
    expect(crossfadeSide(p(), p().clips[a]!, "out")).toMatchObject({ crossfaded: false, centre: 8 });
    expect(crossfadeSide(p(), p().clips[a]!, "in")).toBeNull();
    await act(async () => {
      await mock.send(cmd("Clip", { type: "SetCrossfade", first: a, second: b, at: 8, length: 1, curve: null }));
    });
    expect(crossfadeSide(p(), p().clips[b]!, "in")).toMatchObject({ crossfaded: true, centre: 8, length: 1 });
  });
});
