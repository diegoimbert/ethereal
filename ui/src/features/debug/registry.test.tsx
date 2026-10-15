import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetFloatingWindows } from "@/kit";
import { DebugPanelsRoot, DebugPanelsSettings } from "./DebugPanelsRoot";
import { closeDebugPanel, debugPanelCommands, openDebugPanel, registerDebugPanel, resetDebugPanels, useDebugPanels } from "./registry";

let unregister: Array<() => void> = [];
const register = (...args: Parameters<typeof registerDebugPanel>) => unregister.push(registerDebugPanel(...args));

beforeEach(() => {
  resetDebugPanels();
  resetFloatingWindows();
});
afterEach(() => {
  act(() => {
    for (const u of unregister) u();
    resetDebugPanels();
  });
  unregister = [];
});

describe("debug panels", () => {
  it("a registered panel gets a palette entry that opens it in a floating window", () => {
    const onOpen = vi.fn();
    register({ id: "probe", title: "Probe", render: () => <p>Probe content</p>, defaultSize: { width: 500, height: 320 }, keywords: "sonde", onOpen });
    render(<DebugPanelsRoot />);
    expect(screen.queryByText("Probe content")).toBeNull();
    const cmd = debugPanelCommands().find((c) => c.label === "debug: Probe")!;
    expect(cmd).toMatchObject({ group: "Debug", keywords: "debug panel sonde" });
    act(() => cmd.run());
    const win = screen.getByRole("dialog", { name: "Probe debug panel" });
    expect(win).toContainElement(screen.getByText("Probe content"));
    expect(win).toHaveAttribute("data-debug-panel", "probe");
    expect(win.style.width).toBe("500px");
    expect(win.style.height).toBe("320px");
    expect(onOpen).toHaveBeenCalledTimes(1);
    // Running it again keeps it open (and brings it up).
    act(() => cmd.run());
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(onOpen).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "close the Probe debug panel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("remembers which panels are open", () => {
    register({ id: "a", title: "A", render: () => "a" });
    act(() => openDebugPanel("a"));
    expect(JSON.parse(localStorage.getItem("eth.debug.panels")!)).toEqual(["a"]);
    act(() => closeDebugPanel("a"));
    expect(JSON.parse(localStorage.getItem("eth.debug.panels")!)).toEqual([]);
  });

  it("an open panel registered later shows once registered; unregistered, it goes", () => {
    act(() => openDebugPanel("late"));
    render(<DebugPanelsRoot />);
    expect(screen.queryByRole("dialog")).toBeNull();
    act(() => register({ id: "late", title: "Late", render: () => "late content" }));
    expect(screen.getByRole("dialog", { name: "Late debug panel" })).toBeInTheDocument();
    act(() => unregister.pop()!());
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(useDebugPanels.getState().open).toEqual(["late"]);
  });

  it("a panel that throws shows its error, not a blank app", () => {
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});
    const Boom = () => {
      throw new Error("kaput");
    };
    register({ id: "boom", title: "Boom", render: () => <Boom /> });
    act(() => openDebugPanel("boom"));
    render(<DebugPanelsRoot />);
    expect(screen.getByRole("alert")).toHaveTextContent("the Boom debug panel failed: kaput");
    spy.mockRestore();
  });

  it("Settings › Advanced lists the panels with their open switch", () => {
    register({ id: "x", title: "Xray", render: () => "x content" });
    const onOpen = vi.fn();
    render(
      <>
        <DebugPanelsSettings onOpen={onOpen} />
        <DebugPanelsRoot />
      </>,
    );
    const toggle = screen.getByRole("switch", { name: "Xray" });
    expect(toggle).toHaveAttribute("aria-checked", "false");
    fireEvent.click(toggle);
    expect(screen.getByText("x content")).toBeInTheDocument();
    expect(onOpen).toHaveBeenCalled();
    expect(screen.getByRole("switch", { name: "Xray" })).toHaveAttribute("aria-checked", "true");
  });
});
