import { describe, expect, it } from "vitest";
import { stepWrapped } from "./libraryNav";

describe("stepWrapped", () => {
  it("steps one row within the list", () => {
    expect(stepWrapped(1, 4, 1)).toBe(2);
    expect(stepWrapped(2, 4, -1)).toBe(1);
  });
  it("wraps Up on the first row to the last, and Down on the last to the first", () => {
    expect(stepWrapped(0, 4, -1)).toBe(3);
    expect(stepWrapped(3, 4, 1)).toBe(0);
  });
  it("stays on the only row of a single-row list", () => {
    expect(stepWrapped(0, 1, 1)).toBe(0);
    expect(stepWrapped(0, 1, -1)).toBe(0);
  });
  it("starts at an end when no row is active, and gives -1 for an empty list", () => {
    expect(stepWrapped(-1, 4, 1)).toBe(0);
    expect(stepWrapped(-1, 4, -1)).toBe(3);
    expect(stepWrapped(-1, 0, 1)).toBe(-1);
    expect(stepWrapped(0, 0, -1)).toBe(-1);
  });
});
