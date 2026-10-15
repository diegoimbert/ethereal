import { beforeEach, describe, expect, it } from "vitest";
import { chatStorageKey, clearChat, loadChat, MAX_SAVED_CHARS, saveChat, type SavedItem } from "./persist";

const item = (id: number, text = "hi"): SavedItem => ({ kind: "user", id: `ai-${id}`, text });

beforeEach(() => localStorage.clear());

describe("saved AI conversations (#717)", () => {
  it("round-trips items, provider and history per project", () => {
    saveChat("p1", { items: [item(1)], provider: "anthropic", history: [{ role: "user", content: "hi" }] });
    expect(loadChat("p1")).toEqual({ items: [item(1)], provider: "anthropic", history: [{ role: "user", content: "hi" }] });
    expect(loadChat("p2")).toBeNull();
  });

  it("an empty conversation removes the saved one; clearChat forgets it", () => {
    saveChat("p1", { items: [item(1)], provider: null, history: null });
    saveChat("p1", { items: [], provider: null, history: null });
    expect(localStorage.getItem(chatStorageKey("p1"))).toBeNull();
    saveChat("p1", { items: [item(1)], provider: null, history: null });
    clearChat("p1");
    expect(loadChat("p1")).toBeNull();
  });

  it("ignores corrupt, old-version and malformed data", () => {
    localStorage.setItem(chatStorageKey("p1"), "{nope");
    expect(loadChat("p1")).toBeNull();
    localStorage.setItem(chatStorageKey("p1"), JSON.stringify({ version: 0, items: [item(1)] }));
    expect(loadChat("p1")).toBeNull();
    localStorage.setItem(chatStorageKey("p1"), JSON.stringify({ version: 1, items: [item(1), 3, { kind: "x" }], provider: "nope", history: [1] }));
    expect(loadChat("p1")).toEqual({ items: [item(1)], provider: null, history: null });
  });

  it("too big: drops the model-side history first, then the oldest messages", () => {
    const big = "x".repeat(MAX_SAVED_CHARS);
    saveChat("p1", { items: [item(1)], provider: "anthropic", history: [big] });
    expect(loadChat("p1")).toEqual({ items: [item(1)], provider: null, history: null });

    const items = Array.from({ length: 40 }, (_, i) => item(i + 1, "y".repeat(MAX_SAVED_CHARS / 20)));
    saveChat("p1", { items, provider: "anthropic", history: [] });
    const kept = loadChat("p1")!;
    expect(kept.history).toBeNull();
    expect(kept.items.length).toBeGreaterThan(0);
    expect(kept.items.length).toBeLessThan(20);
    expect(kept.items.at(-1)!.id).toBe("ai-40");
    expect(localStorage.getItem(chatStorageKey("p1"))!.length).toBeLessThanOrEqual(MAX_SAVED_CHARS);
  });
});
