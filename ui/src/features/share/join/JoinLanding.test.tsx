import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { JoinLanding } from "./JoinLanding";
import { APP_OPEN_TIMEOUT_MS, prefersBrowser, type JoinRoute } from "./landing";

const ROUTE: JoinRoute = {
  link: "https://etherealws.pages.dev/join/AbCdEfGhIjKlMnOpQrStUv#10123456789_-abcdefghij",
  deepLink: "ethereal://join/AbCdEfGhIjKlMnOpQrStUv#10123456789_-abcdefghij",
  problem: null,
};

afterEach(() => {
  vi.useRealTimers();
  localStorage.clear();
});

function show(route = ROUTE) {
  const onContinue = vi.fn();
  const onOpenApp = vi.fn();
  const openDeepLink = vi.fn();
  render(<JoinLanding route={route} onContinue={onContinue} onOpenApp={onOpenApp} openDeepLink={openDeepLink} />);
  return { onContinue, onOpenApp, openDeepLink };
}

describe("JoinLanding", () => {
  it("says you're invited, without naming anyone (the host is not known yet)", () => {
    show();
    expect(screen.getByRole("heading")).toHaveTextContent("You've been invited to an Ethereal project");
  });

  it("Open in the app: follows the deep link, then offers the download if the page stays", () => {
    vi.useFakeTimers();
    const { openDeepLink } = show();
    fireEvent.click(screen.getByRole("button", { name: "Open in the app" }));
    expect(openDeepLink).toHaveBeenCalledWith(ROUTE.deepLink);
    expect(screen.queryByText(/Don't have the app/)).toBeNull();
    act(() => vi.advanceTimersByTime(APP_OPEN_TIMEOUT_MS + 10));
    expect(screen.getByRole("status")).toHaveTextContent("Don't have the app? Download it");
    expect(screen.getByRole("link", { name: "Download it" })).toHaveAttribute("href", expect.stringContaining("/releases/latest"));
  });

  it("Continue in browser boots the app, remembering the choice when asked", () => {
    const { onContinue } = show();
    fireEvent.click(screen.getByRole("switch", { name: "Always continue in browser" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue in browser" }));
    expect(onContinue).toHaveBeenCalled();
    expect(prefersBrowser()).toBe(true);
  });

  it("Continue without remembering leaves the preference off", () => {
    const { onContinue } = show();
    fireEvent.click(screen.getByRole("button", { name: "Continue in browser" }));
    expect(onContinue).toHaveBeenCalled();
    expect(prefersBrowser()).toBe(false);
  });

  it("a damaged link says so and offers to open Ethereal", () => {
    const { onOpenApp } = show({ ...ROUTE, problem: "The invite link is damaged." });
    expect(screen.getByRole("alert")).toHaveTextContent("The invite link is damaged.");
    expect(screen.queryByRole("button", { name: "Continue in browser" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Open Ethereal" }));
    expect(onOpenApp).toHaveBeenCalled();
  });
});
