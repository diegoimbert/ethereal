import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ToastStack } from "@/kit";
import { NotificationToastItems } from "./NotificationToasts";
import { INFO_TOAST_MS, MAX_NOTICES, notify, onNotificationEvent, sentence, toastLook, useNotices } from "./store";

afterEach(() => {
  act(() => useNotices.getState().clear());
  vi.useRealTimers();
});

const renderToasts = () =>
  render(
    <ToastStack>
      <NotificationToastItems />
    </ToastStack>,
  );

describe("engine notifications as toasts", () => {
  it("maps severity to the toast's title, colour and auto-dismiss", () => {
    expect(toastLook("Info")).toEqual({ title: "Info", accent: "var(--eth-color-info)", timeoutMs: INFO_TOAST_MS });
    expect(toastLook("Warning")).toEqual({ title: "Warning", accent: "var(--eth-color-warn)", timeoutMs: undefined });
    expect(toastLook("Error")).toEqual({ title: "Error", accent: "var(--eth-color-danger)", timeoutMs: undefined });
  });

  it("turns Event::Notification into a notice (as a sentence) and ignores other events", () => {
    onNotificationEvent({ type: "Notification", level: "Info", message: "left the collaboration session (another project was opened)" });
    onNotificationEvent({ type: "Project", event: { type: "DirtyChanged", dirty: true } });
    expect(useNotices.getState().notices).toMatchObject([{ level: "Info", message: "Left the collaboration session (another project was opened)" }]);
    expect(sentence("  ")).toBe("");
  });

  it("auto-dismisses info toasts and keeps warnings until dismissed", () => {
    vi.useFakeTimers();
    renderToasts();
    act(() => {
      notify("Info", 'your local version of this project was kept as "Song (local copy)"');
      notify("Warning", "export: a plugin could not render offline");
    });
    expect(screen.getByText('Your local version of this project was kept as "Song (local copy)"')).toBeInTheDocument();
    expect(screen.getByText("Info")).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(INFO_TOAST_MS + 200));
    expect(screen.queryByText(/local version/)).toBeNull();
    // The warning stays until its close button.
    act(() => vi.advanceTimersByTime(60_000));
    expect(screen.getByText("Warning")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByText("Warning")).toBeNull();
  });

  it("keeps the newest few", () => {
    act(() => {
      for (let i = 0; i < MAX_NOTICES + 2; i++) notify("Error", `e${i}`);
    });
    expect(useNotices.getState().notices.map((n) => n.message)).toEqual(["E2", "E3", "E4"]);
  });
});
