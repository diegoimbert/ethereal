import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getMotionLevel, setMotionLevel } from "@/timeline/motion";
import { FloatingPane, type FloatingPaneProps } from "./FloatingPane";

const base: FloatingPaneProps = {
  side: "right",
  label: "Inspector",
  open: true,
  pinned: false,
  size: 300,
  minSize: 100,
  maxSize: () => 1000,
  onResize: () => {},
  onPinnedChange: () => {},
  onCollapsedChange: () => {},
  collapsed: false,
  children: <div data-testid="content" />,
};
const ui = (p: Partial<FloatingPaneProps>) => <FloatingPane {...base} {...p} />;
const card = () => document.querySelector<HTMLElement>(".eth-float");
const bar = () => document.querySelector<HTMLElement>(".eth-float-bar");
const wait = (ms: number) =>
  act(() => {
    vi.advanceTimersByTime(ms);
  });

describe("FloatingPane collapse animation", () => {
  const level = getMotionLevel();
  beforeEach(() => {
    vi.useFakeTimers();
    setMotionLevel("classic");
  });
  afterEach(() => {
    vi.useRealTimers();
    setMotionLevel(level);
  });

  it("collapsing flies the card out with its content mounted, then the bar fades in", () => {
    const { rerender } = render(ui({}));
    expect(bar()).toBeNull();
    rerender(ui({ collapsed: true }));
    expect(card()).toHaveClass("eth-float--closing");
    expect(screen.getByTestId("content")).toBeInTheDocument();
    expect(bar()).toBeNull();
    wait(200);
    expect(card()).toBeNull();
    expect(bar()).not.toBeNull();
    expect(bar()).not.toHaveClass("eth-float-bar--closing");
  });

  it("never unmounts before the exit animation has had its time", () => {
    const { rerender } = render(ui({}));
    rerender(ui({ collapsed: true }));
    wait(50);
    expect(card()).toHaveClass("eth-float--closing");
    wait(300);
    expect(card()).toBeNull();
    expect(bar()).not.toBeNull();
  });

  it("expanding fades the bar out while the card flies in", () => {
    const { rerender } = render(ui({ collapsed: true }));
    expect(bar()).not.toBeNull();
    expect(card()).toBeNull();
    rerender(ui({ collapsed: false }));
    expect(card()).not.toBeNull();
    expect(card()).not.toHaveClass("eth-float--closing");
    expect(screen.getByTestId("content")).toBeInTheDocument();
    expect(bar()).toHaveClass("eth-float-bar--closing");
    wait(200);
    expect(bar()).toBeNull();
  });

  it("reduced motion is instant both ways", () => {
    setMotionLevel("reduced");
    const { rerender } = render(ui({}));
    rerender(ui({ collapsed: true }));
    expect(card()).toBeNull();
    expect(bar()).not.toBeNull();
    rerender(ui({ collapsed: false }));
    expect(bar()).toBeNull();
    expect(card()).not.toBeNull();
  });

  it("rapid toggling ends in the consistent state", () => {
    const { rerender } = render(ui({}));
    rerender(ui({ collapsed: true }));
    rerender(ui({ collapsed: false })); // back before the exit ended
    expect(card()).not.toHaveClass("eth-float--closing");
    expect(bar()).toBeNull();
    rerender(ui({ collapsed: true }));
    wait(200);
    rerender(ui({ collapsed: false })); // the bar had barely appeared
    rerender(ui({ collapsed: true }));
    wait(500);
    expect(card()).toBeNull();
    expect(bar()).not.toBeNull();
    expect(bar()).not.toHaveClass("eth-float-bar--closing");
    rerender(ui({ open: false }));
    wait(500);
    expect(bar()).toBeNull();
    expect(card()).toBeNull();
  });
});
