import { describe, expect, it } from "vitest";
import { copyName, formatModified, sortProjects, uniqueName } from "./projectNames";

describe("project name helpers", () => {
  const projects = [
    { id: "a", name: "Demo", modified_ms: 10 },
    { id: "b", name: "demo 2", modified_ms: 30 },
    { id: "c", name: "Beat", modified_ms: 30 },
  ];

  it("sorts by save time, then name", () => {
    expect(sortProjects(projects).map((p) => p.id)).toEqual(["c", "b", "a"]);
  });

  it("makes names unique (case-insensitive)", () => {
    expect(uniqueName("Song", projects)).toBe("Song");
    expect(uniqueName(" Demo ", projects)).toBe("Demo 3");
    expect(uniqueName("", projects)).toBe("Untitled");
    expect(copyName("Beat", projects)).toBe("Beat copy");
    expect(copyName("Beat", [...projects, { name: "Beat copy" }])).toBe("Beat copy 2");
  });

  it("formats save times", () => {
    const now = new Date(2026, 0, 15, 12, 0).getTime();
    expect(formatModified(now - 5_000, now)).toBe("just now");
    expect(formatModified(now - 5 * 60_000, now)).toBe("5 min ago");
    expect(formatModified(now - 3 * 3_600_000, now)).not.toMatch(/ago/);
  });
});
