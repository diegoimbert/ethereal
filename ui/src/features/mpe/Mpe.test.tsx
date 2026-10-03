import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it } from "vitest";
import type { Command, Track } from "@/generated";
import { pickOption } from "@/kit/testing";
import { useEditorStore, useProjectStore, useTrack } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { PianoRoll } from "@/features/piano-roll";
import { MpeSettingsFields } from "./MpeSettingsFields";
import { checkMpe, DEFAULT_MPE, memberChannels, pitchWindow, zoneSummary } from "./model";

let mock: MockTransport | undefined;
const store = () => useProjectStore.getState();

afterEach(() => {
  mock?.dispose();
  mock = undefined;
  store().reset();
  useEditorStore.getState().close();
  itemSelection.getState().clear();
});

async function flush() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

async function send(command: Command) {
  await act(async () => {
    await mock!.send(command);
  });
}

const midiTrack = (): Track => Object.values(store().project!.tracks).find((t) => t.kind === "Midi")!;

describe("MPE model", () => {
  it("validates like ether_model::check_mpe and lays out zones", () => {
    expect(checkMpe(DEFAULT_MPE)).toBeNull();
    expect(checkMpe({ ...DEFAULT_MPE, member_channels: 0 })).toMatch(/member channels/);
    expect(checkMpe({ ...DEFAULT_MPE, member_channels: 2.5 })).toMatch(/member channels/);
    expect(checkMpe({ ...DEFAULT_MPE, note_pitch_range: 97 })).toMatch(/note pitch range/);
    expect(checkMpe({ ...DEFAULT_MPE, master_pitch_range: -1 })).toMatch(/master pitch range/);
    expect(memberChannels(DEFAULT_MPE)).toEqual([2, 16]);
    expect(memberChannels({ zone: "Upper", member_channels: 4 })).toEqual([12, 15]);
    expect(zoneSummary({ ...DEFAULT_MPE, member_channels: 1 })).toBe("Master 1 · notes on 2");
    expect(pitchWindow(undefined)).toBe(12);
    expect(pitchWindow({ ...DEFAULT_MPE, note_pitch_range: 24 })).toBe(24);
  });
});

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div>
      <span>{label}</span>
      {children}
    </div>
  );
}

function Fields() {
  const track = useTrack(Object.values(useProjectStore((s) => s.project?.tracks) ?? {}).find((t) => t.kind === "Midi")?.id);
  return track ? <MpeSettingsFields track={track} Row={Row} /> : null;
}

describe("MPE settings in the inspector", () => {
  it("switches MPE on and off and edits the zone (one undo step each)", async () => {
    mock = new MockTransport({ timers: "manual", seed: 1 });
    render(
      <TransportProvider transport={mock}>
        <Fields />
      </TransportProvider>,
    );
    await waitFor(() => expect(screen.getByTestId("mpe-settings")).toBeTruthy());
    expect(screen.queryByRole("combobox", { name: "MPE zone" })).toBeNull();
    fireEvent.click(screen.getByRole("switch", { name: "MPE" }));
    await flush();
    expect(midiTrack().mpe).toEqual(DEFAULT_MPE);
    expect(screen.getByTestId("mpe-summary").textContent).toBe("Master 1 · notes on 2–16");
    pickOption(screen.getByRole("combobox", { name: "MPE zone" }), "Upper (master 16)");
    await flush();
    expect(midiTrack().mpe?.zone).toBe("Upper");
    const channels = screen.getByLabelText("MPE member channels") as HTMLInputElement;
    fireEvent.change(channels, { target: { value: "4" } });
    fireEvent.keyDown(channels, { key: "Enter" });
    await flush();
    expect(midiTrack().mpe?.member_channels).toBe(4);
    expect(screen.getByTestId("mpe-summary").textContent).toBe("Master 16 · notes on 12–15");
    await send(cmd("Edit", { type: "Undo" }));
    expect(midiTrack().mpe?.member_channels).toBe(15);
    fireEvent.click(screen.getByRole("switch", { name: "MPE" }));
    await flush();
    expect(midiTrack().mpe).toBeUndefined();
  });
});

const CLIP = "01MPETESTCLIP0000000000000";
const NOTE = "01MPETESTNOTE0000000000000";
// Piano roll: 40 px/beat from 0; lane values are padded by 4 px (y = 4 is the top value).
const PX = 40;

describe("per-note pitch in the piano roll", () => {
  it("the Pitch lane spans the track's note bend range; curves show in the note grid", async () => {
    mock = new MockTransport({ timers: "manual", seed: 1 });
    render(
      <TransportProvider transport={mock}>
        <PianoRoll />
      </TransportProvider>,
    );
    await waitFor(() => expect(store().project).not.toBeNull());
    const track = midiTrack();
    await send(cmd("Expression", { type: "SetTrackMpe", track: track.id, mpe: { ...DEFAULT_MPE, note_pitch_range: 24 } }));
    await send(cmd("Clip", { type: "CreateMidi", id: CLIP, track: track.id, start: 64, length: 8, name: "Mpe" }));
    await send(cmd("Note", { type: "Add", clip: CLIP, notes: [{ id: NOTE, pitch: 60, velocity: 0.8, start: 1, duration: 2 }] }));
    act(() => useEditorStore.getState().openClip(CLIP));
    await flush();
    expect(screen.queryByTestId("mpe-pitch-curves")).toBeNull();

    pickOption(screen.getByRole("combobox", { name: "Lane" }), "Note Pitch");
    const lane = screen.getByTestId("note-expression-lane");
    expect(lane.getAttribute("data-kind")).toBe("Note Pitch");
    // A stroke along the top edge over the note: +24 semitones (the window), not +96.
    fireEvent.pointerDown(lane, { button: 0, clientX: 1.2 * PX, clientY: 4 });
    for (let i = 1; i <= 6; i++) {
      fireEvent.pointerMove(window, { clientX: (1.2 + 0.2 * i) * PX, clientY: 4 });
      await flush();
    }
    fireEvent.pointerUp(window, { clientX: 2.4 * PX, clientY: 4 });
    await flush();
    const e = Object.values(store().project!.note_expressions).find((x) => x.note === NOTE && x.kind === "Pitch")!;
    expect(e.points.length).toBeGreaterThan(0);
    for (const p of e.points) expect(p.value).toBeCloseTo(24, 5);
    // Drawn inside the note grid.
    await waitFor(() => expect(screen.getByTestId("mpe-pitch-curve").getAttribute("data-note-id")).toBe(NOTE));
    // Other per-note kinds are offered too.
    pickOption(screen.getByRole("combobox", { name: "Lane" }), "Note Timbre");
    expect(screen.getByTestId("note-expression-lane").getAttribute("data-kind")).toBe("Note Timbre");
  });
});
