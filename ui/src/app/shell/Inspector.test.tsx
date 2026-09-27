import { act, fireEvent, renderHook, screen, within } from "@testing-library/react";
import { create } from "zustand";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import type { Clip } from "@/generated";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/mixer/testUtils";
import { useArrangementUi } from "@/features/arrangement/state";
import { resetCollapsed } from "@/features/devices/collapsed";
import { useEditorStore, useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, type MockTransport } from "@/transport";
import { Inspector } from "./Inspector";
import { useInspectorTarget, type InspectorTarget } from "./inspectorTarget";
import { resetShell, useShellStore } from "./shellStore";

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
  it("edits the name, mute and color of a MIDI clip; Edit notes opens the piano roll drawer", async () => {
    mock = await renderWithMock(<Subject pick={() => ({ kind: "clips", ids: [clipByName("Chords").id] })} />);
    const chords = clipByName("Chords");

    const name = screen.getByRole("textbox", { name: "Clip name" });
    expect(name).toHaveValue("Chords");
    fireEvent.change(name, { target: { value: "Pads" } });
    fireEvent.keyDown(name, { key: "Enter" });
    fireEvent.blur(name);
    await flush();
    expect(store().project!.clips[chords.id]!.name).toBe("Pads");

    fireEvent.click(screen.getByRole("switch", { name: "Mute clip" }));
    await flush();
    expect(store().project!.clips[chords.id]!.muted).toBe(true);

    const swatches = screen.getByRole("radiogroup", { name: "Clip color" });
    fireEvent.click(within(swatches).getAllByRole("radio")[3]!);
    await flush();
    expect(store().project!.clips[chords.id]!.color).not.toBeNull();
    fireEvent.click(within(swatches).getByRole("radio", { name: "Track color" }));
    await flush();
    expect(store().project!.clips[chords.id]!.color).toBeNull();

    // MIDI clips have no audio section.
    expect(screen.queryByRole("spinbutton", { name: "Clip gain" })).toBeNull();
    expect(screen.queryByRole("textbox", { name: "Clip gain" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /Edit notes/ }));
    expect(useShellStore.getState().bottom).toMatchObject({ open: true, tab: "piano-roll" });
    expect(useEditorStore.getState().clip).toBe(chords.id);

    // The clip's track and its devices, stacked.
    expect(screen.getByRole("list", { name: "Keys devices" })).toHaveClass("eth-devices__chain--stack");
  });

  it("shows audio controls for an audio clip (gain, reverse, warp)", async () => {
    mock = await renderWithMock(<Subject pick={() => ({ kind: "clips", ids: [firstAudioClip().id] })} />);
    const clip = firstAudioClip();
    const gain = screen.getByLabelText("Clip gain");
    fireEvent.change(gain, { target: { value: "-6" } });
    fireEvent.keyDown(gain, { key: "Enter" });
    await flush();
    const after = store().project!.clips[clip.id]!;
    expect(after.content.type === "Audio" && after.content.gain).toBe(-6);

    fireEvent.click(screen.getByRole("switch", { name: "Reverse clip" }));
    await flush();
    const rev = store().project!.clips[clip.id]!;
    expect(rev.content.type === "Audio" && rev.content.reversed).toBe(true);
    expect(screen.getByRole("combobox", { name: "Warp mode" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Edit warp/ })).toBeInTheDocument();

    // Dragging the gain field up: 1 dB (two 0.5 steps of 4 px), one undo step.
    const field = screen.getByLabelText("Clip gain");
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

  it("several clips: a count and shared actions", async () => {
    mock = await renderWithMock(
      <Subject pick={() => ({ kind: "clips", ids: [clipByName("Chords").id, clipByName("Bassline").id] })} />,
    );
    expect(screen.getByText("2 clips")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("switch", { name: "Mute clips" }));
    await flush();
    expect(clipByName("Chords").muted && clipByName("Bassline").muted).toBe(true);
  });
});

describe("Inspector: track", () => {
  it("renames, mixes and shows the track's devices as stacked cards", async () => {
    mock = await renderWithMock(<Subject pick={() => ({ kind: "track", id: trackByName("Keys").id })} />);
    const keys = trackByName("Keys");
    const name = screen.getByRole("textbox", { name: "Track name" });
    fireEvent.change(name, { target: { value: "Piano" } });
    fireEvent.blur(name);
    await flush();
    expect(store().project!.tracks[keys.id]!.name).toBe("Piano");

    expect(screen.getByRole("slider", { name: "Piano volume" })).toBeInTheDocument();
    expect(screen.getByRole("slider", { name: "Piano pan" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Mute Piano" }));
    await flush();
    expect(store().project!.tracks[keys.id]!.mixer.mute).toBe(true);
    expect(screen.getByRole("combobox", { name: "Piano output" })).toBeInTheDocument();

    const devices = await screen.findByRole("list", { name: "Piano devices" });
    expect(devices).toHaveClass("eth-devices__chain--stack");
    expect(within(devices).getAllByRole("region").length).toBeGreaterThan(0);
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
    expect(screen.getByRole("textbox", { name: "Track name" })).toHaveValue("Keys");
    act(() => setTarget(null));
    expect(screen.getByRole("textbox", { name: "Track name" })).toHaveValue("Keys");
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
