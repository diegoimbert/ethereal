/** MockTransport: `AudioToMidi::*` (v0.3, audio-to-midi). */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AudioToMidiCommand, AudioToMidiOptions, Command, Event, Project } from "@/generated";
import { createDemoProject } from "../demoProject";
import { audioToMidiCommand, STEP_MS } from "./audioToMidi";
import type { MockHost } from "./host";
import { testId } from "./testUtils";

const options: AudioToMidiOptions = {
  sensitivity: 0.5,
  min_duration: 0.05,
  min_pitch: 21,
  max_pitch: 108,
  kick_key: 36,
  snare_key: 38,
  hihat_key: 42,
};

/** A host over a plain project: records events and the applied document commands. */
function fakeHost(project: Project) {
  const events: Event[] = [];
  const applied: Array<{ label: string; commands: Command[] }> = [];
  const host: MockHost = {
    project: () => project,
    emit: (e) => events.push(e),
    newId: testId,
    applyDocument: (commands, label) => void applied.push({ commands, label }),
    execute: () => {},
  };
  return { host, events, applied };
}

const send = (host: MockHost, c: AudioToMidiCommand) => audioToMidiCommand(c, host);
const audioClip = (p: Project) => Object.values(p.clips).find((c) => c.content.type === "Audio")!;
const kinds = (events: Event[]) => events.map((e) => (e.type === "AudioToMidi" ? e.event.type : e.type));

function start(clip: string, mode: "Melody" | "Harmony" | "Drums" = "Melody", job = "job-1") {
  return {
    type: "Start",
    job,
    clip,
    mode,
    options,
    track: testId(),
    new_clip: testId(),
    seed_notes: testId(),
    instrument: testId(),
  } as const satisfies AudioToMidiCommand;
}

describe("MockTransport audio to MIDI (audio-to-midi)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("replies Unsupported without the mock host", () => {
    expect(() => audioToMidiCommand({ type: "Cancel", job: "j" })).toThrow(expect.objectContaining({ code: "Unsupported" }));
  });

  it("reports progress, then applies one undo step below the source track and reports Done", () => {
    const p = createDemoProject();
    const { host, events, applied } = fakeHost(p);
    const clip = audioClip(p);
    const c = start(clip.id, "Drums");
    expect(send(host, c)).toEqual({ type: "Unit" });
    expect(events).toEqual([]);
    vi.advanceTimersByTime(STEP_MS * 3);
    expect(kinds(events)).toEqual(["Progress", "Progress", "Progress"]);
    vi.advanceTimersByTime(STEP_MS);
    expect(kinds(events)).toEqual(["Progress", "Progress", "Progress", "Progress", "Done"]);
    const done = events.at(-1)!;
    expect(done).toMatchObject({ event: { type: "Done", job: "job-1", track: c.track, clip: c.new_clip } });

    expect(applied).toHaveLength(1);
    const [step] = applied;
    expect(step!.label).toBe("Convert to MIDI");
    const [track, newClip, notes, device] = step!.commands;
    const source = p.tracks[clip.track]!;
    expect(track).toMatchObject({ domain: "Track", command: { type: "Create", id: c.track, kind: "Midi", parent: source.parent } });
    expect(newClip).toMatchObject({
      domain: "Clip",
      command: { type: "CreateMidi", id: c.new_clip, track: c.track, start: clip.start, length: clip.length },
    });
    expect(notes).toMatchObject({ domain: "Note", command: { type: "Add", clip: c.new_clip } });
    const add = (notes as Extract<Command, { domain: "Note" }>).command;
    const keys = new Set(add.type === "Add" ? add.notes.map((n) => n.pitch) : []);
    expect([...keys].sort()).toEqual([36, 38, 42]);
    expect(device).toMatchObject({
      domain: "Device",
      command: { type: "Insert", id: c.instrument, track: c.track, device: { type: "Builtin", device: { type: "DrumRack" } } },
    });
    // "Below": inserted before the source's next sibling (if any).
    const sibs = Object.values(p.tracks)
      .filter((t) => t.parent === source.parent)
      .sort((a, b) => (a.order < b.order ? -1 : 1));
    const next = sibs[sibs.findIndex((t) => t.id === source.id) + 1]?.id ?? null;
    expect((track as Extract<Command, { domain: "Track" }>).command).toMatchObject({ before: next });
  });

  it("one job at a time; cancel; the clip going away cancels", () => {
    const p = createDemoProject();
    const { host, events, applied } = fakeHost(p);
    const clip = audioClip(p);
    send(host, start(clip.id, "Melody", "a"));
    expect(() => send(host, start(clip.id, "Melody", "b"))).toThrow(expect.objectContaining({ code: "InvalidState" }));
    expect(send(host, { type: "Cancel", job: "a" })).toEqual({ type: "Unit" });
    expect(kinds(events)).toEqual(["Cancelled"]);
    vi.advanceTimersByTime(STEP_MS * 10);
    expect(kinds(events)).toEqual(["Cancelled"]);
    expect(applied).toEqual([]);
    // Unknown job: no-op.
    send(host, { type: "Cancel", job: "nope" });
    expect(events).toHaveLength(1);

    send(host, start(clip.id, "Harmony", "c"));
    delete p.clips[clip.id];
    vi.advanceTimersByTime(STEP_MS);
    expect(events.at(-1)).toMatchObject({ event: { type: "Cancelled", job: "c" } });
    expect(applied).toEqual([]);
  });

  it("validates like the engine", () => {
    const p = createDemoProject();
    const { host } = fakeHost(p);
    const audio = audioClip(p);
    const midi = Object.values(p.clips).find((c) => c.content.type === "Midi")!;
    const code = (c: AudioToMidiCommand) => {
      try {
        send(host, c);
        return "ok";
      } catch (e) {
        return (e as { code: string }).code;
      }
    };
    expect(code(start(midi.id))).toBe("InvalidArgument");
    expect(code(start(testId()))).toBe("NotFound");
    expect(code({ ...start(audio.id), job: "" })).toBe("InvalidArgument");
    expect(code({ ...start(audio.id), track: audio.track })).toBe("InvalidArgument");
    expect(code({ ...start(audio.id), options: { ...options, sensitivity: 2 } })).toBe("InvalidArgument");
    expect(code({ ...start(audio.id), options: { ...options, min_pitch: 90, max_pitch: 80 } })).toBe("InvalidArgument");
  });
});
