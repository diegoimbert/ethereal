import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { pickOption } from "@/kit/testing";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { TempoPoint, TimeSignaturePoint } from "@/generated";
import { useContextMenuStore, type ContextMenuItem } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { createTimelineViewStore, Ruler } from "@/timeline";
import { MetronomeSettings } from "./MetronomeSettings";
import { TempoEditor } from "./TempoEditor";
import {
  bpmRange,
  bpmToY,
  snapSignatureTime,
  sortedSignatures,
  sortedTempoPoints,
  tempoPath,
  type LaneGeom,
} from "./tempoCommands";

let mock: MockTransport | undefined;
const store = () => useProjectStore.getState();
const project = () => store().project!;
const tempoPoints = () => sortedTempoPoints(project());
const signatures = () => sortedSignatures(project());

afterEach(() => {
  vi.restoreAllMocks();
  mock?.dispose();
  mock = undefined;
  store().reset();
  useContextMenuStore.getState().close();
});

async function setup(ui: React.ReactNode) {
  mock = new MockTransport({ timers: "manual", seed: 3 });
  render(<TransportProvider transport={mock}>{ui}</TransportProvider>);
  await waitFor(() => expect(store().project).not.toBeNull());
}

async function flush() {
  await act(async () => {
    for (let i = 0; i < 20; i++) await Promise.resolve();
  });
}

const tp = (id: string, time: number, bpm: number, curve: "Step" | "Linear" = "Step"): TempoPoint => ({ id, time, bpm, curve });
const ts = (id: string, time: number, numerator: number, denominator: number): TimeSignaturePoint => ({
  id,
  time,
  signature: { numerator, denominator },
});

describe("tempo helpers", () => {
  it("snaps signature changes anywhere on the grid (a beat, finer if the grid is)", () => {
    const sigs = [ts("a", 0, 4, 4), ts("b", 8, 7, 8)];
    expect(snapSignatureTime(sigs, 5.1)).toBe(5); // a 4/4 beat, mid bar 2
    expect(snapSignatureTime(sigs, 0.2)).toBeNull(); // never onto beat 0
    expect(snapSignatureTime(sigs, 7.9)).toBeNull(); // taken by "b"
    expect(snapSignatureTime(sigs, 9.7)).toBe(9.5); // eighth-note beats of the 7/8
    // A finer grid (1/16) wins; a coarser one (bars) still allows any beat.
    expect(snapSignatureTime(sigs, 2.6, { step: { kind: "beats", beats: 0.25 } })).toBe(2.5);
    expect(snapSignatureTime(sigs, 2.6, { step: { kind: "bars", bars: 1 } })).toBe(3);
    expect(snapSignatureTime(sigs, 2.6, { step: null })).toBe(2.6); // grid off
    expect(snapSignatureTime(sigs, 2.6, { free: true })).toBe(2.6); // Alt
    // Moving "b": it snaps in the 4/4 bars of the map without it.
    expect(snapSignatureTime(sigs, 9.7, { except: "b" })).toBe(10);
    // Beats count from each bar line: a partial bar from 2.5 shifts the grid.
    const mid = [ts("a", 0, 4, 4), ts("b", 2.5, 3, 4), ts("c", 20, 2, 4)];
    expect(snapSignatureTime(mid, 4.4)).toBe(4.5);
  });

  it("lane range and path follow steps and ramps", () => {
    const pts = [tp("a", 0, 120, "Linear"), tp("b", 4, 60), tp("c", 8, 90)];
    const range = bpmRange(pts);
    expect(range.min).toBeLessThan(60);
    expect(range.max).toBeGreaterThan(120);
    const g: LaneGeom = { height: 100, pad: 0, range: { min: 0, max: 200 } };
    expect(bpmToY(100, g)).toBe(50);
    const path = tempoPath(pts, (b) => b * 10, g, 200);
    // Ramp a→b is a straight line; b holds until c; c holds to the end.
    expect(path).toBe("M0.0,40.0 L0.0,40.0 L40.0,70.0 L40.0,70.0 L80.0,70.0 L80.0,55.0 L200.0,55.0");
  });
});

describe("TempoEditor", () => {
  it("adds, drags (one undo step) and removes tempo points", async () => {
    await setup(<TempoEditor />);
    const svg = screen.getByTestId("tempo-lane-svg");
    // jsdom: the lane sits at x=0, 12 px per beat → x=96 is beat 8.
    fireEvent.doubleClick(svg, { clientX: 96, clientY: 10 });
    await waitFor(() => expect(tempoPoints()).toHaveLength(2));
    const added = tempoPoints()[1]!;
    expect(added.time).toBe(8);
    expect(added.bpm).toBeGreaterThan(120);

    // The new point is selected: its BPM field edits it.
    const bpm = await screen.findByRole("spinbutton", { name: "Tempo point BPM" });
    fireEvent.change(bpm, { target: { value: "95" } });
    fireEvent.keyDown(bpm, { key: "Enter" });
    await waitFor(() => expect(project().tempo_points[added.id]!.bpm).toBe(95));

    // Drag right by 4 beats and down: one gesture.
    const circle = document.querySelector(`[data-point="${added.id}"]`)!;
    fireEvent.pointerDown(circle, { button: 0, clientX: 96, clientY: 20 });
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 120, clientY: 25 }) as PointerEvent);
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 144, clientY: 30 }) as PointerEvent);
    });
    await flush();
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointerup", { clientX: 144, clientY: 30 }) as PointerEvent);
    });
    await flush();
    const moved = project().tempo_points[added.id]!;
    expect(moved.time).toBe(12);
    expect(moved.bpm).toBeLessThan(95);
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    expect(project().tempo_points[added.id]).toMatchObject({ time: 8, bpm: 95 });

    // Ramp from the point's menu, then delete it from the header.
    fireEvent.contextMenu(document.querySelector(`[data-point="${added.id}"]`)!, { clientX: 1, clientY: 1 });
    const menu = useContextMenuStore.getState().menu!;
    const ramp = menu.items.find((i) => i !== "separator" && i.label === "Ramp to Next Point") as ContextMenuItem;
    act(() => ramp.onSelect());
    await waitFor(() => expect(project().tempo_points[added.id]!.curve).toBe("Linear"));
    fireEvent.click(screen.getByRole("button", { name: "Delete tempo point" }));
    await waitFor(() => expect(tempoPoints()).toHaveLength(1));
  });

  it("Shift-drag changes BPM in fine 0.01 steps, like the transport bar's tempo drag", async () => {
    await setup(<TempoEditor />);
    const zero = tempoPoints()[0]!;
    const circle = document.querySelector(`[data-point="${zero.id}"]`)!;
    fireEvent.pointerDown(circle, { button: 0, clientX: 0, clientY: 20, shiftKey: true });
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 0, clientY: 11, shiftKey: true }) as PointerEvent);
    });
    await flush();
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointerup", { clientX: 0, clientY: 11, shiftKey: true }) as PointerEvent);
    });
    await flush();
    // 9 px up at 0.05 BPM per pixel.
    expect(project().tempo_points[zero.id]!.bpm).toBeCloseTo(zero.bpm + 0.45, 9);
  });

  it("the point at beat 0 only changes BPM and can't be deleted", async () => {
    await setup(<TempoEditor />);
    const zero = tempoPoints()[0]!;
    const circle = document.querySelector(`[data-point="${zero.id}"]`)!;
    fireEvent.pointerDown(circle, { button: 0, clientX: 0, clientY: 20 });
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 60, clientY: 10 }) as PointerEvent);
    });
    await flush();
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointerup", { clientX: 60, clientY: 10 }) as PointerEvent);
    });
    await flush();
    expect(project().tempo_points[zero.id]!.time).toBe(0);
    expect(project().tempo_points[zero.id]!.bpm).toBeGreaterThan(zero.bpm);
    expect(screen.queryByRole("button", { name: "Delete tempo point" })).toBeNull();
    fireEvent.doubleClick(document.querySelector(`[data-point="${zero.id}"]`)!);
    await flush();
    expect(tempoPoints()).toHaveLength(1);
  });

  it("adds, edits, moves and removes time signatures", async () => {
    await setup(<TempoEditor />);
    const lane = screen.getByTestId("signature-lane");
    // Beat 7 → a 4/4 beat, mid bar 2 (a partial bar).
    fireEvent.doubleClick(lane, { clientX: 84 });
    await waitFor(() => expect(signatures()).toHaveLength(2));
    const added = signatures()[1]!;
    expect(added.time).toBe(7);

    const num = await screen.findByRole("spinbutton", { name: "Beats per bar" });
    fireEvent.change(num, { target: { value: "7" } });
    fireEvent.keyDown(num, { key: "Enter" });
    await waitFor(() => expect(project().time_signatures[added.id]!.signature.numerator).toBe(7));
    pickOption(screen.getByRole("combobox", { name: "Beat unit" }), { value: "8" });
    await waitFor(() => expect(project().time_signatures[added.id]!.signature).toEqual({ numerator: 7, denominator: 8 }));

    // Drag it left: it lands on a 4/4 beat (7 - 46 px / 12 px per beat → 3.17 → 3).
    const marker = document.querySelector(`[data-signature="${added.id}"]`)!;
    fireEvent.pointerDown(marker, { button: 0, clientX: 96, clientY: 5 });
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 50, clientY: 5 }) as PointerEvent);
    });
    await flush();
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointerup", { clientX: 50, clientY: 5 }) as PointerEvent);
    });
    await flush();
    expect(project().time_signatures[added.id]!.time).toBe(3);

    fireEvent.contextMenu(document.querySelector(`[data-signature="${added.id}"]`)!, { clientX: 1, clientY: 1 });
    const items = useContextMenuStore.getState().menu!.items.filter((i) => i !== "separator") as ContextMenuItem[];
    act(() => items.find((i) => i.label === "3/4")!.onSelect());
    await waitFor(() => expect(project().time_signatures[added.id]!.signature.numerator).toBe(3));
    act(() => useContextMenuStore.getState().close());
    fireEvent.click(screen.getByRole("button", { name: "Delete time signature" }));
    await waitFor(() => expect(signatures()).toHaveLength(1));
  });
});

describe("ruler tempo editing", () => {
  it("shows markers and adds changes from the ruler's menu", async () => {
    const view = createTimelineViewStore({ pxPerBeat: 10 });
    await setup(<Ruler view={view} syncWidth={false} />);
    act(() => view.getState().setWidth(800));
    const ruler = screen.getByTestId("ruler");
    await waitFor(() => expect(ruler.querySelector("[data-tempo-point]")).not.toBeNull());
    expect(ruler.querySelector("[data-signature]")!.textContent).toBe("4/4");

    fireEvent.contextMenu(ruler, { clientX: 160, clientY: 5 });
    let items = useContextMenuStore.getState().menu!.items.filter((i) => i !== "separator") as ContextMenuItem[];
    act(() => items.find((i) => i.label === "Add Tempo Change Here")!.onSelect());
    await waitFor(() => expect(tempoPoints()).toHaveLength(2));
    expect(tempoPoints()[1]!.time).toBe(16);

    fireEvent.contextMenu(ruler, { clientX: 130, clientY: 5 });
    items = useContextMenuStore.getState().menu!.items.filter((i) => i !== "separator") as ContextMenuItem[];
    act(() => items.find((i) => i.label === "Add Time Signature Change Here")!.onSelect());
    await waitFor(() => expect(signatures()).toHaveLength(2));
    // Beat 13 (the ruler's grid is a bar here, but a signature change snaps to a beat).
    expect(signatures()[1]!.time).toBe(13);

    // Drag the tempo marker 4 beats right (to 20): one undo step. It snaps to the ruler's
    // bar grid, which restarts at the mid-bar change at 13 (bars at 13, 17, 21).
    const id = tempoPoints()[1]!.id;
    const marker = ruler.querySelector(`[data-tempo-point="${id}"]`)!;
    fireEvent.pointerDown(marker, { button: 0, clientX: 160 });
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 200 }) as PointerEvent);
    });
    await flush();
    await act(async () => {
      window.dispatchEvent(new MouseEvent("pointerup", { clientX: 200 }) as PointerEvent);
    });
    await flush();
    expect(project().tempo_points[id]!.time).toBe(21);
    expect(store().transport?.playing ?? false).toBe(false);
  });

  it("is off on rulers with their own tempo map", async () => {
    const view = createTimelineViewStore({ pxPerBeat: 10 });
    await setup(<Ruler view={view} showLoop={false} syncWidth={false} />);
    expect(screen.queryByTestId("ruler-tempo")).toBeNull();
  });
});

describe("MetronomeSettings", () => {
  it("toggles the metronome and edits volume, accent and sound", async () => {
    await setup(<MetronomeSettings />);
    fireEvent.click(await screen.findByRole("button", { name: "Metronome settings" }));
    const on = await screen.findByRole("switch", { name: "Metronome" });
    fireEvent.click(on);
    await waitFor(() => expect(project().settings.metronome).toBe(true));

    const volume = screen.getByRole("spinbutton", { name: "Metronome volume" });
    fireEvent.change(volume, { target: { value: "-12" } });
    fireEvent.keyDown(volume, { key: "Enter" });
    await waitFor(() => expect(project().settings.metronome_volume).toBe(-12));

    fireEvent.click(screen.getByRole("switch", { name: "Accent downbeat" }));
    await waitFor(() => expect(project().settings.metronome_accent).toBe(false));

    pickOption(screen.getByRole("combobox", { name: "Metronome sound" }), { value: "Beep" });
    await waitFor(() => expect(project().settings.metronome_sound).toBe("Beep"));

    // Each change is its own undo step.
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    expect(project().settings.metronome_sound).toBe("Classic");
    expect(project().settings.metronome_accent).toBe(false);
  });
});
