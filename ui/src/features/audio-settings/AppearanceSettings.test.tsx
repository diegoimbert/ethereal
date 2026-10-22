import { afterEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { setThemeId, setZoom } from "@/theme";
import { AppearanceSettings } from "./AppearanceSettings";

afterEach(() => {
  setZoom(100);
  setThemeId("dark");
});

describe("Settings > Appearance", () => {
  it("has the theme, the interface zoom (scales the page, remembered) and translucency", () => {
    render(<AppearanceSettings />);
    expect(screen.getByRole("group", { name: "theme" })).toBeInTheDocument();
    expect(screen.getByRole("spinbutton", { name: "translucency" })).toBeInTheDocument();
    const zoom = screen.getByRole("spinbutton", { name: "zoom" });
    fireEvent.change(zoom, { target: { value: "110" } });
    fireEvent.blur(zoom);
    expect(document.documentElement.style.zoom).toBe("1.1");
    expect(localStorage.getItem("eth-ui-zoom")).toBe("110");
    // The color blobs (modals, arrangement, primary buttons) can be turned off.
    fireEvent.click(screen.getByRole("switch", { name: "color blobs" }));
    expect(document.documentElement.dataset.blobs).toBe("off");
    fireEvent.click(screen.getByRole("switch", { name: "color blobs" }));
    expect(document.documentElement.dataset.blobs).toBeUndefined();
  });

  it("shows the six built-in themes as cards; a click applies one", () => {
    render(<AppearanceSettings />);
    const cards = within(screen.getByRole("group", { name: "theme" })).getAllByRole("button");
    expect(cards.map((c) => c.textContent)).toEqual(["dark", "obsidian", "ember", "tide", "dusk", "light"]);
    expect(screen.getByRole("button", { name: "dark" })).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(screen.getByRole("button", { name: "obsidian" }));
    expect(document.documentElement.dataset.themeId).toBe("obsidian");
    expect(screen.getByRole("button", { name: "obsidian" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "dark" })).toHaveAttribute("aria-pressed", "false");
  });

  it("follows the system with a light and a dark theme to pick", () => {
    render(<AppearanceSettings />);
    expect(screen.queryByRole("combobox", { name: "dark theme" })).toBeNull();
    fireEvent.click(screen.getByRole("switch", { name: "follow the system's light or dark" }));
    expect(screen.getByRole("combobox", { name: "light theme" })).toBeInTheDocument();
    expect(screen.getByRole("combobox", { name: "dark theme" })).toBeInTheDocument();
    // Picking a card stops following it.
    fireEvent.click(screen.getByRole("button", { name: "tide" }));
    expect(screen.getByRole("switch", { name: "follow the system's light or dark" })).toHaveAttribute("aria-checked", "false");
  });
});
