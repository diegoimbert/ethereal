import { act, fireEvent, render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useMarquee, type Rect } from "./marquee";
import { createItemSelectionStore } from "./selection";
import { createTimelineViewStore } from "./viewStore";

describe("useMarquee", () => {
  it("keeps its start in the content while the view scrolls under the pointer (auto-scroll)", () => {
    const store = createItemSelectionStore();
    const tested: Rect[] = [];
    let scrollTop = 0;
    function Grid() {
      const m = useMarquee({ kind: "clip", store, hitTest: (r) => (tested.push(r), []) });
      return <div data-testid="grid" onPointerDown={m.onPointerDown} />;
    }
    const { getByTestId } = render(<Grid />);
    const grid = getByTestId("grid");
    // The content element scrolls with its container: its top moves up by the scroll.
    grid.getBoundingClientRect = () =>
      ({
        left: 0,
        top: 100 - scrollTop,
        right: 500,
        bottom: 2100 - scrollTop,
        width: 500,
        height: 2000,
        x: 0,
        y: 100 - scrollTop,
        toJSON: () => ({}),
      }) as DOMRect;

    fireEvent.pointerDown(grid, { button: 0, clientX: 50, clientY: 120 });
    act(() => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 60, clientY: 300 }));
    });
    expect(tested.at(-1)).toEqual({ x0: 50, y0: 20, x1: 60, y1: 200 });

    // The view scrolls down 500 px, the pointer still: the box grows with the content.
    scrollTop = 500;
    act(() => {
      grid.dispatchEvent(new Event("scroll"));
    });
    expect(tested.at(-1)).toEqual({ x0: 50, y0: 20, x1: 60, y1: 700 });
    act(() => {
      window.dispatchEvent(new MouseEvent("pointerup", { clientX: 60, clientY: 300 }));
    });
  });

  it("on a timeline, the start stays on its beat while the view scrolls; the drawn box stays in the element", () => {
    const store = createItemSelectionStore();
    const view = createTimelineViewStore({ pxPerBeat: 10, scrollBeats: 20, widthPx: 400 });
    const tested: Rect[] = [];
    let shown: Rect | null = null;
    let cut = "";
    function Grid() {
      const m = useMarquee({ kind: "clip", store, hitTest: (r) => (tested.push(r), []), timeline: { view, originPx: () => 100 } });
      shown = m.rect;
      cut = m.cut;
      return <div data-testid="grid" onPointerDown={m.onPointerDown} />;
    }
    const { getByTestId } = render(<Grid />);
    const grid = getByTestId("grid");
    grid.getBoundingClientRect = () => ({ left: 0, top: 0, right: 500, bottom: 300, width: 500, height: 300, x: 0, y: 0, toJSON: () => ({}) }) as DOMRect;
    Object.defineProperty(grid, "clientWidth", { value: 500 });
    Object.defineProperty(grid, "clientHeight", { value: 300 });

    // Pressed at beat 20 + (300 - 100) / 10 = 40.
    fireEvent.pointerDown(grid, { button: 0, clientX: 300, clientY: 50 });
    act(() => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 450, clientY: 100 }));
    });
    expect(tested.at(-1)).toEqual({ x0: 300, y0: 50, x1: 450, y1: 100 });

    // The view scrolls 15 beats later: beat 40 is now at x 150.
    act(() => view.getState().scrollByPx(150));
    expect(tested.at(-1)).toEqual({ x0: 150, y0: 50, x1: 450, y1: 100 });

    // Past the element's right and bottom edges: the hit test gets the whole drag, the drawn
    // box stays inside the element (drawn past it, it would scroll it).
    act(() => {
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 700, clientY: 400 }));
    });
    expect(tested.at(-1)).toMatchObject({ x0: 150, y0: 50, y1: 400 });
    expect(tested.at(-1)!.x1).toBeGreaterThanOrEqual(700);
    expect(shown).toMatchObject({ x0: 150, y0: 50, x1: 500, y1: 300 });
    // Drawn without the cut sides' borders: the box goes on past the edges.
    expect(cut).toBe("right bottom");
    act(() => {
      window.dispatchEvent(new MouseEvent("pointerup", { clientX: 700, clientY: 400 }));
    });
  });
});
