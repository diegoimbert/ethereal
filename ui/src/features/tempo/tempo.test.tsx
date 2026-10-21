import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { pickOption } from "@/kit/testing";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { TempoPoint, TimeSignaturePoint } from "@/generated";
import { useContextMenuStore, type ContextMenuItem } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, TransportProvider } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { createTimelineViewStore, itemSelection } from "@/timeline";
import { resetTimeSelection, useTimeSelection } from "@/features/time-edits/store";
import { TempoRuler as Ruler } from "./TempoRuler";
import { MetronomePanel } from "./MetronomeSettings";
import { signatureMenu } from "./menus";
import { TempoAutomationLane } from "./TempoAutomationLane";
import {
  addTempoPointCommand,
  bpmRange,
  bpmToY,
  editTempoPointCommand,
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
  const r = render(<TransportProvider transport={mock}>{ui}</TransportProvider>);
  await waitFor(() => expect(store().project).not.toBeNull());
  return r;
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

/** The master's tempo lane, as the arrangement mounts it (12 px per beat here). */
function Lane() {
  const view = createTimelineViewStore({ pxPerBeat: 12 });
  return <TempoAutomationLane view={view} height={120} />;
}

describe("TempoAutomationLane", () => {
  it("adds, drags (one undo step) and removes tempo points", async () => {
    await setup(<Lane />);
    const svg = screen.getByTestId("tempo-lane-svg");
    // jsdom: the lane sits at x=0, 12 px per beat → x=96 is beat 8.
    fireEvent.doubleClick(svg, { clientX: 96, clientY: 10 });
    await waitFor(() => expect(tempoPoints()).toHaveLength(2));
    const added = tempoPoints()[1]!;
    expect(added.time).toBe(8);
    // The tempo there (not the pointer's y): the map doesn't change.
    expect(added.bpm).toBe(120);
    expect(added.curve).toBe("Step");
    // The new point is selected.
    await waitFor(() => expect(document.querySelector(`[data-point="${added.id}"]`)!.getAttribute("aria-selected")).toBe("true"));

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
    expect(moved.bpm).toBeLessThan(added.bpm);
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    expect(project().tempo_points[added.id]).toMatchObject({ time: 8, bpm: added.bpm });

    // Ramp from the point's menu, then delete it with the Delete key.
    fireEvent.contextMenu(document.querySelector(`[data-point="${added.id}"]`)!, { clientX: 1, clientY: 1 });
    const menu = useContextMenuStore.getState().menu!;
    const ramp = menu.items.find((i) => i !== "separator" && i.label === "ramp to next point") as ContextMenuItem;
    act(() => ramp.onSelect());
    await waitFor(() => expect(project().tempo_points[added.id]!.curve).toBe("Linear"));
    fireEvent.keyDown(screen.getByTestId("tempo-lane"), { key: "Delete" });
    await waitFor(() => expect(tempoPoints()).toHaveLength(1));
  });

  it("a point added on a ramp takes the tempo there and keeps ramping: the map doesn't change", async () => {
    await setup(<Lane />);
    const zero = tempoPoints()[0]!;
    await act(async () => {
      await mock!.send(addTempoPointCommand(16, 160, "Step"));
      await mock!.send(editTempoPointCommand(zero.id, { curve: "Linear" }));
    });
    const before = tempoPoints();
    // x=96: beat 8, half way up the 120 → 160 ramp, whatever the pointer's y.
    fireEvent.doubleClick(screen.getByTestId("tempo-lane-svg"), { clientX: 96, clientY: 110 });
    await waitFor(() => expect(tempoPoints()).toHaveLength(3));
    expect(tempoPoints()[1]).toMatchObject({ time: 8, bpm: 140, curve: "Linear" });
    for (const p of before) expect(project().tempo_points[p.id]).toEqual(p);
  });

  it("unmounting mid-drag ends the drag (no throw, no edits, one EndGesture) (#591)", async () => {
    const r = await setup(<Lane />);
    const zero = tempoPoints()[0]!;
    const circle = document.querySelector(`[data-point="${zero.id}"]`)!;
    const errors: unknown[] = [];
    const onError = (e: ErrorEvent) => {
      errors.push(e.error);
      e.preventDefault();
    };
    window.addEventListener("error", onError);
    const sent = vi.spyOn(mock!, "send");
    try {
      fireEvent.pointerDown(circle, { button: 0, clientX: 0, clientY: 20 });
      await act(async () => {
        window.dispatchEvent(new MouseEvent("pointermove", { clientX: 0, clientY: 40 }) as PointerEvent);
      });
      await flush();
      expect(sent).toHaveBeenCalled();
      r.unmount();
      await flush();
      const before = sent.mock.calls.length;
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 0, clientY: 60 }) as PointerEvent);
      window.dispatchEvent(new MouseEvent("pointerup", { clientX: 0, clientY: 60 }) as PointerEvent);
      await flush();
      expect(errors).toEqual([]);
      expect(sent.mock.calls.slice(before)).toEqual([]);
      expect(sent.mock.calls.filter(([c]) => c.domain === "Edit" && c.command.type === "EndGesture")).toHaveLength(1);
    } finally {
      window.removeEventListener("error", onError);
    }
  });

  it("Shift-drag changes BPM in fine 0.01 steps, like the transport bar's tempo drag", async () => {
    await setup(<Lane />);
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

  it("Alt-drag is a precision drag: the BPM moves a tenth as far, in hundredths (#793)", async () => {
    await setup(<Lane />);
    const zero = tempoPoints()[0]!;
    const circle = document.querySelector(`[data-point="${zero.id}"]`)!;
    const bpm = () => project().tempo_points[zero.id]!.bpm;
    const drag = async (dy: number, altKey: boolean) => {
      fireEvent.pointerDown(circle, { button: 0, clientX: 0, clientY: 20, altKey });
      await act(async () => {
        window.dispatchEvent(new MouseEvent("pointermove", { clientX: 0, clientY: 20 + dy, altKey }) as PointerEvent);
      });
      await flush();
      await act(async () => {
        window.dispatchEvent(new MouseEvent("pointerup", { clientX: 0, clientY: 20 + dy, altKey }) as PointerEvent);
      });
      await flush();
    };
    await drag(30, true);
    const fine = bpm() - zero.bpm;
    expect(fine).toBeLessThan(0);
    expect(Math.round(fine * 100)).toBeCloseTo(fine * 100, 6);
    const before = bpm();
    await drag(30, false);
    const full = bpm() - before;
    // The plain drag rounds to whole BPM: a tenth of it, give or take that rounding.
    expect(Math.abs(fine * 10 - full)).toBeLessThanOrEqual(0.6);
  });

  describe("edits like the automation lanes and the arranger (#621)", () => {
    afterEach(() => {
      itemSelection.getState().clear();
      resetTimeSelection();
    });

    /** Points at beats 4 (130), 8 (140) and 16 (100), besides the one at 0. */
    async function withPoints() {
      await setup(<Lane />);
      for (const [id, time, bpm] of [
        ["01TEMPOA00000000000000000A", 4, 130],
        ["01TEMPOB00000000000000000B", 8, 140],
        ["01TEMPOC00000000000000000C", 16, 100],
      ] as const)
        await act(async () => void (await mock!.send(cmd("Tempo", { type: "AddTempoPoint", id, time, bpm, curve: "Step" }))));
      return { a: "01TEMPOA00000000000000000A", b: "01TEMPOB00000000000000000B", c: "01TEMPOC00000000000000000C" };
    }
    const drag = async (from: [number, number], to: [number, number], init: { shiftKey?: boolean } = {}) => {
      fireEvent.pointerDown(screen.getByTestId("tempo-lane-svg"), { button: 0, clientX: from[0], clientY: from[1], ...init });
      await act(async () => {
        window.dispatchEvent(new MouseEvent("pointermove", { clientX: to[0], clientY: to[1], ...init }) as PointerEvent);
        window.dispatchEvent(new MouseEvent("pointerup", { clientX: to[0], clientY: to[1], ...init }) as PointerEvent);
      });
      await flush();
    };
    const selectedIds = () => [...itemSelection.getState().selected.tempoPoint].sort();
    const key = (k: string, init: { ctrlKey?: boolean } = {}) => fireEvent.keyDown(screen.getByTestId("tempo-lane"), { key: k, ...init });

    it("a drag on empty space selects the points in its box; Delete removes them in one undo step", async () => {
      const { a, b } = await withPoints();
      // Beats 3..9 (x 36..108), the whole height.
      await drag([36, 0], [108, 120]);
      expect(selectedIds()).toEqual([a, b].sort());
      expect(useTimeSelection.getState().selection).toBeNull();
      key("Delete");
      await waitFor(() => expect(tempoPoints().map((p) => p.time)).toEqual([0, 16]));
      await act(async () => void (await mock!.send(cmd("Edit", { type: "Undo" }))));
      expect(tempoPoints().map((p) => p.time)).toEqual([0, 4, 8, 16]);
    });

    it("Shift+drag selects a time section of the lane (no points); Delete clears it", async () => {
      await withPoints();
      await drag([36, 60], [108, 60], { shiftKey: true });
      const sel = useTimeSelection.getState().selection!;
      expect(sel).toMatchObject({ automationLane: "tempo" });
      expect(sel.start).toBeLessThanOrEqual(4);
      expect(sel.end).toBeGreaterThan(8);
      expect(sel.end).toBeLessThan(16);
      expect(selectedIds()).toEqual([]);
      expect(screen.getByTestId("automation-range")).toBeTruthy();
      key("Delete");
      await waitFor(() => expect(tempoPoints().map((p) => p.time)).toEqual([0, 16]));
    });

    it("copy / paste at the selection cursor and duplicate, with the arranger's chords", async () => {
      const { b } = await withPoints();
      fireEvent.pointerDown(document.querySelector(`[data-point="${b}"]`)!, { button: 0, clientX: 96, clientY: 60 });
      await act(async () => void window.dispatchEvent(new MouseEvent("pointerup", { clientX: 96, clientY: 60 }) as PointerEvent));
      expect(selectedIds()).toEqual([b]);
      key("c", { ctrlKey: true });
      act(() => useTimeSelection.setState({ cursor: 24 }));
      key("v", { ctrlKey: true });
      await waitFor(() => expect(tempoPoints().find((p) => p.time === 24)?.bpm).toBe(140));
      // The pasted point is selected; ⌘D puts a copy right after it.
      key("d", { ctrlKey: true });
      await waitFor(() => expect(tempoPoints().filter((p) => p.bpm === 140)).toHaveLength(3));
    });

    it("Escape cancels a point drag: the points go back and no undo step is left (#717)", async () => {
      const { b } = await withPoints();
      const before = useProjectStore.getState().history;
      fireEvent.pointerDown(document.querySelector(`[data-point="${b}"]`)!, { button: 0, clientX: 96, clientY: 60 });
      await act(async () => void window.dispatchEvent(new MouseEvent("pointermove", { clientX: 132, clientY: 60 }) as PointerEvent));
      await flush();
      expect(project().tempo_points[b]!.time).toBeGreaterThan(8);
      await act(async () => void window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })));
      await act(async () => void window.dispatchEvent(new MouseEvent("pointerup", { clientX: 132, clientY: 60 }) as PointerEvent));
      await flush();
      expect(project().tempo_points[b]!.time).toBe(8);
      // The history is as before the drag: undo undoes the step before it, nothing to redo.
      const after = useProjectStore.getState().history;
      expect(after.undo_id).toBe(before.undo_id);
      expect(after.undo_label).toBe(before.undo_label);
      expect(after.can_redo).toBe(false);
    });

    it("its menus have the automation lanes' entries that apply", async () => {
      const { b } = await withPoints();
      fireEvent.contextMenu(document.querySelector(`[data-point="${b}"]`)!, { clientX: 1, clientY: 1 });
      const labels = useContextMenuStore.getState().menu!.items.map((i) => (i === "separator" ? "-" : i.label));
      expect(labels).toEqual(["cut", "copy", "paste", "duplicate", "-", "ramp to next point", "-", "delete tempo point"]);
      fireEvent.contextMenu(screen.getByTestId("tempo-lane-svg"), { clientX: 200, clientY: 1 });
      const lane = useContextMenuStore.getState().menu!.items.map((i) => (i === "separator" ? "-" : i.label));
      expect(lane).toEqual(["paste here", "paste at cursor", "-", "select all points", "copy", "delete"]);
    });
  });

  it("the point at beat 0 only changes BPM and can't be deleted", async () => {
    await setup(<Lane />);
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
    fireEvent.keyDown(screen.getByTestId("tempo-lane"), { key: "Delete" });
    fireEvent.doubleClick(document.querySelector(`[data-point="${zero.id}"]`)!);
    await flush();
    expect(tempoPoints()).toHaveLength(1);
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
    act(() => items.find((i) => i.label === "add tempo change here")!.onSelect());
    await waitFor(() => expect(tempoPoints()).toHaveLength(2));
    expect(tempoPoints()[1]!.time).toBe(16);

    fireEvent.contextMenu(ruler, { clientX: 130, clientY: 5 });
    items = useContextMenuStore.getState().menu!.items.filter((i) => i !== "separator") as ContextMenuItem[];
    act(() => items.find((i) => i.label === "add time signature change here")!.onSelect());
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

describe("MetronomePanel", () => {
  it("toggles the metronome and edits volume, accent and sound", async () => {
    await setup(<MetronomePanel />);
    const on = await screen.findByRole("switch", { name: "metronome" });
    fireEvent.click(on);
    await waitFor(() => expect(project().settings.metronome).toBe(true));

    const volume = screen.getByRole("spinbutton", { name: "metronome volume" });
    fireEvent.change(volume, { target: { value: "-12" } });
    fireEvent.keyDown(volume, { key: "Enter" });
    await waitFor(() => expect(project().settings.metronome_volume).toBe(-12));

    fireEvent.click(screen.getByRole("switch", { name: "accent downbeat" }));
    await waitFor(() => expect(project().settings.metronome_accent).toBe(false));

    pickOption(screen.getByRole("combobox", { name: "metronome sound" }), { value: "Beep" });
    await waitFor(() => expect(project().settings.metronome_sound).toBe("Beep"));

    // Each change is its own undo step.
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    expect(project().settings.metronome_sound).toBe("Classic");
    expect(project().settings.metronome_accent).toBe(false);
  });
});

describe("signatureMenu", () => {
  const del = (time: number) =>
    signatureMenu({ id: "s", time, signature: { numerator: 4, denominator: 4 } }, vi.fn(), vi.fn()).find(
      (e): e is ContextMenuItem => e !== "separator" && e.label === "delete time signature",
    )!;

  it("says why the bar-1 signature can't be deleted", () => {
    expect(del(0)).toMatchObject({ disabled: true, reason: expect.stringMatching(/bar 1/) });
    expect(del(4).disabled).toBe(false);
  });
});
