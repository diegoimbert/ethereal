import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { t } from "@/i18n";
import { BAND_TEXT, DesktopBandFrame, resetDesktopBandDismissal } from "./index";

afterEach(() => {
  resetDesktopBandDismissal();
  vi.restoreAllMocks();
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

function show(desktop = true) {
  const openDeepLink = vi.fn();
  render(
    <DesktopBandFrame show={desktop} openDeepLink={openDeepLink}>
      <div data-testid="app" />
    </DesktopBandFrame>,
  );
  return { openDeepLink, open: () => fireEvent.click(screen.getByRole("button", { name: "open on desktop" })) };
}

describe("DesktopBand", () => {
  it("on a desktop machine (web): the band above the app", () => {
    show();
    const band = screen.getByTestId("desktop-band");
    expect(band).toHaveTextContent(t(BAND_TEXT));
    expect(screen.getByRole("button", { name: "open on desktop" })).toBeInTheDocument();
    // Above the app.
    expect(band.compareDocumentPosition(screen.getByTestId("app")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("not on phones and tablets: the app alone", () => {
    show(false);
    expect(screen.queryByTestId("desktop-band")).toBeNull();
    expect(screen.getByTestId("app")).toBeInTheDocument();
  });

  it("never in the desktop app", () => {
    (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
    show();
    expect(screen.queryByTestId("desktop-band")).toBeNull();
    expect(screen.getByTestId("app")).toBeInTheDocument();
  });

  it("Open on desktop follows ethereal://open", () => {
    const { openDeepLink, open } = show();
    open();
    expect(openDeepLink).toHaveBeenCalledWith("ethereal://open");
    expect(screen.getByTestId("desktop-band")).toHaveTextContent(t(BAND_TEXT));
  });

  it("close hides the band (the app stays mounted); a reload shows it again", () => {
    show();
    const app = screen.getByTestId("app");
    const close = screen.getByRole("button", { name: "close" });
    close.focus();
    expect(close).toHaveFocus();
    fireEvent.click(close);
    expect(screen.queryByTestId("desktop-band")).toBeNull();
    // Same app element: closing doesn't remount the editor.
    expect(screen.getByTestId("app")).toBe(app);
    // Still closed while the page lives (another mount in the same page load).
    cleanup();
    show();
    expect(screen.queryByTestId("desktop-band")).toBeNull();
    cleanup();
    // A refresh: module state starts over, nothing was stored.
    resetDesktopBandDismissal();
    show();
    expect(screen.getByTestId("desktop-band")).toHaveTextContent(t(BAND_TEXT));
    expect(localStorage.length + sessionStorage.length).toBe(0);
  });
});
