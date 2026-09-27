/** MockTransport: MIDI learn (midi-learn). */
import { describe, expect, it } from "vitest";
import type { ReplyValue } from "@/generated";
import { cmd } from "../../cmd";
import { project, trackNamed, useMock } from "./testUtils";

describe("MockTransport midiLearn", () => {
  const f = useMock();

  it("MIDI learn via simulateMidiInput, then the mapping drives the target", async () => {
    const keys = trackNamed(f, "Keys");
    await f.mock.send(cmd("MidiMap", { type: "Learn", target: { type: "Param", target: { type: "TrackPan", track: keys.id } } }));
    expect(f.events.at(-1)).toMatchObject({ type: "MidiMap", event: { type: "LearnChanged" } });
    f.mock.simulateMidiInput("kbd", [0xb2, 21, 64]);
    const maps = Object.values(project(f).midi_mappings);
    expect(maps).toHaveLength(1);
    expect(maps[0]!.source).toEqual({ port: "kbd", channel: 2, control: { type: "Cc", number: 21 } });
    expect(f.events.some((e) => e.type === "MidiMap" && e.event.type === "Learned")).toBe(true);
    f.mock.simulateMidiInput("kbd", [0xb2, 21, 127]);
    expect(project(f).tracks[keys.id]!.mixer.pan).toBeCloseTo(1, 6);
    const list = (await f.mock.send(cmd("MidiMap", { type: "List" }))) as Extract<ReplyValue, { type: "MidiMappings" }>;
    expect(list.mappings).toHaveLength(1);
    // Deleting the track removes its mappings.
    await f.mock.send(cmd("Track", { type: "Delete", id: keys.id }));
    expect(Object.keys(project(f).midi_mappings)).toHaveLength(0);
  });

  it("learns with the controller's defaults: Toggle for notes and on/off targets, replacing the target's mapping", async () => {
    const keys = trackNamed(f, "Keys");
    const mute = { type: "TrackMute", track: keys.id } as const;
    await f.mock.send(cmd("MidiMap", { type: "Learn", target: mute }));
    f.mock.simulateMidiInput("kbd", [0xb0, 5, 127]);
    let maps = Object.values(project(f).midi_mappings);
    expect(maps.map((m) => m.mode)).toEqual([{ type: "Toggle" }]);

    // A note-off never completes a learn; the note-on does, and replaces the old mapping.
    await f.mock.send(cmd("MidiMap", { type: "Learn", target: mute }));
    f.mock.simulateMidiInput("kbd", [0x80, 60, 0]);
    expect(Object.values(project(f).midi_mappings)[0]!.source.control).toEqual({ type: "Cc", number: 5 });
    f.mock.simulateMidiInput("kbd", [0x90, 60, 100]);
    maps = Object.values(project(f).midi_mappings);
    expect(maps).toHaveLength(1);
    expect(maps[0]!.source.control).toEqual({ type: "Note", key: 60 });
    expect(maps[0]!.mode).toEqual({ type: "Toggle" });

    // Continuous targets from a CC stay absolute.
    await f.mock.send(cmd("MidiMap", { type: "Learn", target: { type: "Param", target: { type: "TrackVolume", track: keys.id } } }));
    f.mock.simulateMidiInput("kbd", [0xb0, 7, 10]);
    const vol = Object.values(project(f).midi_mappings).find((m) => m.target.type === "Param")!;
    expect(vol.mode).toEqual({ type: "Absolute" });
  });
});
