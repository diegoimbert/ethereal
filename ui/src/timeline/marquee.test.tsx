import { act, fireEvent, render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useMarquee, type Rect } from "./marquee";
import { createItemSelectionStore } from "./selection";

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
});
