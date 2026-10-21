import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { ContextMenuItem } from "@/kit";
import { PAN_INFO, VOLUME_INFO } from "./params";
import { ValueEditHost, valueMenuEntries } from "./valueMenu";

const items = (entries: ReturnType<typeof valueMenuEntries>) => entries.filter((e): e is ContextMenuItem => e !== "separator");

describe("the value menu", () => {
  it("offers the automation lane only for an automatable value, then set value and reset", () => {
    const anchor = document.createElement("div");
    const withLane = items(valueMenuEntries({ info: PAN_INFO, plain: 0.5, set: () => {}, automation: { show: () => {} } }, anchor));
    expect(withLane.map((e) => e.label)).toEqual(["show automation lane", "set value…", "reset to default"]);
    const without = items(valueMenuEntries({ info: PAN_INFO, plain: 0, set: () => {} }, anchor));
    expect(without.map((e) => e.label)).toEqual(["set value…", "reset to default"]);
    // At its default: nothing to reset.
    expect(without[1]!.disabled).toBe(true);
  });

  it("'set value…' opens a field at the control; Enter applies the typed value", () => {
    render(<ValueEditHost />);
    const set = vi.fn();
    const anchor = document.createElement("div");
    const entry = items(valueMenuEntries({ info: VOLUME_INFO, plain: 0, set }, anchor)).find((e) => e.label === "set value…")!;
    act(() => entry.onSelect!());
    const field = screen.getByRole("textbox", { name: /volume/ });
    fireEvent.change(field, { target: { value: "-6" } });
    fireEvent.keyDown(field, { key: "Enter" });
    expect(set).toHaveBeenCalledWith(-6);
  });
});
