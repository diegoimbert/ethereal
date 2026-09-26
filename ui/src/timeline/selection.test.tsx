import { act, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { createDemoProject } from "@/transport";
import { useProjectStore } from "@/state/projectStore";
import { marqueeHits, rectFromPoints, rectsIntersect, useMarquee, type Rect } from "./marquee";
import {
  bindSelectionToProject,
  combineSelection,
  createItemSelectionStore,
  selectModeFromEvent,
  type ItemSelectionStore,
} from "./selection";

afterEach(() => useProjectStore.getState().reset());

describe("combineSelection", () => {
  const cur = new Set(["a", "b"]);
  it("replace / add / remove / toggle", () => {
    expect([...combineSelection(cur, ["c"], "replace")]).toEqual(["c"]);
    expect([...combineSelection(cur, ["c"], "add")].sort()).toEqual(["a", "b", "c"]);
    expect([...combineSelection(cur, ["a"], "remove")]).toEqual(["b"]);
    expect([...combineSelection(cur, ["a", "c"], "toggle")].sort()).toEqual(["b", "c"]);
  });
  it("returns the same set when nothing changes", () => {
    expect(combineSelection(cur, ["a"], "add")).toBe(cur);
    expect(combineSelection(cur, ["b", "a"], "replace")).toBe(cur);
  });
  it("modes from modifiers", () => {
    expect(selectModeFromEvent({ shiftKey: false, metaKey: false, ctrlKey: false })).toBe("replace");
    expect(selectModeFromEvent({ shiftKey: true, metaKey: false, ctrlKey: false })).toBe("add");
    expect(selectModeFromEvent({ shiftKey: false, metaKey: true, ctrlKey: false })).toBe("toggle");
  });
});

describe("item selection store", () => {
  it("keeps kinds independent and tracks the anchor", () => {
    const s = createItemSelectionStore();
    s.getState().select("clip", ["c1", "c2"]);
    s.getState().select("note", ["n1"]);
    expect([...s.getState().selected.clip]).toEqual(["c1", "c2"]);
    expect(s.getState().anchor.clip).toBe("c2");
    s.getState().select("note", ["n2"], "add");
    expect(s.getState().isSelected("note", "n1")).toBe(true);
    expect(s.getState().isSelected("clip", "c1")).toBe(true);
    s.getState().clear("note");
    expect(s.getState().selected.note.size).toBe(0);
    expect(s.getState().selected.clip.size).toBe(2);
    s.getState().setTimeRange({ start: 0, end: 4 });
    s.getState().clear();
    expect(s.getState().selected.clip.size).toBe(0);
    expect(s.getState().timeRange).toBeNull();
  });

  it("does not notify when nothing changes", () => {
    const s = createItemSelectionStore();
    s.getState().select("clip", ["c1"]);
    let calls = 0;
    const unsub = s.subscribe(() => calls++);
    s.getState().select("clip", ["c1"], "add");
    s.getState().clear("note");
    unsub();
    expect(calls).toBe(0);
  });

  it("prunes ids removed from the project mirror", () => {
    const project = createDemoProject();
    const clipIds = Object.keys(project.clips);
    const noteIds = Object.keys(project.notes);
    expect(clipIds.length).toBeGreaterThan(1);
    const s = createItemSelectionStore();
    const unbind = bindSelectionToProject(s);
    useProjectStore.getState().loadProject(project);
    s.getState().select("clip", clipIds.slice(0, 2));
    s.getState().select("note", noteIds.slice(0, 1));

    const gone = clipIds[0]!;
    useProjectStore.getState().applyPatch({
      revision: 1,
      changes: [{ type: "Remove", key: { type: "Clip", id: gone } }],
      history: useProjectStore.getState().history,
    });
    expect(s.getState().isSelected("clip", gone)).toBe(false);
    expect(s.getState().isSelected("clip", clipIds[1]!)).toBe(true);
    expect(s.getState().selected.note.size).toBe(1);

    useProjectStore.getState().reset();
    expect(s.getState().selected.clip.size).toBe(0);
    unbind();
  });
});

describe("marquee geometry", () => {
  it("normalizes and intersects rectangles", () => {
    const r = rectFromPoints({ x: 10, y: 20 }, { x: 0, y: 5 });
    expect(r).toEqual({ x0: 0, y0: 5, x1: 10, y1: 20 });
    expect(rectsIntersect(r, { x0: 10, y0: 20, x1: 30, y1: 30 })).toBe(true);
    expect(rectsIntersect(r, { x0: 11, y0: 0, x1: 30, y1: 30 })).toBe(false);
    const boxes = [
      { id: "a", rect: { x0: 0, y0: 0, x1: 5, y1: 5 } },
      { id: "b", rect: { x0: 50, y0: 0, x1: 60, y1: 5 } },
    ];
    expect(marqueeHits(r, boxes)).toEqual(["a"]);
  });
});

describe("useMarquee", () => {
  const boxes = [
    { id: "n1", rect: { x0: 0, y0: 0, x1: 10, y1: 10 } },
    { id: "n2", rect: { x0: 40, y0: 0, x1: 50, y1: 10 } },
    { id: "n3", rect: { x0: 80, y0: 0, x1: 90, y1: 10 } },
  ];

  function Surface({ store, onRect }: { store: ItemSelectionStore; onRect?: (r: Rect | null) => void }) {
    const m = useMarquee({ kind: "note", store, hitTest: (r) => marqueeHits(r, boxes) });
    onRect?.(m.rect);
    return <div data-testid="surface" onPointerDown={m.onPointerDown} />;
  }

  // jsdom has no PointerEvent constructor: dispatch MouseEvents with pointer event names.
  const pointer = (type: string, x: number, y: number, init: MouseEventInit = {}) =>
    window.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y, bubbles: true, ...init }));

  it("drag selects the hits; shift-drag adds; click clears", () => {
    const store = createItemSelectionStore();
    let rect: Rect | null = null;
    const { getByTestId } = render(<Surface store={store} onRect={(r) => (rect = r)} />);
    const el = getByTestId("surface");

    fireEvent.pointerDown(el, { button: 0, clientX: 5, clientY: 5 });
    act(() => pointer("pointermove", 45, 8));
    expect(rect).toEqual({ x0: 5, y0: 5, x1: 45, y1: 8 });
    expect([...store.getState().selected.note].sort()).toEqual(["n1", "n2"]);
    act(() => pointer("pointerup", 45, 8));
    expect(rect).toBeNull();

    fireEvent.pointerDown(el, { button: 0, clientX: 85, clientY: 5, shiftKey: true });
    act(() => pointer("pointermove", 88, 9));
    act(() => pointer("pointerup", 88, 9));
    expect([...store.getState().selected.note].sort()).toEqual(["n1", "n2", "n3"]);

    fireEvent.pointerDown(el, { button: 0, clientX: 200, clientY: 5 });
    act(() => pointer("pointerup", 200, 5));
    expect(store.getState().selected.note.size).toBe(0);
  });
});
