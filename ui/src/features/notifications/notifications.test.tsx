import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { resetI18nForTests, setLocale } from "@/i18n/runtime";
import { ToastStack } from "@/kit";
import { commandErrorText } from "@/transport";
import { NotificationToastItems } from "./NotificationToasts";
import { INFO_TOAST_MS, MAX_NOTICES, notify, onNotificationEvent, sentence, toastLook, useNotices } from "./store";

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
    // (Compared lowercased: the toast's sentence case is the lowercase sweep's, #524.)
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
    expect(toastLook("Warning")).toEqual({ title: "warning", accent: "var(--eth-color-warn)", timeoutMs: undefined });
    expect(toastLook("Error")).toEqual({ title: "error", accent: "var(--eth-color-danger)", timeoutMs: undefined });
  });

  it("turns Event::Notification into a notice (as a sentence) and ignores other events", () => {
    onNotificationEvent({ type: "Notification", level: "Info", message: "left the collaboration session (another project was opened)" });
    onNotificationEvent({ type: "Project", event: { type: "DirtyChanged", dirty: true } });
    expect(useNotices.getState().notices).toMatchObject([{ level: "Info", message: "left the collaboration session (another project was opened)" }]);
    expect(sentence("  ")).toBe("");
  });

  it("auto-dismisses info toasts and keeps warnings until dismissed", () => {
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
    // The warning stays until its close button.
    act(() => vi.advanceTimersByTime(60_000));
    expect(screen.getByText("warning")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "dismiss" }));
    expect(screen.queryByText("warning")).toBeNull();
  });

  it("keeps the newest few", () => {
    act(() => {
      for (let i = 0; i < MAX_NOTICES + 2; i++) notify("Error", `e${i}`);
    });
    expect(useNotices.getState().notices.map((n) => n.message)).toEqual(["e2", "e3", "e4"]);
  });
});
