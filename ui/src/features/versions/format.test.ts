import { describe, expect, it } from "vitest";
import { countsLabel, formatSize, formatWhen, isEmptyDiff, sortTables, tableLabel, versionTitle } from "./format";

describe("versions format", () => {
  it("titles versions by name, else by kind", () => {
    expect(versionTitle({ name: "Mix A", kind: "Manual" })).toBe("Mix A");
    expect(versionTitle({ name: null, kind: "Autosave" })).toBe("Autosave");
    expect(versionTitle({ name: null, kind: "BeforeRestore" })).toBe("Before restore");
  });

  it("formats times relative to now", () => {
    const now = new Date(2026, 9, 3, 15, 0).getTime();
    expect(formatWhen(now - 10_000, now)).toBe("just now");
    expect(formatWhen(now - 12 * 60_000, now)).toBe("12 min ago");
    expect(formatWhen(now - 3 * 3_600_000, now)).not.toMatch(/ago|Yesterday/);
    expect(formatWhen(now - 20 * 3_600_000, now)).toMatch(/^Yesterday /);
  });

  it("formats sizes, tables and counts", () => {
    expect(formatSize(512)).toBe("512 B");
    expect(formatSize(2048)).toBe("2 KB");
    expect(formatSize(3 * 1024 * 1024)).toBe("3.0 MB");
    expect(tableLabel("automation_points")).toBe("Automation points");
    expect(countsLabel({ added: 2, removed: 0, changed: 1 })).toBe("+2 ~1");
  });

  it("orders tables people think in first", () => {
    const row = (table: string) => ({ table, added: 1, removed: 0, changed: 0, names: [] });
    const diff = { tables: [row("markers"), row("clips"), row("zzz"), row("tracks")], settings_changed: false };
    expect(sortTables(diff).map((t) => t.table)).toEqual(["tracks", "clips", "markers", "zzz"]);
    expect(isEmptyDiff({ tables: [], settings_changed: false })).toBe(true);
    expect(isEmptyDiff({ tables: [], settings_changed: true })).toBe(false);
  });
});
