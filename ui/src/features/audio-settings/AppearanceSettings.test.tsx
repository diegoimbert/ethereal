import { afterEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { setZoom } from "@/theme";
import { AppearanceSettings } from "./AppearanceSettings";

afterEach(() => setZoom(100));

describe("Settings > Appearance", () => {
  it("has the theme, the interface zoom (scales the page, remembered) and translucency", () => {
    render(<AppearanceSettings />);
    expect(screen.getByRole("combobox", { name: "theme" })).toBeInTheDocument();
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
});
