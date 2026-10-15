import { describe, expect, it } from "vitest";
import { additiveClick, planSolo } from "./soloPlan";

const items = (...s: boolean[]) => s.map((solo, i) => ({ id: `t${i}`, solo }));

describe("planSolo", () => {
  it("plain click on an unsoloed item with others soloed: only it soloed", () => {
    const p = planSolo(items(true, true, false), "t2", false);
    expect(p.exclusive).toBe(true);
    expect(p.changes).toEqual([
      { id: "t2", solo: true },
      { id: "t0", solo: false },
      { id: "t1", solo: false },
    ]);
  });
  it("plain click on a soloed item among several: only it stays soloed", () => {
    const p = planSolo(items(true, true, true), "t1", false);
    expect(p.exclusive).toBe(true);
    expect(p.changes).toEqual([
      { id: "t0", solo: false },
      { id: "t2", solo: false },
    ]);
  });
  it("plain click on an unsoloed item with nothing soloed solos it", () => {
    expect(planSolo(items(false, false), "t0", false)).toEqual({ changes: [{ id: "t0", solo: true }], exclusive: true });
  });
  it("plain click on the only soloed item unsolos it", () => {
    expect(planSolo(items(false, true), "t1", false)).toEqual({ changes: [{ id: "t1", solo: false }], exclusive: false });
  });
  it("additive click toggles only the clicked item", () => {
    expect(planSolo(items(true, false), "t1", true)).toEqual({ changes: [{ id: "t1", solo: true }], exclusive: false });
    expect(planSolo(items(true, true), "t1", true)).toEqual({ changes: [{ id: "t1", solo: false }], exclusive: false });
  });
  it("detects Ctrl and Cmd", () => {
    expect(additiveClick({ ctrlKey: true })).toBe(true);
    expect(additiveClick({ metaKey: true })).toBe(true);
    expect(additiveClick({})).toBe(false);
    expect(additiveClick()).toBe(false);
  });
});
