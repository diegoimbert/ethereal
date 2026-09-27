import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { setTheme } from "@/kit";
import { KitGallery } from "./index";

afterEach(() => act(() => setTheme("dark")));

describe("KitGallery", () => {
  it("renders every component family and token section", () => {
    render(<KitGallery />);
    for (const name of ["Buttons", "Knobs, faders, meters", "Inputs, tabs, badges", "Overlays", "Track palette", "Typography", "Spacing", "Motion"]) {
      expect(screen.getByRole("region", { name })).toBeInTheDocument();
    }
    expect(screen.getAllByRole("slider").length).toBeGreaterThan(4);
    expect(screen.getAllByRole("meter").length).toBe(2);
    expect(screen.getByText("--eth-color-accent")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Component tokens: knob" })).toHaveTextContent("--knob-track");
  });

  it("switches theme from the toggle", () => {
    render(<KitGallery />);
    fireEvent.click(screen.getByRole("tab", { name: "light" }));
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(screen.getByRole("region", { name: "Colors (light)" })).toBeInTheDocument();
  });
});
