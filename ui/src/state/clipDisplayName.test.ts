// A clip without a name of its own (new clips have none) reads as its track.
import { describe, expect, it } from "vitest";
import type { Project } from "@/generated";
import { clipDisplayName } from "./defaultNames";

const project = { tracks: { t: { name: "Bass" } } } as unknown as Pick<Project, "tracks">;

describe("clipDisplayName", () => {
  it("is the clip's own name, else its track's", () => {
    expect(clipDisplayName(project, { name: "Hook", track: "t" })).toBe("Hook");
    expect(clipDisplayName(project, { name: "", track: "t" })).toBe("Bass");
    expect(clipDisplayName(project, { name: "", track: "gone" })).toBe("");
  });
});
