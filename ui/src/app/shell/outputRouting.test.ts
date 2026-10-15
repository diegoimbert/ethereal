import { describe, expect, it } from "vitest";
import { createBeatSketchProject } from "@/transport/mock/demoProject";
import { outputTargets, outputValue, parseOutputValue } from "./outputRouting";

describe("output routing", () => {
  it("encodes outputs", () => {
    for (const o of [{ type: "Default" }, { type: "None" }, { type: "Track", track: "X" }] as const) {
      expect(parseOutputValue(outputValue(o))).toEqual(o);
    }
  });

  it("excludes cycles from outputs", () => {
    const p = createBeatSketchProject();
    const byName = (name: string) => Object.values(p.tracks).find((t) => t.name === name)!;
    expect(outputTargets(p.tracks, byName("Kick")).map((t) => t.name)).toEqual(["Drum Bus"]);
    expect(outputTargets(p.tracks, byName("Drum Bus"))).toEqual([]);
  });
});
