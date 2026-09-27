import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AudioContent, Clip, Command } from "@/generated";
import { playheadStore, useProjectStore } from "@/state";
import { cmd, MockTransport, newId, TransportProvider } from "@/transport";
import { ContextMenuHost, type ContextMenuEntry } from "@/kit";
import { ArrangementView } from "@/features/arrangement";
import { resetArrangementUi, useArrangementUi } from "@/features/arrangement/uiStore";
import { resetAutomationUi } from "@/features/automation";
import { dragCurve, dragFadeLength, fadePaths, tensionForMidGain, withClipEditingEntries } from "./clipEditing";
import { fadeGain } from "./fades";
import { MarkerLane } from "./MarkerLane";

/** Default arrangement zoom: 24 px per beat. */
const PX = 24;
const project = () => useProjectStore.getState().project!;
let mock: MockTransport;
let sent: Command[];

const drumClip = (): Clip => Object.values(project().clips).find((c) => c.content.type === "Audio")!;
const audio = (c: Clip) => c.content as AudioContent;
const clipEl = (c: Clip) => document.querySelector<HTMLElement>(`[data-clip-id="${c.id}"]`)!;

async function flush() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

async function drag(el: Element, dx: number, dy = 0, alt = false) {
  const [x, y] = [100, 20];
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

async function menuItem(target: Element, label: string) {
  fireEvent.contextMenu(target, { clientX: 50, clientY: 50 });
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
  const ctx = new Proxy({}, { get: () => () => {} });
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockImplementation(() => ctx as unknown as CanvasRenderingContext2D);
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
    await drag(clipEl(c).querySelector("[data-testid=fade-in-handle]")!, 2 * PX);
    expect(audio(drumClip()).fade_in).toBeCloseTo(2, 6);
    await drag(clipEl(c).querySelector("[data-testid=fade-out-handle]")!, -3 * PX);
    expect(audio(drumClip()).fade_out).toBeCloseTo(3, 6);
    // The overlay draws both fades.
    expect(clipEl(c).querySelectorAll(".eth-clip-fades__line")).toHaveLength(2);
    await undo();
    expect(audio(drumClip()).fade_out).toBe(0);
    expect(audio(drumClip()).fade_in).toBeCloseTo(2, 6);
  });

  it("dragging the curve handle bends the fade; double-click resets it", async () => {
    const c = drumClip();
    await drag(clipEl(c).querySelector("[data-testid=fade-in-handle]")!, 4 * PX);
    const handle = clipEl(c).querySelector("[data-testid=fade-in-curve]")!;
    expect(handle).not.toBeNull();
    await drag(handle, 0, 10);
    const curve = audio(drumClip()).fade_in_curve;
    expect(curve.type).toBe("Curve");
    await act(async () => {
      fireEvent.doubleClick(clipEl(c).querySelector("[data-testid=fade-in-curve]")!);
    });
    await flush();
    expect(audio(drumClip()).fade_in_curve).toEqual({ type: "Linear" });
  });

  it("context menu: reverse (badge), equal-power fades, crossfade with the next clip", async () => {
    const c = drumClip();
    const title = () => clipEl(c).querySelector(".eth-clip__title")!;
    await menuItem(title(), "^Reverse$");
    expect(audio(drumClip()).reversed).toBe(true);
    expect(clipEl(c).querySelector("[data-testid=clip-reversed]")).not.toBeNull();
    await menuItem(title(), "Unreverse");
    expect(audio(drumClip()).reversed).toBe(false);
    await menuItem(title(), "Equal-Power Fades");
    expect(audio(drumClip())).toMatchObject({ fade_in_curve: { type: "EqualPower" }, fade_out_curve: { type: "EqualPower" } });

    const second = newId();
    await act(async () => {
      await mock.send(cmd("Clip", { type: "Split", id: c.id, at: 8, new_id: second }));
    });
    await flush();
    await menuItem(title(), "Crossfade with Next Clip");
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
    expect(markers()[0]).toMatchObject({ position: 4, name: "Marker 1" });

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
    const input = await screen.findByLabelText("Marker name");
    await act(async () => {
      fireEvent.change(input, { target: { value: "Chorus" } });
      fireEvent.keyDown(input, { key: "Enter" });
    });
    await flush();
    expect(markers()[0]!.name).toBe("Chorus");

    // Delete from the context menu, then undo brings it back.
    await menuItem(el(), "Delete Marker");
    expect(markers()).toHaveLength(0);
    await undo();
    expect(markers()[0]!.name).toBe("Chorus");
  });

  it("adds a marker at the playhead with +, and from the lane menu", async () => {
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Add marker at playhead" }));
    });
    await flush();
    expect(markers().map((m) => m.position)).toEqual([playheadStore.getPlayhead()?.transport.position ?? 0]);
    await menuItem(lane(), "Add Marker Here");
    expect(markers()).toHaveLength(2);
  });
});

describe("pure helpers", () => {
  it("fade paths follow the fade law", () => {
    expect(fadePaths("in", 0, 100, { type: "Linear" })).toBeNull();
    const p = fadePaths("in", 50, 100, { type: "EqualPower" })!;
    expect(p.line.startsWith("M0.00,1.0000")).toBe(true);
    expect(p.line.endsWith("L50.00,0.0000")).toBe(true);
    const o = fadePaths("out", 40, 100, { type: "Linear" })!;
    expect(o.line.startsWith("M60.00,0.0000")).toBe(true);
    expect(o.line.endsWith("L100.00,1.0000")).toBe(true);
  });

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

  it("menu entries go before the trailing Delete group", () => {
    const c = drumClip();
    const base: ContextMenuEntry[] = [{ label: "Copy", onSelect: () => {} }, "separator", { label: "Delete", onSelect: () => {} }];
    const out = withClipEditingEntries(base, mock, c);
    const labels = out.map((e) => (e === "separator" ? "-" : e.label));
    expect(labels[0]).toBe("Copy");
    expect(labels.slice(-2)).toEqual(["-", "Delete"]);
    expect(labels).toContain("Reverse");
  });
});
