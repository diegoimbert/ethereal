import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { createEmptyProject, newId, newProjectId } from "@/transport";
import { tracksOrdered, useProjectStore, useSelectionStore } from "@/state";
import { itemSelection } from "@/timeline/selection";
import { useArrangementUi } from "@/features/arrangement/state";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { CaptureButton } from "./CaptureButton";
import { captureTargetTrack, shouldAdoptTempo } from "./actions";

const store = () => useProjectStore.getState();
const project = () => store().project!;
const button = () => screen.getByRole("button", { name: "Capture MIDI" });

afterEach(() => {
  resetStores();
  useSelectionStore.getState().selectTrack(null);
  useArrangementUi.getState().setTrackFocus(null);
  itemSelection.getState().clear();
});

/** Quarter notes 600 ms apart (100 bpm), advancing the mock clock. */
function playPhrase(mock: { tick(ms: number): void; simulateMidiInput(port: string, data: [number, number, number]): void }, n: number) {
  for (let i = 0; i < n; i++) {
    mock.simulateMidiInput("keys", [0x90, 60 + i, 100]);
    mock.tick(300);
    mock.simulateMidiInput("keys", [0x80, 60 + i, 0]);
    mock.tick(300);
  }
}

describe("CaptureButton", () => {
  it("is disabled without an engine", () => {
    render(<CaptureButton />);
    expect(button()).toBeDisabled();
  });

  it("lights up once something is played and captures into the selected MIDI track", async () => {
    const { mock } = await renderWithMock(<CaptureButton />);
    expect(button()).toBeDisabled();
    const midi = tracksOrdered(project()).filter((t) => t.kind === "Midi");
    const target = midi.at(-1)!;
    useSelectionStore.getState().selectTrack(target.id);
    act(() => playPhrase(mock, 4));
    await waitFor(() => expect(button()).toBeEnabled());
    expect(button()).toHaveAttribute("data-available", "true");
    const clipsBefore = Object.keys(project().clips).length;
    fireEvent.click(button());
    await waitFor(() => expect(Object.keys(project().clips)).toHaveLength(clipsBefore + 1));
    const clip = Object.values(project().clips).find((c) => c.track === target.id && Object.values(project().notes).some((n) => n.clip === c.id && n.pitch === 63))!;
    expect(clip).toBeDefined();
    expect(itemSelection.getState().selected.clip.has(clip.id)).toBe(true);
    // The buffer was used up.
    await waitFor(() => expect(button()).toBeDisabled());
  });

  it("adopts the tempo in an empty project, creating a MIDI track when there is none", async () => {
    const { mock } = await renderWithMock(<CaptureButton />, { project: createEmptyProject(newId, "Empty", newProjectId()) });
    expect(shouldAdoptTempo(project())).toBe(true);
    const before = tracksOrdered(project()).filter((t) => t.kind === "Midi");
    act(() => playPhrase(mock, 8));
    await waitFor(() => expect(button()).toBeEnabled());
    fireEvent.click(button());
    await waitFor(() => expect(Object.keys(project().clips)).toHaveLength(1));
    if (before.length === 0) expect(tracksOrdered(project()).filter((t) => t.kind === "Midi")).toHaveLength(1);
    expect(store().transport?.bpm).toBeCloseTo(100, 1);
    expect(store().transport?.loop_enabled).toBe(true);
  });
});

describe("captureTargetTrack", () => {
  it("prefers the selected MIDI track, then an armed one, then the first", async () => {
    await renderWithMock(<div />);
    const midi = tracksOrdered(project()).filter((t) => t.kind === "Midi");
    const audio = tracksOrdered(project()).find((t) => t.kind === "Audio");
    expect(midi.length).toBeGreaterThan(0);
    expect(captureTargetTrack(project(), [])?.id).toBe(midi[0]!.id);
    if (midi.length > 1) expect(captureTargetTrack(project(), [midi[1]!.id])?.id).toBe(midi[1]!.id);
    if (audio) {
      useSelectionStore.getState().selectTrack(audio.id);
      expect(captureTargetTrack(project(), [])?.kind).toBe("Midi");
    }
    useSelectionStore.getState().selectTrack(midi.at(-1)!.id);
    expect(captureTargetTrack(project(), [])?.id).toBe(midi.at(-1)!.id);
  });
});
