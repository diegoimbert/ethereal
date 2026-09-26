import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { App } from "./App";

describe("App shell", () => {
  it("mounts every feature slot", () => {
    const { container } = render(<App />);
    for (const f of ["transport-bar", "project", "recording", "browser", "arrangement", "devices"]) {
      expect(container.querySelector(`[data-feature="${f}"]`), f).not.toBeNull();
    }
  });

  it("mounts the arrangement as the main view", () => {
    const { container } = render(<App />);
    expect(container.querySelector('[data-feature="arrangement"]')).not.toBeNull();
  });

  it("switches detail tabs", () => {
    const { container } = render(<App />);
    fireEvent.click(screen.getByRole("tab", { name: "Mixer" }));
    expect(container.querySelector('[data-feature="mixer"]')).not.toBeNull();
    fireEvent.click(screen.getByRole("tab", { name: "Plugins" }));
    expect(container.querySelector('[data-feature="plugins"]')).not.toBeNull();
  });
});
