import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { resetI18nForTests, setLocale } from "@/i18n/runtime";
import { ToastStack } from "@/kit";
import { commandErrorText } from "@/transport";
import { NotificationLog, NotificationToastItems } from "./NotificationToasts";
import { ALERT_TOAST_MS, INFO_TOAST_MS, MAX_NOTICES, notify, onNotificationEvent, sentence, skipNotifications, toastLook, useNotices } from "./store";

afterEach(() => {
  act(() => useNotices.getState().clear());
  vi.useRealTimers();
  resetI18nForTests();
});

const renderToasts = () =>
  render(
    <ToastStack>
      <NotificationToastItems />
    </ToastStack>,
  );

describe("engine notifications and errors in the user's language (#667)", () => {
  it("shows a keyed notification and a keyed error in French, else the engine's English", async () => {
    await setLocale("fr", { persist: false });
    const output = { id: "notice.audio.output_named", args: { name: "Démo" } };
    onNotificationEvent({ type: "Notification", level: "Info", message: 'audio output "Démo"', key: output });
    onNotificationEvent({ type: "Notification", level: "Warning", message: "plain engine text" });
    // (Compared lowercased: the toast's sentence case is the lowercase sweep's.)
    const shown = () => useNotices.getState().notices.map((n) => n.message.toLowerCase());
    expect(shown()).toEqual(["la sortie audio «\u00a0démo\u00a0»", "plain engine text"]);
    const autosave = { id: "notice.project.autosave_failed", args: { error: { id: "error.project.none_open", args: {} } } };
    onNotificationEvent({ type: "Notification", level: "Error", message: "autosave failed: no project is open", key: autosave });
    expect(shown().at(-1)).toBe("l’enregistrement automatique a échoué\u00a0: aucun projet n’est ouvert");
    expect(commandErrorText({ code: "InvalidState", message: "nothing to undo", key: { id: "error.edit.nothing_to_undo", args: {} } })).toBe("rien à annuler");
    expect(commandErrorText({ code: "Internal", message: "" })).toBe("Internal");
  });
});

describe("engine notifications as toasts", () => {
  it("maps severity to the toast's title, colour and auto-dismiss", () => {
    expect(toastLook("Info")).toEqual({ title: "info", accent: "var(--eth-color-info)", timeoutMs: INFO_TOAST_MS });
    expect(toastLook("Warning")).toEqual({ title: "warning", accent: "var(--eth-color-warn)", timeoutMs: ALERT_TOAST_MS });
    expect(toastLook("Error")).toEqual({ title: "error", accent: "var(--eth-color-danger)", timeoutMs: ALERT_TOAST_MS });
  });

  it("turns Event::Notification into a notice (as a sentence) and ignores other events", () => {
    onNotificationEvent({ type: "Notification", level: "Info", message: "left the collaboration session (another project was opened)" });
    onNotificationEvent({ type: "Project", event: { type: "DirtyChanged", dirty: true } });
    expect(useNotices.getState().notices).toMatchObject([{ level: "Info", message: "left the collaboration session (another project was opened)" }]);
    expect(sentence("  ")).toBe("");
  });

  it("auto-dismisses info toasts, and warnings a little later", () => {
    vi.useFakeTimers();
    renderToasts();
    act(() => {
      notify("Info", 'your local version of this project was kept as "Song (local copy)"');
      notify("Warning", "export: a plugin could not render offline");
    });
    expect(screen.getByText('your local version of this project was kept as "Song (local copy)"')).toBeInTheDocument();
    expect(screen.getByText("info")).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(INFO_TOAST_MS + 200));
    expect(screen.queryByText(/local version/)).toBeNull();
    // The warning stays longer, then goes too.
    expect(screen.getByText("warning")).toBeInTheDocument();
    act(() => vi.advanceTimersByTime(ALERT_TOAST_MS - INFO_TOAST_MS));
    expect(screen.queryByText("warning")).toBeNull();
  });

  it("shows the time left as a bar that lasts as long as the toast", () => {
    renderToasts();
    act(() => notify("Error", "export failed"));
    expect(screen.getByTestId("toast-life").style.getPropertyValue("--eth-toast-life")).toBe(`${ALERT_TOAST_MS}ms`);
  });

  it("closes a toast early with its close button", () => {
    renderToasts();
    act(() => notify("Warning", "export: a plugin could not render offline"));
    fireEvent.click(screen.getByRole("button", { name: "dismiss" }));
    expect(screen.queryByText("warning")).toBeNull();
  });

  it("keeps the newest few", () => {
    act(() => {
      for (let i = 0; i < MAX_NOTICES + 2; i++) notify("Error", `e${i}`);
    });
    expect(useNotices.getState().notices.map((n) => n.message)).toEqual(["e2", "e3", "e4"]);
  });

  it("offers 'see all messages' when toasts left the stack unseen; the dialog lists them all, newest first", () => {
    render(
      <>
        <ToastStack>
          <NotificationToastItems />
        </ToastStack>
        <NotificationLog />
      </>,
    );
    act(() => notify("Info", "one"));
    expect(screen.queryByTestId("notices-see-all")).toBeNull();
    act(() => {
      for (let i = 0; i < MAX_NOTICES + 1; i++) notify("Warning", `w${i}`);
    });
    const more = screen.getByTestId("notices-see-all");
    expect(more.textContent).toBe("see all messages (5)");
    fireEvent.click(more);
    const rows = screen.getAllByTestId("notice-log-row").map((r) => r.textContent);
    expect(rows).toHaveLength(5);
    expect(rows[0]).toContain("w3");
    expect(rows.at(-1)).toContain("one");
    // The toasts are all in the dialog now.
    expect(useNotices.getState().notices).toEqual([]);
    expect(screen.queryByTestId("notices-see-all")).toBeNull();
  });

  it("an action button runs and dismisses its toast", () => {
    const resolve = vi.fn();
    renderToasts();
    act(() => notify("Warning", "some plugins are missing", { action: { label: "resolve", onSelect: resolve } }));
    fireEvent.click(screen.getByRole("button", { name: "resolve" }));
    expect(resolve).toHaveBeenCalledTimes(1);
    expect(useNotices.getState().notices).toEqual([]);
  });

  it("engine notifications a feature summarizes are skipped while it is registered", () => {
    const off = skipNotifications((e) => e.key?.id === "notice.engine.device_failed");
    const failed = { id: "notice.engine.device_failed", args: { device: "Serum", error: "not installed" } };
    onNotificationEvent({ type: "Notification", level: "Error", message: 'could not create device "Serum": not installed', key: failed });
    expect(useNotices.getState().notices).toEqual([]);
    off();
    onNotificationEvent({ type: "Notification", level: "Error", message: 'could not create device "Serum": not installed', key: failed });
    expect(useNotices.getState().notices).toHaveLength(1);
  });
});
