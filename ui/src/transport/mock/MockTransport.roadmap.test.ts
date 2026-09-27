/** MockTransport simulations of the roadmap v2 commands (contracts-2). */
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Event, Project, ReplyValue } from "@/generated";
import { cmd } from "../cmd";
import { CommandFailedError } from "../EngineTransport";
import { newId } from "../ids";
import { MockTransport } from "./MockTransport";

let mock: MockTransport;
let events: Event[];

beforeEach(async () => {
  mock = new MockTransport({ timers: "manual", seed: 7 });
  events = [];
  mock.onEvent((e) => events.push(e));
  await mock.connect();
});
afterEach(() => mock.dispose());

const p = (): Project => mock.snapshot();
const track = (name: string) => Object.values(p().tracks).find((t) => t.name === name)!;
const undo = () => mock.send(cmd("Edit", { type: "Undo" }));

describe("MockTransport roadmap v2", () => {
  it("creates new audio clips unwarped (Repitch, like the controller)", async () => {
    const media = Object.values(p().media)[0]!;
    const id = newId();
    await mock.send(cmd("Clip", { type: "CreateAudio", id, track: track("Drums").id, start: 32, media: media.id }));
    const c = p().clips[id]!;
    expect(c.content.type === "Audio" && c.content.warp).toEqual({ enabled: false, mode: "Repitch", source_bpm: null });
  });

  it("tempo map CRUD is undoable and protects beat 0", async () => {
    const id = newId();
    await mock.send(cmd("Tempo", { type: "AddTempoPoint", id, time: 16, bpm: 90, curve: "Step" }));
    expect(p().tempo_points[id]?.bpm).toBe(90);
    await mock.send(cmd("Tempo", { type: "EditTempoPoint", id, time: null, bpm: 2000, curve: "Linear" }));
    expect(p().tempo_points[id]).toMatchObject({ bpm: 999, curve: "Linear" });
    const zero = Object.values(p().tempo_points).find((t) => t.time === 0)!;
    await expect(mock.send(cmd("Tempo", { type: "RemoveTempoPoints", ids: [zero.id] }))).rejects.toBeInstanceOf(CommandFailedError);
    await undo();
    await undo();
    expect(p().tempo_points[id]).toBeUndefined();
    await mock.send(cmd("Tempo", { type: "SetMetronomeSettings", volume: 12, accent: false, sound: "Wood" }));
    expect(p().settings).toMatchObject({ metronome_volume: 6, metronome_accent: false, metronome_sound: "Wood" });
  });

  it("markers: add (idempotent), move, rename, remove", async () => {
    const id = newId();
    const add = cmd("Marker", { type: "Add", id, position: 8, name: null, color: null });
    await mock.send(add);
    await mock.send(add);
    expect(Object.keys(p().markers)).toEqual([id]);
    expect(p().markers[id]!.name).toBe("Marker 1");
    await mock.send(cmd("Marker", { type: "Move", id, position: 12 }));
    await mock.send(cmd("Marker", { type: "Rename", id, name: "Chorus" }));
    expect(p().markers[id]).toMatchObject({ position: 12, name: "Chorus" });
    await mock.send(cmd("Marker", { type: "Remove", ids: [id] }));
    expect(p().markers[id]).toBeUndefined();
  });

  it("MIDI learn via simulateMidiInput, then the mapping drives the target", async () => {
    const keys = track("Keys");
    await mock.send(cmd("MidiMap", { type: "Learn", target: { type: "Param", target: { type: "TrackPan", track: keys.id } } }));
    expect(events.at(-1)).toMatchObject({ type: "MidiMap", event: { type: "LearnChanged" } });
    mock.simulateMidiInput("kbd", [0xb2, 21, 64]);
    const maps = Object.values(p().midi_mappings);
    expect(maps).toHaveLength(1);
    expect(maps[0]!.source).toEqual({ port: "kbd", channel: 2, control: { type: "Cc", number: 21 } });
    expect(events.some((e) => e.type === "MidiMap" && e.event.type === "Learned")).toBe(true);
    mock.simulateMidiInput("kbd", [0xb2, 21, 127]);
    expect(p().tracks[keys.id]!.mixer.pan).toBeCloseTo(1, 6);
    const list = (await mock.send(cmd("MidiMap", { type: "List" }))) as Extract<ReplyValue, { type: "MidiMappings" }>;
    expect(list.mappings).toHaveLength(1);
    // Deleting the track removes its mappings.
    await mock.send(cmd("Track", { type: "Delete", id: keys.id }));
    expect(Object.keys(p().midi_mappings)).toHaveLength(0);
  });

  it("export: progress, download, chunked read, release", async () => {
    await expect(
      mock.send(
        cmd("Export", {
          type: "Render",
          job: "j0",
          request: {
            range: { type: "Loop" },
            format: { container: "Flac", bit_depth: "Float32", sample_rate: null },
            mode: { type: "Mix" },
            normalize: false,
            tail_seconds: 0,
            name: null,
          },
        }),
      ),
    ).rejects.toBeInstanceOf(CommandFailedError);
    const reply = await mock.send(
      cmd("Export", {
        type: "Render",
        job: "j1",
        request: {
          range: { type: "Project" },
          format: { container: "Wav", bit_depth: "Int16", sample_rate: 44100 },
          mode: { type: "Mix" },
          normalize: true,
          tail_seconds: 1,
          name: "Song",
        },
      }),
    );
    expect(reply).toEqual({ type: "ExportStarted", job: "j1" });
    mock.tick(16);
    mock.tick(16);
    const exportEvents = events.flatMap((e) => (e.type === "Export" ? [e.event] : []));
    expect(exportEvents.map((e) => e.type)).toEqual(["Progress", "Progress", "Done"]);
    const done = exportEvents[2]!;
    if (done.type !== "Done" || done.result.type !== "Download") throw new Error("expected a download");
    const dl = done.result.downloads[0]!;
    expect(dl).toMatchObject({ name: "Song.wav", mime: "audio/wav" });
    let bytes = "";
    for (let offset = 0; ; ) {
      const r = (await mock.send(cmd("Export", { type: "ReadChunk", token: dl.token, offset, length: 1000 }))) as Extract<ReplyValue, { type: "Bytes" }>;
      const chunk = atob(r.chunk.data);
      bytes += chunk;
      offset += chunk.length;
      if (r.chunk.eof) break;
    }
    expect(bytes.length).toBe(dl.size);
    expect(bytes.slice(0, 4)).toBe("RIFF");
    expect(bytes.slice(8, 12)).toBe("WAVE");
    await mock.send(cmd("Export", { type: "Release", token: dl.token }));
    await expect(mock.send(cmd("Export", { type: "ReadChunk", token: dl.token, offset: 0, length: 10 }))).rejects.toBeInstanceOf(CommandFailedError);
  });

  it("drum racks: pads, note swap, pad chains, cascade on rack removal", async () => {
    const t = track("Keys");
    const rack = newId();
    await mock.send(cmd("Device", { type: "Insert", id: rack, track: t.id, device: { type: "Builtin", device: { type: "DrumRack" } }, before: null }));
    const [a, b] = [newId(), newId()];
    await mock.send(cmd("DrumRack", { type: "AddPad", id: a, rack, note: 36, name: null }));
    await mock.send(cmd("DrumRack", { type: "AddPad", id: b, rack, note: 38, name: "Snare" }));
    expect(p().drum_pads[a]).toMatchObject({ note: 36, name: "C1", choke_group: null });
    await mock.send(cmd("DrumRack", { type: "SetPadNote", id: a, note: 38 }));
    expect([p().drum_pads[a]!.note, p().drum_pads[b]!.note]).toEqual([38, 36]);
    const sampler = newId();
    await mock.send(
      cmd("DrumRack", {
        type: "InsertDevice",
        id: sampler,
        pad: a,
        device: { type: "Builtin", device: { type: "Sampler", sample: null, slices: { enabled: false, base_note: 36, markers: [] } } },
        before: null,
      }),
    );
    expect(p().devices[sampler]).toMatchObject({ pad: a, track: t.id });
    await mock.send(cmd("Device", { type: "Remove", id: rack }));
    expect(p().devices[sampler]).toBeUndefined();
    expect(Object.keys(p().drum_pads)).toHaveLength(0);
    await undo();
    expect(p().devices[sampler]).toBeDefined();
    expect(Object.keys(p().drum_pads)).toHaveLength(2);
  });

  it("humanize is deterministic for a seed", async () => {
    const chords = Object.values(p().clips).find((c) => c.name === "Chords")!;
    const humanize = cmd("Groove", { type: "Humanize", clip: chords.id, notes: null, timing: 0.05, velocity: 0.1, seed: 99 });
    const notesOf = () =>
      Object.values(p().notes)
        .filter((n) => n.clip === chords.id)
        .map((n) => [n.id, n.start, n.velocity]);
    const before = notesOf();
    await mock.send(humanize);
    const first = notesOf();
    expect(first).not.toEqual(before);
    await undo();
    expect(notesOf()).toEqual(before);
    await mock.send(humanize);
    expect(notesOf()).toEqual(first);
    await mock.send(cmd("Groove", { type: "SetSwing", amount: 2, grid: 0.5 }));
    expect(p().settings).toMatchObject({ swing: 1, swing_grid: 0.5 });
  });

  it("collab and uploads are unsupported", async () => {
    await expect(mock.send(cmd("Collab", { type: "Leave" }))).rejects.toMatchObject({ code: "Unsupported" });
    await expect(mock.send(cmd("Media", { type: "BeginUpload", upload: "u", name: "a.wav", size: 3 }))).rejects.toMatchObject({
      code: "Unsupported",
    });
  });
});
