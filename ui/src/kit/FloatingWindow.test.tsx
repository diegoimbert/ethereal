import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import { FloatingWindow, type FloatingWindowProps } from "./FloatingWindow";
import { clampRect, resizeRect } from "./floatingWindowGeometry";
import { size } from "../theme/tokens";
import { resetFloatingWindows, useFloatingWindows } from "./floatingWindowStore";

// jsdom's viewport: 1024 × 768.
const VW = 1024;
const VH = 768;
const LIMITS = { minWidth: 240, minHeight: 120, maxWidth: Infinity, maxHeight: Infinity };

function Win(props: Partial<FloatingWindowProps> & { id?: string }) {
  const [open, setOpen] = useState(true);
  if (!open) return null;
  return (
    <FloatingWindow
      id="t"
      label="Test window"
      title="Test"
      defaultWidth={400}
      defaultHeight={300}
      defaultPosition={() => ({ x: 100, y: 100 })}
      onClose={() => setOpen(false)}
      {...props}
    >
      <p>Content</p>
    </FloatingWindow>
  );
}

const win = (name = "Test window") => screen.getByRole("dialog", { name });
const box = (el: HTMLElement) => ({
  x: parseFloat(el.style.left),
  y: parseFloat(el.style.top),
  w: parseFloat(el.style.width),
  h: parseFloat(el.style.height),
});

function drag(target: Element, from: [number, number], to: [number, number]) {
  fireEvent.pointerDown(target, { button: 0, clientX: from[0], clientY: from[1], pointerId: 1 });
  fireEvent.pointerMove(target, { clientX: to[0], clientY: to[1], pointerId: 1 });
  fireEvent.pointerUp(target, { clientX: to[0], clientY: to[1], pointerId: 1 });
}
const handle = (edge: string, el = win()) => el.querySelector(`[data-resize="${edge}"]`)!;

beforeEach(() => {
  resetFloatingWindows();
  window.innerWidth = VW;
  window.innerHeight = VH;
});
afterEach(() => resetFloatingWindows());

describe("FloatingWindow", () => {
  it("opens at its default place and size, and remembers it", () => {
    render(<Win />);
    expect(box(win())).toEqual({ x: 100, y: 100, w: 400, h: 300 });
    expect(useFloatingWindows.getState().rects.t).toEqual({ x: 100, y: 100 });
    expect(win()).toHaveFocus();
  });

  it("moves by its title bar (not by its buttons)", () => {
    render(<Win />);
    const header = win().querySelector("header")!;
    drag(header, [150, 110], [200, 150]);
    expect(box(win())).toMatchObject({ x: 150, y: 140 });
    expect(useFloatingWindows.getState().rects.t).toEqual({ x: 150, y: 140 });
    drag(screen.getByRole("button", { name: "close" }), [0, 0], [300, 300]);
    expect(box(win())).toMatchObject({ x: 150, y: 140 });
    fireEvent.click(screen.getByRole("button", { name: "close" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("resizes from an edge, within its minimum size", () => {
    render(<Win />);
    drag(handle("e"), [500, 200], [550, 200]);
    expect(box(win())).toEqual({ x: 100, y: 100, w: 450, h: 300 });
    // The left edge, far right: the minimum width, the right edge stays put.
    drag(handle("w"), [100, 200], [1000, 200]);
    expect(box(win())).toEqual({ x: 310, y: 100, w: 240, h: 300 });
    expect(useFloatingWindows.getState().rects.t).toEqual({ x: 310, y: 100, w: 240, h: 300 });
  });

  it("resizes from a corner, within its minimum size", () => {
    render(<Win />);
    drag(handle("nw"), [100, 100], [50, 70]);
    expect(box(win())).toEqual({ x: 50, y: 70, w: 450, h: 330 });
    drag(handle("se"), [500, 400], [-1000, -1000]);
    expect(box(win())).toEqual({ x: 50, y: 70, w: 240, h: 120 });
  });

  it("may go past the left, right and bottom edges, part of its title bar on screen (never above the top)", () => {
    const grip = parseFloat(size.floatingWindowGrip);
    const bar = parseFloat(size.panelHeaderHeight);
    render(<Win />);
    drag(win().querySelector("header")!, [150, 110], [5000, 5000]);
    expect(box(win())).toMatchObject({ x: VW - grip, y: VH - bar });
    drag(win().querySelector("header")!, [VW - grip + 10, VH - bar + 5], [-5000, -5000]);
    expect(box(win())).toMatchObject({ x: grip - 400, y: 0 });
    act(() => {
      window.innerWidth = 800;
      window.innerHeight = 600;
      window.dispatchEvent(new Event("resize"));
    });
    act(() => useFloatingWindows.getState().setRect("t", { x: 5000, y: 5000 }));
    expect(box(win())).toMatchObject({ x: 800 - grip, y: 600 - bar });
    // A window bigger than the viewport is shrunk to it.
    act(() => useFloatingWindows.getState().setRect("t", { x: 0, y: 0, w: 2000, h: 2000 }));
    expect(box(win())).toEqual({ x: 0, y: 0, w: 800, h: 600 });
    // Resizing may go past the right and bottom edges (up to the viewport's size).
    act(() => useFloatingWindows.getState().setRect("t", { x: 100, y: 100, w: 400, h: 300 }));
    drag(handle("se"), [500, 400], [5000, 5000]);
    expect(box(win())).toEqual({ x: 100, y: 100, w: 800, h: 600 });
  });

  it("comes back where it was, with its size (persisted per id)", () => {
    const { unmount } = render(<Win />);
    drag(win().querySelector("header")!, [150, 110], [170, 130]);
    drag(handle("se"), [520, 420], [560, 460]);
    unmount();
    expect(JSON.parse(localStorage.getItem("eth.windows")!)).toEqual({ t: { x: 120, y: 120, w: 440, h: 340 } });
    render(<Win />);
    expect(box(win())).toEqual({ x: 120, y: 120, w: 440, h: 340 });
  });

  it("double-clicking the title resets the size (not the place)", () => {
    render(<Win />);
    drag(handle("se"), [500, 400], [600, 450]);
    expect(box(win())).toMatchObject({ w: 500, h: 350 });
    fireEvent.doubleClick(win().querySelector(".eth-fwin__title")!);
    expect(box(win())).toEqual({ x: 100, y: 100, w: 400, h: 300 });
  });

  it("rises above the other windows when clicked", () => {
    const onRaise = vi.fn();
    render(
      <>
        <Win id="a" label="A" onRaise={onRaise} />
        <Win id="b" label="B" />
      </>,
    );
    const z = (name: string) => Number(win(name).style.getPropertyValue("--fwin-stack"));
    expect(z("B")).toBeGreaterThan(z("A"));
    expect(win("B")).toHaveClass("eth-fwin--top");
    fireEvent.pointerDown(win("A").querySelector("p")!);
    expect(z("A")).toBeGreaterThan(z("B"));
    expect(win("A")).toHaveClass("eth-fwin--top");
    expect(onRaise).toHaveBeenCalledTimes(1);
  });

  it("keyboard: Alt+arrows move, Alt+Shift+arrows resize, Escape closes", () => {
    render(<Win />);
    const w = win();
    fireEvent.keyDown(w, { key: "ArrowRight", altKey: true });
    fireEvent.keyDown(w, { key: "ArrowDown", altKey: true });
    expect(box(w)).toMatchObject({ x: 122, y: 122 });
    fireEvent.keyDown(w, { key: "ArrowLeft", altKey: true, shiftKey: true });
    expect(box(w)).toMatchObject({ w: 378, h: 300 });
    // Not from a control inside (it handles its own keys).
    fireEvent.keyDown(screen.getByRole("button", { name: "close" }), { key: "Escape" });
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    fireEvent.keyDown(w, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("extra header actions and data attributes", () => {
    render(<Win actions={<button type="button">Pin</button>} data-thing="x" />);
    expect(win().querySelector("header")).toContainElement(screen.getByRole("button", { name: "Pin" }));
    expect(win()).toHaveAttribute("data-thing", "x");
    expect(win()).toHaveAttribute("data-window", "t");
  });

  it("a window sized by its content has no height until resized", () => {
    render(<Win defaultHeight={undefined} />);
    expect(win().style.height).toBe("");
    expect(win()).not.toHaveClass("eth-fwin--sized");
  });

  it("ignores corrupt storage", async () => {
    localStorage.setItem("eth.windows", "{not json");
    vi.resetModules();
    const fresh = await import("./floatingWindowStore");
    expect(fresh.useFloatingWindows.getState().rects).toEqual({});
    localStorage.setItem("eth.windows", JSON.stringify({ a: { x: 1, y: 2, w: -5, h: 40 }, b: { x: "no" } }));
    vi.resetModules();
    const again = await import("./floatingWindowStore");
    expect(again.useFloatingWindows.getState().rects).toEqual({ a: { x: 1, y: 2, h: 40 } });
  });
});

describe("floating window geometry", () => {
  const vp = { width: VW, height: VH };
  it("clampRect keeps the window inside and its size within the limits", () => {
    expect(clampRect({ x: -50, y: -10 }, vp, LIMITS, 400, 100)).toEqual({ x: 0, y: 0 });
    expect(clampRect({ x: 900, y: 700 }, vp, LIMITS, 400, 100)).toEqual({ x: VW - 400, y: VH - 100 });
    expect(clampRect({ x: 0, y: 0, w: 10, h: 10 }, vp, LIMITS, 400, 100)).toEqual({ x: 0, y: 0, w: 240, h: 120 });
  });
  it("clampRect with a grip: past the left / right / bottom edges, a part of the title bar kept", () => {
    const grip = { x: 100, y: 20 };
    expect(clampRect({ x: -1000, y: -50 }, vp, LIMITS, 400, 100, grip)).toEqual({ x: 100 - 400, y: 0 });
    expect(clampRect({ x: 5000, y: 5000 }, vp, LIMITS, 400, 100, grip)).toEqual({ x: VW - 100, y: VH - 20 });
    expect(clampRect({ x: -50, y: 30 }, vp, LIMITS, 400, 100, grip)).toEqual({ x: -50, y: 30 });
  });
  it("resizeRect keeps the opposite edge put", () => {
    const start = { x: 100, y: 100, w: 400, h: 300 };
    expect(resizeRect(start, "n", 0, 50, vp, LIMITS)).toEqual({ x: 100, y: 150, w: 400, h: 250 });
    expect(resizeRect(start, "n", 0, -500, vp, LIMITS)).toEqual({ x: 100, y: 0, w: 400, h: 400 });
    expect(resizeRect(start, "sw", -20, 20, vp, LIMITS)).toEqual({ x: 80, y: 100, w: 420, h: 320 });
    expect(resizeRect(start, "ne", 1000, 1000, vp, LIMITS)).toEqual({ x: 100, y: 280, w: VW - 100, h: 120 });
  });
});
