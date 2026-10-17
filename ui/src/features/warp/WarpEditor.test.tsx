import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Clip, Command, GestureId, WarpMarker } from "@/generated";
import { useEditorStore, useProjectStore, warpMarkersOfClip } from "@/state";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { itemSelection } from "@/timeline";
import { WarpEditor } from "./index";
import { pickOption } from "@/kit/testing";

let mock: MockTransport | undefined;
const store = () => useProjectStore.getState();

// jsdom has no canvas: the waveform draws nothing (its geometry is covered by warpMap tests).
beforeEach(() => {
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
});

afterEach(() => {
  vi.restoreAllMocks();
  mock?.dispose();
  mock = undefined;
  store().reset();
  itemSelection.getState().clear();
  useEditorStore.getState().close();
});

async function flush() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

interface Sent {
  command: Command;
  gesture: GestureId | null;
}

/** Mock transport (optionally pretending to be the web build), with every sent command recorded. */
async function setup(opts: { kind?: "wasm"; open?: "audio" | "midi" | null } = {}) {
  mock = new MockTransport({ timers: "manual", seed: 3 });
  if (opts.kind) Object.defineProperty(mock, "kind", { value: opts.kind });
  const sent: Sent[] = [];
  const send = mock.send.bind(mock);
  vi.spyOn(mock, "send").mockImplementation((command, o) => {
    sent.push({ command, gesture: o?.gesture ?? null });
    return send(command, o);
  });
  render(
    <TransportProvider transport={mock}>
      <WarpEditor />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  const clips = Object.values(store().project!.clips);
  const audio = clips.find((c) => c.content.type === "Audio")!;
  const midi = clips.find((c) => c.content.type === "Midi")!;
  const open = opts.open === undefined ? "audio" : opts.open;
  if (open) act(() => useEditorStore.getState().openClip(open === "audio" ? audio.id : midi.id));
  await flush();
  return { clip: audio, sent };
}

const clipNow = (id: string): Clip => store().project!.clips[id]!;
const markers = (clip: string): WarpMarker[] => warpMarkersOfClip(store().project!, clip);
const view = () => screen.getByTestId("warp-view");
const pxPerBeat = () => Number(view().dataset.pxPerBeat);
const markerEl = (id: string) => document.querySelector(`[data-testid="warp-marker"][data-marker-id="${id}"]`)!;

async function addMarker(beat: number) {
  fireEvent.doubleClick(view(), { clientX: beat * pxPerBeat() });
  await flush();
}

describe("WarpEditor", () => {
  it("asks for an audio clip", async () => {
    await setup({ open: null });
    expect(screen.getByTestId("warp-empty").textContent).toMatch(/double-click an audio clip/);
    act(() => useEditorStore.getState().openClip(Object.values(store().project!.clips).find((c) => c.content.type === "Midi")!.id));
    expect(screen.getByTestId("warp-empty").textContent).toMatch(/midi clip/);
  });

  it("shows the clip's warp settings and a waveform", async () => {
    const { clip } = await setup();
    expect(screen.getByTestId("warp-editor").dataset.clipId).toBe(clip.id);
    expect((screen.getByLabelText("warp") as HTMLInputElement).checked).toBe(true);
    expect(screen.getByRole("combobox", { name: "warp mode" })).toHaveTextContent("repitch");
    expect(screen.getByTestId("warp-source-bpm").textContent).toBe("source 120.00 bpm");
    expect(screen.getByTestId("warp-waveform")).toBeTruthy();
    expect(screen.queryByTestId("warp-web-fallback")).toBeNull();
  });

  it("toggles warp, switches the mode and transposes", async () => {
    const { clip, sent } = await setup();
    pickOption(screen.getByRole("combobox", { name: "warp mode" }), { value: "Complex" });
    await flush();
    const content = () => clipNow(clip.id).content as Extract<Clip["content"], { type: "Audio" }>;
    expect(content().warp.mode).toBe("Complex");
    fireEvent.click(screen.getByLabelText("warp"));
    await flush();
    expect(content().warp.enabled).toBe(false);
    expect(screen.getByRole("combobox", { name: "warp mode" })).toBeDisabled();
    expect(screen.getByTestId("warp-marker-count").textContent).toBe("unwarped");
    fireEvent.change(screen.getByRole("spinbutton", { name: "transpose" }), { target: { value: "7" } });
    await flush();
    expect(content().transpose).toBe(7);
    expect(sent.map((s) => s.command)).toContainEqual(cmd("Clip", { type: "SetTranspose", id: clip.id, semitones: 7 }));
  });

  it("adds markers on double-click at the snapped beat, pinned to the current source time", async () => {
    const { clip } = await setup();
    await addMarker(4.1); // snaps to 4
    // 120 BPM source at 120 BPM: beat b plays source b / 2 s. The clip had no markers (its
    // source tempo mapped it): the first one also pins the media start and end (8 s).
    expect(markers(clip.id).map((m) => [m.beat, m.source])).toEqual([
      [0, 0],
      [4, 2],
      [16, 8],
    ]);
    await addMarker(8);
    expect(markers(clip.id).map((m) => [m.beat, m.source])).toEqual([
      [0, 0],
      [4, 2],
      [8, 4],
      [16, 8],
    ]);
    expect(screen.getByTestId("warp-marker-count").textContent).toBe("4 markers");
    // Not while unwarped.
    fireEvent.click(screen.getByLabelText("warp"));
    await flush();
    await addMarker(12);
    expect(markers(clip.id)).toHaveLength(4);
    expect(document.querySelectorAll('[data-testid="warp-marker"]')).toHaveLength(0);
  });

  it("moves a marker in one undo gesture, between its neighbours", async () => {
    const { clip, sent } = await setup();
    await addMarker(4);
    await addMarker(8);
    const at = (beat: number) => markers(clip.id).find((m) => m.beat === beat)!;
    const [a, b] = [at(4), at(8)];
    const px = pxPerBeat();
    sent.length = 0;
    fireEvent.pointerDown(markerEl(b!.id), { button: 0, clientX: 8 * px });
    for (const beat of [7.5, 6, 5]) fireEvent.pointerMove(window, { clientX: beat * px });
    fireEvent.pointerMove(window, { clientX: 1 * px }); // clamped just right of marker a
    fireEvent.pointerUp(window);
    await flush();
    const moved = markers(clip.id).find((m) => m.id === b!.id)!;
    expect(moved.beat).toBeCloseTo(4 + 1 / 64);
    expect(moved.source).toBe(4); // the audio stretches: the source pin stays
    expect(markers(clip.id).find((m) => m.id === a!.id)!.beat).toBe(4);
    const moves = sent.filter((s) => s.command.domain === "Warp");
    expect(moves.length).toBeGreaterThan(1);
    const g = moves[0]!.gesture;
    expect(g).not.toBeNull();
    expect(moves.every((s) => s.gesture === g)).toBe(true);
    expect(sent.at(-1)!.command).toEqual(cmd("Edit", { type: "EndGesture", gesture: g! }));
    // One undo restores the original position.
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    expect(markers(clip.id).find((m) => m.id === b!.id)!.beat).toBe(8);
  });

  it("dragging a marker keeps the view and everything outside its neighbours still", async () => {
    const { clip } = await setup();
    await addMarker(4);
    await addMarker(8);
    const at = (beat: number) => markers(clip.id).find((m) => m.beat === beat)!;
    const ids = [0, 4, 8, 16].map((beat) => at(beat).id);
    const xOf = (id: string) => markerEl(id).getAttribute("transform");
    const px = pxPerBeat();
    const x0 = ids.map(xOf);
    // The middle marker at 4: only it moves on screen; the scale holds.
    fireEvent.pointerDown(markerEl(ids[1]!), { button: 0, clientX: 4 * px });
    for (const beat of [5, 6, 7]) {
      fireEvent.pointerMove(window, { clientX: beat * px });
      await flush();
      expect(pxPerBeat()).toBe(px);
      expect([ids[0], ids[2], ids[3]].map((id) => xOf(id!))).toEqual([x0[0], x0[2], x0[3]]);
    }
    fireEvent.pointerUp(window);
    await flush();
    expect(markers(clip.id).map((m) => [m.beat, m.source])).toEqual([
      [0, 0],
      [7, 2],
      [8, 4],
      [16, 8],
    ]);
    // The last marker moves the media end: the view still holds during the drag, and
    // afterwards zooms out only if the audio no longer fits.
    fireEvent.pointerDown(markerEl(ids[3]!), { button: 0, clientX: 16 * px });
    for (const beat of [18, 20, 22]) {
      fireEvent.pointerMove(window, { clientX: beat * px });
      await flush();
      expect(pxPerBeat()).toBe(px);
      expect(xOf(ids[2]!)).toBe(x0[2]);
    }
    fireEvent.pointerUp(window);
    await flush();
    expect(at(22).source).toBe(8);
    expect(pxPerBeat()).toBeLessThan(px);
  });

  it("deletes markers by double-click or Delete", async () => {
    const { clip } = await setup();
    await addMarker(2);
    await addMarker(6);
    const [a, b] = [2, 6].map((beat) => markers(clip.id).find((m) => m.beat === beat)!);
    fireEvent.doubleClick(markerEl(a!.id));
    await flush();
    expect(markers(clip.id).map((m) => m.beat)).toEqual([0, 6, 16]);
    fireEvent.pointerDown(markerEl(b!.id), { button: 0, clientX: 0 });
    fireEvent.pointerUp(window);
    fireEvent.keyDown(view(), { key: "Delete" });
    await flush();
    expect(markers(clip.id).map((m) => m.beat)).toEqual([0, 16]);
  });

  it("keyboard changes on the transpose knob are single undo steps, not open gestures", async () => {
    const { clip, sent } = await setup();
    sent.length = 0;
    const knob = screen.getByRole("slider", { name: "transpose" });
    fireEvent.keyDown(knob, { key: "ArrowRight" });
    await flush();
    fireEvent.keyDown(knob, { key: "End" });
    await flush();
    const transposes = sent.filter((s) => s.command.domain === "Clip");
    expect(transposes.map((s) => s.gesture)).toEqual([null, null]);
    expect(transposes.at(-1)!.command).toEqual(cmd("Clip", { type: "SetTranspose", id: clip.id, semitones: 48 }));
    expect((clipNow(clip.id).content as Extract<Clip["content"], { type: "Audio" }>).transpose).toBe(48);
  });

  it("offers pitch-preserving Complex in the browser build (no fallback note)", async () => {
    await setup({ kind: "wasm" });
    pickOption(screen.getByRole("combobox", { name: "warp mode" }), { value: "Complex" });
    await flush();
    expect(screen.queryByTestId("warp-web-fallback")).toBeNull();
  });

  it("with several audio clips selected, transpose and warp apply to all of them (base-150)", async () => {
    const { clip, sent } = await setup();
    await act(async () => {
      await mock!.send(cmd("Clip", { type: "Duplicate", id: clip.id, new_id: "clip-b", start: 200 }));
      await mock!.send(cmd("Clip", { type: "SetTranspose", id: "clip-b", semitones: 3 }));
    });
    act(() => itemSelection.getState().select("clip", [clip.id, "clip-b"], "replace"));
    await flush();
    const tr = (id: string) => (clipNow(id).content as Extract<Clip["content"], { type: "Audio" }>).transpose;

    // A knob step moves each by the same amount (differences kept), one Batch = one undo step.
    sent.length = 0;
    fireEvent.keyDown(screen.getByRole("slider", { name: "transpose" }), { key: "ArrowRight" });
    await flush();
    expect([tr(clip.id), tr("clip-b")]).toEqual([1, 4]);
    expect(sent.filter((s) => s.command.domain === "Edit").map((s) => s.command.command.type)).toEqual(["Batch"]);
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    expect([tr(clip.id), tr("clip-b")]).toEqual([0, 3]);

    // A typed value sets all.
    fireEvent.change(screen.getByRole("spinbutton", { name: "transpose" }), { target: { value: "-5" } });
    await flush();
    expect([tr(clip.id), tr("clip-b")]).toEqual([-5, -5]);

    // Warp on/off too.
    const warp = (id: string) => (clipNow(id).content as Extract<Clip["content"], { type: "Audio" }>).warp.enabled;
    const before = warp(clip.id);
    fireEvent.click(screen.getByRole("checkbox", { name: "warp" }));
    await flush();
    expect([warp(clip.id), warp("clip-b")]).toEqual([!before, !before]);

    // Not part of the selection: only the edited clip.
    act(() => itemSelection.getState().select("clip", ["clip-b"], "replace"));
    await flush();
    fireEvent.change(screen.getByRole("spinbutton", { name: "transpose" }), { target: { value: "7" } });
    await flush();
    expect([tr(clip.id), tr("clip-b")]).toEqual([7, -5]);
  });
});
