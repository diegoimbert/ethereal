import { describe, expect, it } from "vitest";
import type { MediaId } from "@/generated";
import { dragZone, hitTest, mapDropped, newZone, noteName, rootFromName, yToVel, xToKey, zoneRect } from "./zoneMath";

const m = (n: number) => `m${n}` as MediaId;

describe("zone math", () => {
  it("names notes with C3 = 60", () => {
    expect(noteName(60)).toBe("C3");
    expect(noteName(0)).toBe("C-2");
    expect(noteName(61)).toBe("C#3");
    expect(noteName(127)).toBe("G8");
  });

  it("parses root keys from sample names", () => {
    expect(rootFromName("Piano C3.wav")).toBe(60);
    expect(rootFromName("Bass A1.wav")).toBe(45);
    expect(rootFromName("strings_F#2_mf.wav")).toBe(54);
    expect(rootFromName("Rhodes Db4.aif")).toBe(73);
    expect(rootFromName("kick C-2.wav")).toBe(0);
    expect(rootFromName("Vox_060.wav")).toBe(60);
    expect(rootFromName("piano 72.flac")).toBe(72);
    // No note: plain words, single digits, out of range.
    expect(rootFromName("Pad C.wav")).toBeNull();
    expect(rootFromName("Chop 1.wav")).toBeNull();
    expect(rootFromName("hit 300.wav")).toBeNull();
  });

  it("maps several samples across keys between their roots", () => {
    const zones = mapDropped(
      [
        { media: m(3), name: "Piano C4.wav" },
        { media: m(1), name: "Piano C2.wav" },
        { media: m(2), name: "Piano C3.wav" },
      ],
      30,
      false,
    );
    expect(zones.map((z) => [z.media, z.root_key, z.keys.lo, z.keys.hi])).toEqual([
      [m(1), 48, 30, 54],
      [m(2), 60, 55, 66],
      [m(3), 72, 67, 72],
    ]);
  });

  it("maps unnamed samples to consecutive keys from the drop key", () => {
    const zones = mapDropped(
      [
        { media: m(1), name: "Hit.wav" },
        { media: m(2), name: "Hit (2).wav" },
      ],
      36,
      false,
    );
    expect(zones.map((z) => [z.root_key, z.keys.lo, z.keys.hi])).toEqual([
      [36, 36, 36],
      [37, 37, 37],
    ]);
  });

  it("maps one sample to the whole keyboard on an empty map, else to its key", () => {
    expect(mapDropped([{ media: m(1), name: "Pad C.wav" }], 64, true)[0]).toMatchObject({ root_key: 64, keys: { lo: 0, hi: 127 } });
    expect(mapDropped([{ media: m(1), name: "Bass A1.wav" }], 64, false)[0]).toMatchObject({ root_key: 45, keys: { lo: 45, hi: 45 } });
  });

  it("hit-tests edges of the selected zone, then zone bodies", () => {
    const size = { w: 1280, h: 127 };
    const zones = [newZone(m(1), 60, { lo: 0, hi: 127 }), newZone(m(2), 60, { lo: 60, hi: 71 })];
    const r = zoneRect(zones[1]!, size);
    expect(r).toMatchObject({ x: 600, w: 120 });
    expect(hitTest(zones, 1, 600, 60, size, 4)).toEqual({ zone: 1, part: "left" });
    expect(hitTest(zones, 1, 721, 60, size, 4)).toEqual({ zone: 1, part: "right" });
    expect(hitTest(zones, 1, 650, 0, size, 4)).toEqual({ zone: 1, part: "top" });
    // Topmost (last) zone under the pointer wins; elsewhere the full-range zone.
    expect(hitTest(zones, null, 650, 60, size, 4)).toEqual({ zone: 1, part: "body" });
    expect(hitTest(zones, null, 100, 60, size, 4)).toEqual({ zone: 0, part: "body" });
    expect(hitTest([], null, 100, 60, size, 4)).toBeNull();
    expect(xToKey(650, size)).toBe(65);
    expect(yToVel(0, size)).toBe(127);
    expect(yToVel(127, size)).toBe(1);
  });

  it("drags edges in order and moves bodies inside the map", () => {
    const z = newZone(m(1), 60, { lo: 60, hi: 71 });
    expect(dragZone(z, "right", 5, 0).keys).toEqual({ lo: 60, hi: 76 });
    expect(dragZone(z, "right", -20, 0).keys).toEqual({ lo: 60, hi: 60 });
    expect(dragZone(z, "left", -100, 0).keys).toEqual({ lo: 0, hi: 71 });
    expect(dragZone(z, "bottom", 0, 63).velocities).toEqual({ lo: 64, hi: 127 });
    expect(dragZone(z, "top", 0, -200).velocities).toEqual({ lo: 1, hi: 1 });
    const moved = dragZone(z, "body", 100, 0);
    expect(moved.keys).toEqual({ lo: 116, hi: 127 });
    expect(moved.root_key).toBe(116);
  });
});
