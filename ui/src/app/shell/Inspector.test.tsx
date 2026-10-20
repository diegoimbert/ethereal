import { act, fireEvent, renderHook, screen, waitFor, within } from "@testing-library/react";
import { create } from "zustand";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Clip } from "@/generated";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/devices/testUtils";
import { useArrangementUi } from "@/features/arrangement/state";
import { resetCollapsed } from "@/features/devices/collapsed";
import { useEditorStore, useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd } from "@/transport";
import { type MockTransport } from "@/transport/mock";
import { pickOption } from "@/kit/testing";
import { Inspector } from "./Inspector";
import { useInspectorTarget, type InspectorTarget } from "./inspectorTarget";
import { resetShell } from "@/state/shell";

let mock: MockTransport | undefined;
beforeAll(stubPointerCapture);
afterEach(() => {
  resetStores(mock);
  mock = undefined;
  resetShell();
  resetCollapsed();
  itemSelection.getState().clear();
  useArrangementUi.getState().setTrackFocus(null);
  useEditorStore.getState().close();
});

const clipByName = (name: string): Clip => Object.values(store().project!.clips).find((c) => c.name === name)!;
const firstAudioClip = (): Clip => Object.values(store().project!.clips).find((c) => c.content.type === "Audio")!;

/** The inspector for a target resolved once the project has loaded. */
function Subject({ pick }: { pick: () => InspectorTarget | null }) {
  const loaded = useProjectStore((s) => s.project !== null);
  return loaded ? <Inspector target={pick()} /> : null;
}

describe("Inspector: clip", () => {
  it("edits the name, mute and color of a MIDI clip; shows its track's devices", async () => {
    mock = await renderWithMock(<Subject pick={() => ({ kind: "clips", ids: [clipByName("Chords").id] })} />);
    const chords = clipByName("Chords");

    const name = screen.getByRole("textbox", { name: "clip name" });
    expect(name).toHaveValue("Chords");
    fireEvent.change(name, { target: { value: "Pads" } });
    fireEvent.keyDown(name, { key: "Enter" });
    fireEvent.blur(name);
    await flush();
    expect(store().project!.clips[chords.id]!.name).toBe("Pads");

    fireEvent.click(screen.getByRole("switch", { name: "mute clip" }));
    await flush();
    expect(store().project!.clips[chords.id]!.muted).toBe(true);

    // The color is one swatch opening the palette in a popover (closed on pick).
    expect(screen.queryByRole("radiogroup", { name: "clip color" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "clip color" }));
    fireEvent.click(within(await screen.findByRole("radiogroup", { name: "clip color" })).getAllByRole("radio")[3]!);
    await flush();
    expect(store().project!.clips[chords.id]!.color).not.toBeNull();
    await waitFor(() => expect(screen.queryByRole("radiogroup", { name: "clip color" })).toBeNull());
    fireEvent.click(screen.getByRole("button", { name: "clip color" }));
    fireEvent.click(within(await screen.findByRole("radiogroup", { name: "clip color" })).getByRole("radio", { name: "track color" }));
    await flush();
    expect(store().project!.clips[chords.id]!.color).toBeNull();

    // MIDI clips have no audio section.
    expect(screen.queryByRole("spinbutton", { name: "clip gain" })).toBeNull();
    expect(screen.queryByRole("textbox", { name: "clip gain" })).toBeNull();

    // No editor button: the clip opens its editor from the arrangement.
    expect(screen.queryByRole("button", { name: /Edit notes/ })).toBeNull();

    // The clip's track and its devices, stacked.
    expect(screen.getByRole("list", { name: "Keys devices" })).toHaveClass("eth-devices__chain--stack");
  });

  it("shows audio controls for an audio clip (gain, reverse, warp)", async () => {
    mock = await renderWithMock(<Subject pick={() => ({ kind: "clips", ids: [firstAudioClip().id] })} />);
    const clip = firstAudioClip();
    const gain = screen.getByLabelText("clip gain");
    fireEvent.change(gain, { target: { value: "-6" } });
    fireEvent.keyDown(gain, { key: "Enter" });
    await flush();
    const after = store().project!.clips[clip.id]!;
    expect(after.content.type === "Audio" && after.content.gain).toBe(-6);

    fireEvent.click(screen.getByRole("switch", { name: "reverse clip" }));
    await flush();
    const rev = store().project!.clips[clip.id]!;
    expect(rev.content.type === "Audio" && rev.content.reversed).toBe(true);
    expect(screen.getByRole("combobox", { name: "warp mode" })).toBeInTheDocument();

    // Dragging the gain field up: 1 dB (two 0.5 steps of 4 px), one undo step.
    const field = screen.getByLabelText("clip gain");
    fireEvent.pointerDown(field, { button: 0, pointerId: 1, clientY: 100 });
    fireEvent.pointerMove(field, { pointerId: 1, clientY: 96 });
    fireEvent.pointerMove(field, { pointerId: 1, clientY: 92 });
    fireEvent.pointerUp(field, { pointerId: 1, clientY: 92 });
    await flush();
    const dragged = store().project!.clips[clip.id]!;
    expect(dragged.content.type === "Audio" && dragged.content.gain).toBe(-5);
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    const undone = store().project!.clips[clip.id]!;
    expect(undone.content.type === "Audio" && undone.content.gain).toBe(-6);
  });

  it("transposing a Repitch clip resizes it; trimmed at the next clip; one undo step", async () => {
    mock = await renderWithMock(<Subject pick={() => ({ kind: "clips", ids: [firstAudioClip().id] })} />);
    const clip = firstAudioClip();
    expect(clip.length).toBe(16);
    const setTranspose = async (v: string) => {
      const field = screen.getByLabelText("clip transpose");
      fireEvent.change(field, { target: { value: v } });
      fireEvent.keyDown(field, { key: "Enter" });
      await flush();
    };
    const now = () => store().project!.clips[clip.id]!;
    const transpose = () => {
      const c = now().content;
      return c.type === "Audio" ? c.transpose : NaN;
    };

    // An octave up plays twice as fast: half as long.
    await setTranspose("12");
    expect(transpose()).toBe(12);
    expect(now().length).toBeCloseTo(8, 6);
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
    expect(transpose()).toBe(0);
    expect(now().length).toBe(16);

    // An octave down would be 32 beats: trimmed at a clip starting at beat 20.
    await act(async () => {
      await mock!.send(cmd("Clip", { type: "Duplicate", id: clip.id, new_id: "next-clip", start: 20 }));
    });
    await setTranspose("-12");
    expect(transpose()).toBe(-12);
    expect(now().start + now().length).toBeCloseTo(20, 6);
    expect(store().project!.clips["next-clip"]).toMatchObject({ start: 20, length: 16 });

    // Pitch-preserving (Complex): only the pitch changes.
    await act(async () => {
      await mock!.send(cmd("Warp", { type: "SetWarp", clip: clip.id, warp: { enabled: true, mode: "Complex", source_bpm: 120 } }));
    });
    const len = now().length;
    await setTranspose("5");
    expect(transpose()).toBe(5);
    expect(now().length).toBe(len);
  });

  it("several clips: a count and shared actions", async () => {
    mock = await renderWithMock(<Subject pick={() => ({ kind: "clips", ids: [clipByName("Chords").id, clipByName("Bassline").id] })} />);
    expect(screen.getByText("2 clips")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("switch", { name: "mute clips" }));
    await flush();
    expect(clipByName("Chords").muted && clipByName("Bassline").muted).toBe(true);
  });
});

describe("Inspector: several audio clips (base-150)", () => {
  const audioOf = (id: string) => {
    const c = store().project!.clips[id]!.content;
    if (c.type !== "Audio") throw new Error("not audio");
    return c;
  };
  const undo = async () => {
    await act(async () => {
      await mock!.send(cmd("Edit", { type: "Undo" }));
    });
  };
  /** Two audio clips far apart (no trimming between them), the second at +2 st. */
  async function twoClips() {
    const target = create<{ t: InspectorTarget | null }>()(() => ({ t: null }));
    function Selected() {
      return <Inspector target={target((s) => s.t)} />;
    }
    mock = await renderWithMock(<Selected />);
    const a = firstAudioClip().id;
    await act(async () => {
      await mock!.send(cmd("Clip", { type: "Duplicate", id: a, new_id: "clip-b", start: 200 }));
      await mock!.send(cmd("Clip", { type: "SetTranspose", id: "clip-b", semitones: 2 }));
    });
    act(() => target.setState({ t: { kind: "clips", ids: [a, "clip-b"] } }));
    await flush();
    return { a, b: "clip-b" };
  }

  it("shows mixed values; a typed value sets all; an arrow step and a drag move each by the same amount; one undo each", async () => {
    const { a, b } = await twoClips();
    const field = screen.getByLabelText("clip transpose");
    expect(field).toHaveValue("—");
    expect(field).toHaveAttribute("aria-valuetext", "mixed");

    // Typed: absolute on all, one undo step reverts both.
    fireEvent.change(field, { target: { value: "5" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await flush();
    expect([audioOf(a).transpose, audioOf(b).transpose]).toEqual([5, 5]);
    expect(screen.getByLabelText("clip transpose")).toHaveValue("5");
    await undo();
    expect([audioOf(a).transpose, audioOf(b).transpose]).toEqual([0, 2]);

    // Arrow step: each moves by +1 (differences kept), one undo step.
    fireEvent.keyDown(screen.getByLabelText("clip transpose"), { key: "ArrowUp" });
    await flush();
    expect([audioOf(a).transpose, audioOf(b).transpose]).toEqual([1, 3]);
    await undo();
    expect([audioOf(a).transpose, audioOf(b).transpose]).toEqual([0, 2]);

    // Drag up two steps (4 px each): +2 on each, one gesture = one undo step.
    const f = screen.getByLabelText("clip transpose");
    fireEvent.pointerDown(f, { button: 0, pointerId: 1, clientY: 100 });
    fireEvent.pointerMove(f, { pointerId: 1, clientY: 96 });
    fireEvent.pointerMove(f, { pointerId: 1, clientY: 92 });
    fireEvent.pointerUp(f, { pointerId: 1, clientY: 92 });
    await flush();
    expect([audioOf(a).transpose, audioOf(b).transpose]).toEqual([2, 4]);
    await undo();
    expect([audioOf(a).transpose, audioOf(b).transpose]).toEqual([0, 2]);
  });

  it("transposing several Repitch clips resizes each of them", async () => {
    const { a, b } = await twoClips();
    const lenA = store().project!.clips[a]!.length;
    const lenB = store().project!.clips[b]!.length;
    const field = screen.getByLabelText("clip transpose");
    fireEvent.change(field, { target: { value: "12" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await flush();
    expect(store().project!.clips[a]!.length).toBeCloseTo(lenA / 2, 6);
    // b went from +2 to +12.
    expect(store().project!.clips[b]!.length).toBeCloseTo(lenB / 2 ** (10 / 12), 6);
  });

  it("gain, reverse, warp and warp mode apply to every selected audio clip", async () => {
    const { a, b } = await twoClips();
    const gain = screen.getByLabelText("clip gain");
    fireEvent.change(gain, { target: { value: "-3" } });
    fireEvent.keyDown(gain, { key: "Enter" });
    await flush();
    expect([audioOf(a).gain, audioOf(b).gain]).toEqual([-3, -3]);
    fireEvent.keyDown(screen.getByLabelText("clip gain"), { key: "ArrowUp" });
    await flush();
    expect([audioOf(a).gain, audioOf(b).gain]).toEqual([-2.5, -2.5]);

    fireEvent.click(screen.getByRole("switch", { name: "reverse clips" }));
    await flush();
    expect([audioOf(a).reversed, audioOf(b).reversed]).toEqual([true, true]);

    // Mixed warp: the switch shows off and turns warp on for both.
    await act(async () => {
      await mock!.send(cmd("Warp", { type: "SetWarp", clip: a, warp: { ...audioOf(a).warp, enabled: true } }));
      await mock!.send(cmd("Warp", { type: "SetWarp", clip: b, warp: { ...audioOf(b).warp, enabled: false } }));
    });
    expect(screen.getByRole("switch", { name: "warp clips" })).toHaveAttribute("aria-checked", "false");
    fireEvent.click(screen.getByRole("switch", { name: "warp clips" }));
    await flush();
    expect([audioOf(a).warp.enabled, audioOf(b).warp.enabled]).toEqual([true, true]);
    pickOption(screen.getByRole("combobox", { name: "warp mode" }), "complex");
    await flush();
    expect([audioOf(a).warp.mode, audioOf(b).warp.mode]).toEqual(["Complex", "Complex"]);
  });

  it("a mixed audio + MIDI selection edits only the audio clips", async () => {
    mock = await renderWithMock(<Subject pick={() => ({ kind: "clips", ids: [firstAudioClip().id, clipByName("Chords").id] })} />);
    expect(screen.getByText("audio (1 of 2 clips)")).toBeInTheDocument();
    const field = screen.getByLabelText("clip transpose");
    fireEvent.change(field, { target: { value: "3" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await flush();
    expect(audioOf(firstAudioClip().id).transpose).toBe(3);
  });
});

describe("Inspector: track", () => {
  it("renames, mixes and shows the track's devices as stacked cards", async () => {
    mock = await renderWithMock(<Subject pick={() => ({ kind: "track", id: trackByName("Keys").id })} />);
    const keys = trackByName("Keys");
    const name = screen.getByRole("textbox", { name: "track name" });
    fireEvent.change(name, { target: { value: "Piano" } });
    fireEvent.blur(name);
    await flush();
    expect(store().project!.tracks[keys.id]!.name).toBe("Piano");

    expect(screen.getByRole("slider", { name: "Piano volume" })).toBeInTheDocument();
    expect(screen.getByRole("slider", { name: "Piano pan" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "mute Piano" }));
    await flush();
    expect(store().project!.tracks[keys.id]!.mixer.mute).toBe(true);
    expect(screen.getByRole("combobox", { name: "Piano output" })).toBeInTheDocument();

    const devices = await screen.findByRole("list", { name: "Piano devices" });
    expect(devices).toHaveClass("eth-devices__chain--stack");
    expect(within(devices).getAllByRole("region").length).toBeGreaterThan(0);
  });

  it("a send's pre/post toggle: disabled until the send exists, then flips its tap point (one undo step)", async () => {
    mock = await renderWithMock(<Subject pick={() => ({ kind: "track", id: trackByName("Bass").id })} />);
    const bass = trackByName("Bass");
    const ret = Object.values(store().project!.tracks).find((t) => t.kind === "Return")!;
    const sendOf = () => Object.values(store().project!.sends).find((s) => s.from === bass.id && s.to === ret.id);
    expect(sendOf()).toBeUndefined();
    const tap = screen.getByRole("button", { name: `send ${ret.name} pre-fader` });
    expect(tap).toBeDisabled();
    expect(tap).toHaveTextContent("post");

    await act(async () => void (await mock!.send(cmd("Mixer", { type: "CreateSend", id: "s-bass", from: bass.id, to: ret.id, level: -6, pre_fader: false }))));
    await flush();
    expect(tap).toBeEnabled();
    fireEvent.click(tap);
    await flush();
    expect(sendOf()!.pre_fader).toBe(true);
    expect(tap).toHaveAttribute("aria-pressed", "true");
    expect(tap).toHaveTextContent("pre");

    await act(async () => void (await mock!.send(cmd("Edit", { type: "Undo" }))));
    await flush();
    expect(sendOf()!.pre_fader).toBe(false);
    expect(sendOf()!.level).toBe(-6);
  });

  it("the send knobs follow the returns' order (#791)", async () => {
    mock = await renderWithMock(<Subject pick={() => ({ kind: "track", id: trackByName("Bass").id })} />);
    const first = Object.values(store().project!.tracks).find((t) => t.kind === "Return")!;
    await act(
      async () =>
        void (await mock!.send(cmd("Track", { type: "Create", id: "ret-b", kind: "Return", name: "B Rev", color: null, parent: null, before: null }))),
    );
    await flush();
    const knobs = () =>
      within(screen.getByLabelText("sends"))
        .getAllByRole("slider")
        .map((s) => s.getAttribute("aria-label"));
    expect(knobs()).toEqual([`send ${first.name}`, "send B Rev"]);
    await act(async () => void (await mock!.send(cmd("Track", { type: "Move", id: "ret-b", parent: null, before: first.id }))));
    await flush();
    expect(knobs()).toEqual(["send B Rev", `send ${first.name}`]);
  });

  it("keeps showing the last subject while it closes (target becomes null)", async () => {
    const current = create<{ target: InspectorTarget | null }>()(() => ({ target: null }));
    const setTarget = (target: InspectorTarget | null) => current.setState({ target });
    function Controlled() {
      return <Inspector target={current((s) => s.target)} />;
    }
    mock = await renderWithMock(<Controlled />);
    expect(screen.queryByTestId("inspector")).toBeNull();
    act(() => setTarget({ kind: "track", id: trackByName("Keys").id }));
    expect(screen.getByRole("textbox", { name: "track name" })).toHaveValue("Keys");
    act(() => setTarget(null));
    expect(screen.getByRole("textbox", { name: "track name" })).toHaveValue("Keys");
  });
});

describe("useInspectorTarget", () => {
  it("selected clips, else the focused track, else nothing", () => {
    const { result } = renderHook(() => useInspectorTarget());
    expect(result.current).toBeNull();
    act(() => useArrangementUi.getState().setTrackFocus("t1"));
    expect(result.current).toEqual({ kind: "track", id: "t1" });
    act(() => itemSelection.getState().select("clip", ["c1"], "replace"));
    expect(result.current).toEqual({ kind: "clips", ids: ["c1"] });
  });
});
