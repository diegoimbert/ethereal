import { describe, expect, it } from "vitest";
import type { Clip, Device, DeviceId, LoadProgress, MediaId, Project, TrackId } from "@/generated";
import { loadingOf, loadingTracksOf } from "./trackLoading";

const device = (id: string, track: string): Device => ({ id, track }) as unknown as Device;
const clip = (id: string, track: string, type: "Audio" | "Midi", media?: string): Clip => ({ id, track, content: { type, media } }) as unknown as Clip;
const project = {
  devices: { A: device("A", "T1"), B: device("B", "T2") },
  clips: {
    c0: clip("c0", "T1", "Midi"),
    c1: clip("c1", "T3", "Audio", "m1"),
    c2: clip("c2", "T4", "Midi"),
    c3: clip("c3", "T5", "Audio", "m2"),
    c4: clip("c4", "T5", "Audio", "m3"),
    c5: clip("c5", "T5", "Audio", "m2"),
  },
} as unknown as Project;
const load = (p: Partial<LoadProgress>): LoadProgress => ({ plugins_loaded: 0, plugins_total: 0, media_loaded: 0, media_total: 0, ...p });
const ids = (s: ReadonlySet<string>) => [...s].sort();

describe("tracks still loading (#260)", () => {
  it("are those with a plugin still to load (all their clips wait)", () => {
    const l = loadingOf(project, ["A" as DeviceId], load({ plugins_total: 2, plugins_loaded: 1 }));
    expect(ids(l.tracks)).toEqual(["T1"]);
    expect(ids(l.clips)).toEqual(["c0"]);
    expect(loadingTracksOf(project, [], load({ plugins_total: 2, plugins_loaded: 2 })).size).toBe(0);
  });

  it("and those whose own audio files still decode: only those clips wait", () => {
    const l = loadingOf(project, [], load({ media_total: 3, media_loaded: 1, media_pending: ["m2" as MediaId, "m3" as MediaId] }));
    // T3's file is in: it is ready before the rest.
    expect(ids(l.tracks)).toEqual(["T5"]);
    expect(ids(l.clips)).toEqual(["c3", "c4", "c5"]);
    expect(l.media.get("T5" as TrackId)).toEqual([0, 2]);
    const later = loadingOf(project, [], load({ media_total: 3, media_loaded: 2, media_pending: ["m3" as MediaId] }));
    expect(ids(later.clips)).toEqual(["c4"]);
    expect(later.media.get("T5" as TrackId)).toEqual([1, 2]);
  });

  it("every audio clip waits when the engine doesn't say which files decode", () => {
    expect(ids(loadingTracksOf(project, ["A" as DeviceId], load({ media_total: 3, media_loaded: 1 })))).toEqual(["T1", "T3", "T5"]);
    // Audio files loaded (the audio starting): none.
    expect(loadingTracksOf(project, [], load({ media_total: 3, media_loaded: 3 })).size).toBe(0);
    // Loaded, or no project.
    expect(loadingTracksOf(project, [], null).size).toBe(0);
    expect(loadingTracksOf(null, ["A" as DeviceId], load({ media_total: 1 })).size).toBe(0);
  });
});
