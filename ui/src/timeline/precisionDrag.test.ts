// @vitest-environment node
import { describe, expect, it } from "vitest";
import { precisionDrag, PRECISION_RATIO } from "./precisionDrag";

describe("precisionDrag (#793)", () => {
  const start = { x: 100, y: 200 };

  it("follows the pointer without the modifier, and moves 1/10 as far with it", () => {
    expect(PRECISION_RATIO).toBe(0.1);
    const plain = precisionDrag(start);
    expect(plain({ x: 150, y: 100 }, false)).toEqual({ dx: 50, dy: -100 });
    const fine = precisionDrag(start);
    const o = fine({ x: 150, y: 100 }, true);
    expect(o.dx).toBeCloseTo(5, 12);
    expect(o.dy).toBeCloseTo(-10, 12);
  });

  it("never jumps when the modifier is pressed or released mid-drag (the anchor rebases)", () => {
    const off = precisionDrag(start);
    expect(off({ x: 140, y: 200 }, false).dx).toBe(40);
    // Alt pressed: the same pointer position gives the same offset...
    expect(off({ x: 140, y: 200 }, true).dx).toBeCloseTo(40, 12);
    // ...and only the motion since counts a tenth.
    expect(off({ x: 240, y: 200 }, true).dx).toBeCloseTo(50, 12);
    // Released: no jump back to the raw pointer offset (140); full speed from here.
    expect(off({ x: 240, y: 200 }, false).dx).toBeCloseTo(50, 12);
    expect(off({ x: 250, y: 200 }, false).dx).toBeCloseTo(60, 12);
  });

  it("rebases at the last pointer seen when the modifier changes between moves", () => {
    const off = precisionDrag(start);
    off({ x: 120, y: 200 }, false);
    // Alt pressed between two moves: the 20 px before the press were full speed.
    expect(off({ x: 220, y: 200 }, true).dx).toBeCloseTo(20 + 10, 12);
  });

  it("slows each axis on its own (Shift + Alt: fine time, full-speed value)", () => {
    const off = precisionDrag(start);
    const both = { x: true, y: false };
    const o = off({ x: 200, y: 100 }, both);
    expect(o.dx).toBeCloseTo(10, 12);
    expect(o.dy).toBe(-100);
    // Shift released (Alt still held): the value goes fine from here, no jump.
    const o2 = off({ x: 200, y: 100 }, true);
    expect(o2).toEqual(o);
    expect(off({ x: 200, y: 0 }, true).dy).toBeCloseTo(-110, 12);
  });
});
