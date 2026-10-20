// A Select with thousands of options (e.g. every param of a plugin, in the
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
    expect(screen.getByRole("searchbox", { name: "search Show parameter" })).toBe(document.activeElement);
  });

  it("filters as you type; arrows and Enter pick from the search field", () => {
    const picked: string[] = [];
    render(<Harness onPick={(v) => picked.push(v)} />);
    fireEvent.click(screen.getByRole("combobox", { name: "Show parameter" }));
    const search = screen.getByRole("searchbox", { name: "search Show parameter" });
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

describe("Select 'Active' filter", () => {
  const AOPTS = Array.from({ length: 60 }, (_, i) => ({ value: `a${i}`, label: `Param ${i}`, active: i === 3 || i === 7 }));
  const NONE = AOPTS.map((o) => ({ ...o, active: false }));
  const key = "test.activeOnly";
  const Stored = ({ options }: { options: typeof AOPTS }) => {
    // Read the stored value at each mount, like the app's persisted preference.
    const [on, setOn] = useState(() => localStorage.getItem(key) === "1");
    return (
      <Select
        aria-label="Param"
        value=""
        options={options}
        onChange={() => {}}
        activeFilter={{
          value: on,
          onChange: (v) => {
            localStorage.setItem(key, v ? "1" : "0");
            setOn(v);
          },
        }}
      />
    );
  };
  const open = () => fireEvent.click(screen.getByRole("combobox", { name: "Param" }));
  const count = () => within(screen.getByRole("listbox", { name: "Param" })).getAllByRole("option").length;
  const toggle = () => screen.getByRole("button", { name: "show only automated parameters" });
  afterEach(() => localStorage.removeItem(key));

  it("limits the list to active options, and combines with search", () => {
    render(<Stored options={AOPTS} />);
    open();
    expect(toggle().getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(toggle());
    expect(toggle().getAttribute("aria-pressed")).toBe("true");
    expect(count()).toBe(2);
    fireEvent.change(screen.getByRole("searchbox", { name: "search Param" }), { target: { value: "param 7" } });
    expect(count()).toBe(1);
  });

  it("persists across remounts", () => {
    const { unmount } = render(<Stored options={AOPTS} />);
    open();
    fireEvent.click(toggle());
    unmount();
    render(<Stored options={AOPTS} />);
    open();
    expect(toggle().getAttribute("aria-pressed")).toBe("true");
    expect(count()).toBe(2);
  });

  it("is off, disabled and shows everything when nothing is active, keeping the stored value", () => {
    localStorage.setItem(key, "1");
    render(<Stored options={NONE} />);
    open();
    expect(toggle().getAttribute("aria-pressed")).toBe("false");
    expect((toggle() as HTMLButtonElement).disabled).toBe(true);
    expect(count()).toBeGreaterThan(2);
    expect(localStorage.getItem(key)).toBe("1");
  });
});
