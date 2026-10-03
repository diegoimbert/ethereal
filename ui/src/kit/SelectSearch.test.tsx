// base-132: a Select with thousands of options (e.g. every param of a plugin, in the
// automation lane picker) is searchable and windowed: never more than VIRTUAL_MAX_ROWS rows.
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { pickOption } from "./testing";
import { SEARCH_OVER, Select } from "./Select";
import { VIRTUAL_MAX_ROWS } from "./VirtualList";

afterEach(cleanup);

const OPTIONS = Array.from({ length: 10_000 }, (_, i) => ({ value: `p${i}`, label: `Param ${i}`, group: i < 2 ? "Mixer" : "Plugin" }));

function Harness({ onPick }: { onPick?: (v: string) => void }) {
  const [v, setV] = useState("");
  return (
    <Select
      aria-label="Show parameter"
      placeholder="+ Parameter…"
      value={v}
      options={OPTIONS}
      onChange={(x) => {
        setV(x);
        onPick?.(x);
      }}
    />
  );
}

describe("searchable Select", () => {
  it("is on above SEARCH_OVER options and mounts only a window of them", () => {
    expect(OPTIONS.length).toBeGreaterThan(SEARCH_OVER);
    render(<Harness />);
    fireEvent.click(screen.getByRole("combobox", { name: "Show parameter" }));
    const listbox = screen.getByRole("listbox", { name: "Show parameter" });
    const options = within(listbox).getAllByRole("option");
    expect(options.length).toBeGreaterThan(5);
    expect(options.length).toBeLessThanOrEqual(VIRTUAL_MAX_ROWS);
    expect(screen.getByRole("searchbox", { name: "Search Show parameter" })).toBe(document.activeElement);
  });

  it("filters as you type; arrows and Enter pick from the search field", () => {
    const picked: string[] = [];
    render(<Harness onPick={(v) => picked.push(v)} />);
    fireEvent.click(screen.getByRole("combobox", { name: "Show parameter" }));
    const search = screen.getByRole("searchbox", { name: "Search Show parameter" });
    act(() => {
      fireEvent.change(search, { target: { value: "param 999" } });
    });
    const listbox = screen.getByRole("listbox", { name: "Show parameter" });
    // "Param 999", "Param 9990".."Param 9999"
    expect(within(listbox).getAllByRole("option")).toHaveLength(11);
    fireEvent.keyDown(search, { key: "ArrowDown" });
    fireEvent.keyDown(search, { key: "Enter" });
    expect(picked).toEqual(["p9990"]);
    expect(screen.getByRole("combobox", { name: "Show parameter" }).textContent).toContain("Param 9990");
  });

  it("works with the shared pickOption helper (options near the top)", () => {
    const picked: string[] = [];
    render(<Harness onPick={(v) => picked.push(v)} />);
    pickOption(screen.getByRole("combobox", { name: "Show parameter" }), { value: "p1" });
    expect(picked).toEqual(["p1"]);
  });
});
