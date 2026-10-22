import { describe, expect, it } from "vitest";
import type { Clip, Device, DeviceId, LoadProgress, Project, TrackId } from "@/generated";
import { loadingTracksOf } from "./trackLoading";

const device = (id: string, track: string): Device => ({ id, track }) as unknown as Device;
const clip = (id: string, track: string, type: "Audio" | "Midi"): Clip => ({ id, track, content: { type } }) as unknown as Clip;
const project = {
  devices: { A: device("A", "T1"), B: device("B", "T2") },
  clips: { c1: clip("c1", "T3", "Audio"), c2: clip("c2", "T4", "Midi") },
} as unknown as Project;
const load = (p: Partial<LoadProgress>): LoadProgress => ({ plugins_loaded: 0, plugins_total: 0, media_loaded: 0, media_total: 0, ...p });
const ids = (s: ReadonlySet<TrackId>) => [...s].sort();

describe("tracks still loading (#260)", () => {
  it("are those with a plugin still to load", () => {
    expect(ids(loadingTracksOf(project, ["B" as DeviceId], load({ plugins_total: 2, plugins_loaded: 1 })))).toEqual(["T2"]);
    expect(loadingTracksOf(project, [], load({ plugins_total: 2, plugins_loaded: 2 })).size).toBe(0);
  });

  it("and, while audio files load, those with audio clips", () => {
    expect(ids(loadingTracksOf(project, ["A" as DeviceId], load({ media_total: 3, media_loaded: 1 })))).toEqual(["T1", "T3"]);
    // Audio files loaded (the audio starting): none.
    expect(loadingTracksOf(project, [], load({ media_total: 3, media_loaded: 3 })).size).toBe(0);
    // Loaded, or no project.
    expect(loadingTracksOf(project, [], null).size).toBe(0);
    expect(loadingTracksOf(null, ["A" as DeviceId], load({ media_total: 1 })).size).toBe(0);
  });
});
