import { describe, expect, it } from "vitest";
import type { Track } from "@/generated";
import type { Row } from "../../layout";
import type { NewLane } from "../../newTrackDrag";
import { ghostLaneRows } from "./ghostLanes";

const row = (id: string, y: number, height: number): Row => ({ track: { id } as Track, depth: 0, y, laneHeight: height, height });

describe("ghostLaneRows", () => {
  it("stacks the new lanes under the last row, with stand-in tracks of their kind and color", () => {
    const lanes: NewLane[] = [
      { id: "N1", kind: "Midi", source: "T1", color: 0x112233 },
      { id: "N2", kind: "Audio", source: "T2", color: 0x445566 },
    ];
    const rows = ghostLaneRows(lanes, [row("T1", 0, 56), row("T2", 56, 80)], 56);
    expect(rows.map((r) => [r.track.id, r.track.kind, r.track.color, r.y, r.laneHeight, r.height])).toEqual([
      ["N1", "Midi", 0x112233, 136, 56, 56],
      ["N2", "Audio", 0x445566, 192, 56, 56],
    ]);
    // Not frozen, not a draft: drawn like a lane, but not a track of the project.
    expect(rows[0]!.track.freeze).toBeNull();
  });

  it("starts at the top without rows", () => {
    expect(ghostLaneRows([{ id: "N1", kind: "Midi", source: "T1", color: 0 }], [], 40)[0]!.y).toBe(0);
  });
});
