import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { EngineErrorScreen } from "./EngineErrorScreen";

describe("EngineErrorScreen (#597)", () => {
  it("says the engine didn't start and why, with retry and reload", () => {
    const onRetry = vi.fn();
    const onReload = vi.fn();
    render(<EngineErrorScreen error={new Error("this browser has no AudioWorklet support")} onRetry={onRetry} onReload={onReload} />);
    expect(screen.getByRole("heading")).toHaveTextContent("the audio engine didn't start");
    expect(screen.getByRole("alert")).toHaveTextContent("this browser has no AudioWorklet support");
    fireEvent.click(screen.getByRole("button", { name: "retry" }));
    expect(onRetry).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: "reload" }));
    expect(onReload).toHaveBeenCalledOnce();
  });

  it("without retry, reload is the way out; a reason-less error still says something", () => {
    render(<EngineErrorScreen error={{}} />);
    expect(screen.queryByRole("button", { name: "retry" })).toBeNull();
    expect(screen.getByRole("alert")).toHaveTextContent("unknown error");
  });

  it("a part of the app that didn't load (after a deploy): a new version is out, reload", () => {
    const onReload = vi.fn();
    render(<EngineErrorScreen error={new TypeError("Failed to fetch dynamically imported module: /assets/endpoint-abc.js")} onReload={onReload} />);
    expect(screen.getByRole("heading")).toHaveTextContent("a new version of ethereal is out");
    fireEvent.click(screen.getByRole("button", { name: "reload" }));
    expect(onReload).toHaveBeenCalledOnce();
  });
});
