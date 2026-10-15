import { describe, expect, it } from "vitest";
import { clipGrabAt, EDGE_MAX_SPEED, EDGE_ZONE_PX, edgeSpeed, MIN_TRIM_PX, trimZones } from "./clipTouch";

const box = (id: string, x0: number, x1: number, y0 = 0, y1 = 60) => ({ id, rect: { x0, x1, y0, y1 } });

describe("clipGrabAt", () => {
  const a = box("a", 100, 300);
  const sel = new Set(["a"]);

  it("a selected clip's trim handles: 44 px, half outside the clip", () => {
    expect(clipGrabAt([a], sel, 80, 30)).toEqual({ clip: "a", grab: "resize-start" });
    expect(clipGrabAt([a], sel, 121, 30)).toEqual({ clip: "a", grab: "resize-start" });
    expect(clipGrabAt([a], sel, 123, 30)).toEqual({ clip: "a", grab: "move" });
    expect(clipGrabAt([a], sel, 321, 30)).toEqual({ clip: "a", grab: "resize-end" });
    expect(clipGrabAt([a], sel, 323, 30)).toBeNull();
    expect(clipGrabAt([a], sel, 200, 30)).toEqual({ clip: "a", grab: "move" });
  });

  it("an unselected clip has no handles: only its body", () => {
    expect(clipGrabAt([a], new Set(), 90, 30)).toBeNull();
    expect(clipGrabAt([a], new Set(), 105, 30)).toEqual({ clip: "a", grab: "move" });
  });

  it("a narrow clip keeps a body (inside part capped at a third); a tiny one has no handles", () => {
    const n = box("n", 100, 130);
    expect(trimZones(n.rect)).toEqual({ start: [78, 110], end: [120, 152] });
    expect(clipGrabAt([n], new Set(["n"]), 115, 30)).toEqual({ clip: "n", grab: "move" });
    const tiny = box("t", 100, 100 + MIN_TRIM_PX - 1);
    expect(trimZones(tiny.rect)).toBeNull();
    expect(clipGrabAt([tiny], new Set(["t"]), 101, 30)).toEqual({ clip: "t", grab: "move" });
  });

  it("between two selected clips the nearest edge wins; other rows don't count", () => {
    const b = box("b", 310, 400);
    const both = new Set(["a", "b"]);
    expect(clipGrabAt([a, b], both, 302, 30)).toEqual({ clip: "a", grab: "resize-end" });
    expect(clipGrabAt([a, b], both, 308, 30)).toEqual({ clip: "b", grab: "resize-start" });
    expect(clipGrabAt([a], sel, 200, 70)).toBeNull();
  });
});

describe("edgeSpeed", () => {
  it("zero in the middle, ramping to the maximum at the edges, signed", () => {
    expect(edgeSpeed(500, 0, 1000)).toBe(0);
    expect(edgeSpeed(EDGE_ZONE_PX, 0, 1000)).toBe(0);
    expect(edgeSpeed(EDGE_ZONE_PX / 2, 0, 1000)).toBe(-EDGE_MAX_SPEED / 2);
    expect(edgeSpeed(0, 0, 1000)).toBe(-EDGE_MAX_SPEED);
    expect(edgeSpeed(1000, 0, 1000)).toBe(EDGE_MAX_SPEED);
    expect(edgeSpeed(1100, 0, 1000)).toBe(EDGE_MAX_SPEED);
  });
});
