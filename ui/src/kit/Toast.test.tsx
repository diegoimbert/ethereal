import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Toast } from "./Toast";

// jsdom lays nothing out: make every toast text read as clamped (more lines than shown).
function clampAll() {
  const sh = vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(120);
  const ch = vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(60);
  return () => {
    sh.mockRestore();
    ch.mockRestore();
  };
}

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("toast", () => {
  it("shows no expand control when the text fits", () => {
    render(<Toast title="info">short</Toast>);
    expect(screen.queryByTestId("toast-expand")).toBeNull();
  });

  it("expands a clamped text on click, copies it, and then stays until dismissed", async () => {
    clampAll();
    vi.useFakeTimers();
    const onDismiss = vi.fn();
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    render(
      <Toast title="error" timeoutMs={1000} onDismiss={onDismiss}>
        a very long message
      </Toast>,
    );
    expect(screen.queryByTestId("toast-copy")).toBeNull();
    fireEvent.click(screen.getByText("a very long message"));
    expect(screen.getByRole("status")).toHaveAttribute("data-expanded", "true");
    await act(async () => {
      fireEvent.click(screen.getByTestId("toast-copy"));
      await Promise.resolve();
    });
    expect(writeText).toHaveBeenCalledWith("a very long message");
    act(() => vi.advanceTimersByTime(5000));
    expect(onDismiss).not.toHaveBeenCalled();
    // Collapsing gives the timer back.
    fireEvent.click(screen.getByTestId("toast-expand"));
    act(() => vi.advanceTimersByTime(1500));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("pauses the auto-dismiss while hovered", () => {
    vi.useFakeTimers();
    const onDismiss = vi.fn();
    render(
      <Toast title="info" timeoutMs={1000} onDismiss={onDismiss}>
        text
      </Toast>,
    );
    const toast = screen.getByRole("status");
    fireEvent.pointerEnter(toast);
    act(() => vi.advanceTimersByTime(5000));
    expect(onDismiss).not.toHaveBeenCalled();
    fireEvent.pointerLeave(toast);
    act(() => vi.advanceTimersByTime(1500));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it("keeps a click on a clickable toast for its own action", () => {
    clampAll();
    const onClick = vi.fn();
    render(
      <Toast title="info" onClick={onClick}>
        text
      </Toast>,
    );
    fireEvent.click(screen.getByText("text"));
    expect(onClick).toHaveBeenCalled();
    expect(screen.getByRole("status")).not.toHaveAttribute("data-expanded");
    fireEvent.click(screen.getByTestId("toast-expand"));
    expect(screen.getByRole("status")).toHaveAttribute("data-expanded", "true");
  });
});
