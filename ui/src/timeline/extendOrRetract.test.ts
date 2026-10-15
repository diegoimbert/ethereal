import { describe, expect, it } from "vitest";
import { extendOrRetract, movedEdge } from "./extendOrRetract";

const sel = { start: 4, end: 8, tracks: ["a"] };

describe("extendOrRetract", () => {
  it("extends to a click left of the range", () => {
    expect(extendOrRetract(sel, 2)).toEqual({ start: 2, end: 8, tracks: ["a"] });
  });
  it("extends to a click right of the range", () => {
    expect(extendOrRetract(sel, 11)).toEqual({ start: 4, end: 11, tracks: ["a"] });
  });
  it("retracts the start to a click inside, nearer the start", () => {
    expect(extendOrRetract(sel, 5)).toMatchObject({ start: 5, end: 8 });
  });
  it("retracts the end to a click inside, nearer the end", () => {
    expect(extendOrRetract(sel, 7)).toMatchObject({ start: 4, end: 7 });
  });
  it("moves the end for a click exactly in the middle", () => {
    expect(movedEdge(sel, 6)).toBe("end");
    expect(extendOrRetract(sel, 6)).toMatchObject({ start: 4, end: 6 });
  });
  it("snaps the click before deciding", () => {
    const toBeat = (b: number) => Math.round(b);
    expect(extendOrRetract(sel, 1.7, toBeat)).toMatchObject({ start: 2, end: 8 });
    expect(extendOrRetract(sel, 6.6, toBeat)).toMatchObject({ start: 4, end: 7 });
    // 5.4 snaps to 5: nearer the start.
    expect(extendOrRetract(sel, 5.4, toBeat)).toMatchObject({ start: 5, end: 8 });
    // 8.3 snaps onto the end: unchanged.
    expect(extendOrRetract(sel, 8.3, toBeat)).toMatchObject({ start: 4, end: 8 });
  });
  it("extends from a zero-length range (the insert marker) either way", () => {
    const marker = { start: 4, end: 4 };
    expect(extendOrRetract(marker, 6)).toEqual({ start: 4, end: 6 });
    expect(extendOrRetract(marker, 1)).toEqual({ start: 1, end: 4 });
  });
  it("never goes below zero", () => {
    expect(extendOrRetract(sel, -3)).toMatchObject({ start: 0, end: 8 });
  });
});
