import { act, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useProjectStore } from "@/state";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { loadRecordingSetup, recordingSetupKey, useKeepRecordingSetup } from "./keep";

const store = () => useProjectStore.getState();
let mock: MockTransport;

function Keep() {
  useKeepRecordingSetup();
  return null;
}

async function flush() {
  await act(async () => {
    for (let i = 0; i < 20; i++) await Promise.resolve();
  });
}

const midiTrack = () => Object.values(store().project!.tracks).find((t) => t.kind === "Midi")!;

beforeEach(async () => {
  localStorage.clear();
  mock = new MockTransport({ timers: "manual", seed: 1 });
  render(
    <TransportProvider transport={mock}>
      <Keep />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  await flush();
});

afterEach(() => {
  mock.dispose();
  store().reset();
  localStorage.clear();
});

describe("arms and punch are kept per project (#717)", () => {
  it("come back when the project opens again; another project has its own", async () => {
    const first = store().project!.id;
    const track = midiTrack();
    await act(() => mock.send(cmd("Recording", { type: "Arm", track: track.id, armed: true, exclusive: false })));
    await act(() => mock.send(cmd("Recording", { type: "SetPunch", enabled: true })));
    await flush();
    expect(loadRecordingSetup(first)).toEqual({ armed: [track.id], punch: true });

    // Another project: nothing armed, punch off (it wasn't on there).
    const second = "01KEEPRECTESTPROJECT000000";
    await act(() => mock.send(cmd("Project", { type: "Create", id: second, name: "Other" })));
    await flush();
    expect(store().project!.id).toBe(second);
    expect(store().armedTracks).toEqual([]);
    expect(localStorage.getItem(recordingSetupKey(second))).toBeNull();
    // The first project's setup survived its load clearing the arms.
    expect(loadRecordingSetup(first)).toEqual({ armed: [track.id], punch: true });

    // Back: the same arms and punch.
    const punches: boolean[] = [];
    mock.onEvent((e) => e.type === "Recording" && e.event.type === "PunchChanged" && punches.push(e.event.enabled));
    await act(() => mock.send(cmd("Project", { type: "Open", id: first })));
    await flush();
    expect(store().armedTracks).toEqual([track.id]);
    expect(punches.at(-1)).toBe(true);

    // Disarming is kept too; nothing left to keep removes the entry.
    await act(() => mock.send(cmd("Recording", { type: "Arm", track: track.id, armed: false, exclusive: false })));
    await act(() => mock.send(cmd("Recording", { type: "SetPunch", enabled: false })));
    await flush();
    expect(localStorage.getItem(recordingSetupKey(first))).toBeNull();
  });

  it("reads only well-formed data", async () => {
    const id = store().project!.id;
    localStorage.setItem(recordingSetupKey(id), JSON.stringify({ armed: ["01GONEGONEGONEGONEGONE00000", 4], punch: "yes" }));
    expect(loadRecordingSetup(id)).toEqual({ armed: ["01GONEGONEGONEGONEGONE00000"], punch: false });
    localStorage.setItem(recordingSetupKey(id), "{oops");
    expect(loadRecordingSetup(id)).toBeNull();
  });
});
