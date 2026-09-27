import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import {
  Badge,
  Button,
  Dialog,
  Fader,
  getTheme,
  Popover,
  IconButton,
  Knob,
  Menu,
  NumberField,
  Select,
  setTheme,
  Tabs,
  TextInput,
  Toggle,
  Tooltip,
} from "./index";
import { pickOption } from "./testing";

describe("Button", () => {
  it("maps tone and size to classes; deprecated variant still works", () => {
    render(
      <>
        <Button tone="danger" size="lg">
          a
        </Button>
        <Button variant="primary">b</Button>
        <Button variant="ghost" active>
          c
        </Button>
      </>,
    );
    expect(screen.getByRole("button", { name: "a" })).toHaveClass("eth-button--danger", "eth-button--lg");
    expect(screen.getByRole("button", { name: "b" })).toHaveClass("eth-button--accent", "eth-button--md");
    const c = screen.getByRole("button", { name: "c" });
    expect(c).toHaveClass("eth-button--ghost");
    expect(c).toHaveAttribute("aria-pressed", "true");
  });

  it("IconButton has an accessible name", () => {
    render(<IconButton label="Close" icon="×" />);
    expect(screen.getByRole("button", { name: "Close" })).toHaveClass("eth-icon-button");
  });
});

describe("Knob", () => {
  it("uses size tokens by name and a --knob-size override for pixels", () => {
    const { container } = render(
      <>
        <Knob value={0.5} size="lg" label="A" />
        <Knob value={0.5} size={50} label="B" />
      </>,
    );
    const [a, b] = container.querySelectorAll<HTMLElement>(".eth-knob");
    expect(a).toHaveClass("eth-knob--lg");
    expect(b!.style.getPropertyValue("--knob-size")).toBe("50px");
  });
});

describe("Fader", () => {
  it("drag sensitivity follows the measured height (full-height drag = full range)", () => {
    const onChange = vi.fn();
    render(<Fader value={0} onChange={onChange} label="Vol" />);
    const f = screen.getByRole("slider", { name: "Vol" });
    Object.defineProperty(f, "clientHeight", { value: 200 });
    f.setPointerCapture = () => undefined;
    f.hasPointerCapture = () => false;
    fireEvent.pointerDown(f, { button: 0, clientY: 300, pointerId: 1 });
    fireEvent.pointerMove(f, { clientY: 200, pointerId: 1 });
    expect(onChange).toHaveBeenLastCalledWith(0.5);
    expect(f.style.height).toBe("");
  });
});

describe("Tabs", () => {
  function Harness() {
    const [v, setV] = useState<"a" | "b" | "c">("a");
    return (
      <Tabs
        label="T"
        value={v}
        onChange={setV}
        items={[
          { id: "a", label: "A" },
          { id: "b", label: "B", disabled: true },
          { id: "c", label: "C" },
        ]}
      />
    );
  }
  it("selects on click and moves with arrow keys, skipping disabled tabs", () => {
    render(<Harness />);
    expect(screen.getByRole("tab", { name: "A" })).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(screen.getByRole("tablist"), { key: "ArrowRight" });
    expect(screen.getByRole("tab", { name: "C" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "C" })).toHaveFocus();
    fireEvent.click(screen.getByRole("tab", { name: "A" }));
    expect(screen.getByRole("tab", { name: "A" })).toHaveAttribute("aria-selected", "true");
  });
});

describe("fields", () => {
  it("Toggle flips", () => {
    const onChange = vi.fn();
    render(<Toggle checked={false} onChange={onChange} label="Metronome" size="lg" />);
    expect(screen.getByText("Metronome").parentElement).toHaveClass("eth-toggle--lg");
    fireEvent.click(screen.getByRole("switch", { name: "Metronome" }));
    expect(onChange).toHaveBeenCalledWith(true);
  });

  describe("Select", () => {
    const options = [
      { value: "a", label: "Apple", group: "Fruit" },
      { value: "b", label: "Banana", group: "Fruit" },
      { value: "c", label: "Carrot", group: "Veg", disabled: true },
      { value: "d", label: "Daikon", group: "Veg" },
    ] as const;
    const setup = (value = "a") => {
      const onChange = vi.fn();
      render(<Select aria-label="S" value={value} onChange={onChange} options={options} placeholder="Pick…" />);
      return { onChange, trigger: screen.getByRole("combobox", { name: "S" }) };
    };

    it("opens our own list and reports the picked option", async () => {
      const { onChange, trigger } = setup();
      expect(trigger).toHaveTextContent("Apple");
      fireEvent.click(trigger);
      expect(trigger).toHaveAttribute("aria-expanded", "true");
      expect(screen.getByRole("option", { name: "Apple" })).toHaveAttribute("aria-selected", "true");
      expect(screen.getByText("Veg")).toBeInTheDocument(); // group heading
      fireEvent.click(screen.getByRole("option", { name: "Banana" }));
      expect(onChange).toHaveBeenCalledWith("b");
      // Closes with an exit animation, then unmounts.
      expect(trigger).toHaveAttribute("aria-expanded", "false");
      await waitFor(() => expect(screen.queryByRole("listbox")).toBeNull());
    });

    it("keyboard: arrows skip disabled options, type-ahead, Enter picks, Escape closes", async () => {
      const { onChange, trigger } = setup("b");
      fireEvent.keyDown(trigger, { key: "ArrowDown" });
      fireEvent.keyDown(trigger, { key: "ArrowDown" }); // Banana → (Carrot disabled) → Daikon
      expect(trigger.getAttribute("aria-activedescendant")).toBe(
        screen.getByRole("option", { name: "Daikon" }).id,
      );
      fireEvent.keyDown(trigger, { key: "a" });
      expect(trigger.getAttribute("aria-activedescendant")).toBe(screen.getByRole("option", { name: "Apple" }).id);
      fireEvent.keyDown(trigger, { key: "Enter" });
      expect(onChange).toHaveBeenCalledWith("a");
      fireEvent.keyDown(trigger, { key: " " });
      expect(screen.getByRole("listbox")).toBeInTheDocument();
      fireEvent.keyDown(trigger, { key: "Escape" });
      await waitFor(() => expect(screen.queryByRole("listbox")).toBeNull());
      expect(onChange).toHaveBeenCalledTimes(1);
    });

    it("shows the placeholder for a value that isn't an option; pickOption helper", () => {
      const { onChange, trigger } = setup("zzz");
      expect(trigger).toHaveTextContent("Pick…");
      pickOption(trigger, { value: "d" });
      expect(onChange).toHaveBeenCalledWith("d");
    });
  });

  it("TextInput marks invalid", () => {
    render(<TextInput aria-label="T" invalid size="sm" />);
    expect(screen.getByRole("textbox", { name: "T" })).toHaveAttribute("aria-invalid", "true");
  });

  it("NumberField: hold and drag up/down changes the value (shift: fine); a click still types", () => {
    const onChange = vi.fn();
    const start = vi.fn();
    const end = vi.fn();
    render(<NumberField aria-label="N" value={10} onChange={onChange} step={1} min={0} max={20} onChangeStart={start} onChangeEnd={end} />);
    const input = screen.getByRole("spinbutton", { name: "N" });
    fireEvent.pointerDown(input, { button: 0, pointerId: 1, clientY: 100 });
    fireEvent.pointerMove(input, { pointerId: 1, clientY: 88 }); // 12 px up, 4 px per step
    expect(start).toHaveBeenCalledOnce();
    expect(onChange).toHaveBeenLastCalledWith(13);
    fireEvent.pointerMove(input, { pointerId: 1, clientY: 200, shiftKey: true }); // 100 px down, tenth steps
    expect(onChange).toHaveBeenLastCalledWith(7.5);
    fireEvent.pointerMove(input, { pointerId: 1, clientY: 400 });
    expect(onChange).toHaveBeenLastCalledWith(0); // clamped
    fireEvent.pointerUp(input, { pointerId: 1, clientY: 400 });
    expect(end).toHaveBeenCalledOnce();
    expect(document.activeElement).not.toBe(input);
    // A click without a drag focuses it for typing.
    fireEvent.pointerDown(input, { button: 0, pointerId: 2, clientY: 100 });
    fireEvent.pointerUp(input, { pointerId: 2, clientY: 100 });
    expect(document.activeElement).toBe(input);
  });

  it("NumberField commits on Enter (clamped), steps with arrows, reverts on Escape", () => {
    const onChange = vi.fn();
    render(<NumberField aria-label="N" value={120} onChange={onChange} min={20} max={200} unit="BPM" />);
    const input = screen.getByRole("spinbutton", { name: "N" });
    fireEvent.change(input, { target: { value: "999" } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onChange).toHaveBeenLastCalledWith(200);
    fireEvent.keyDown(input, { key: "ArrowUp", shiftKey: true });
    expect(onChange).toHaveBeenLastCalledWith(130);
    fireEvent.change(input, { target: { value: "50" } });
    fireEvent.keyDown(input, { key: "Escape" });
    expect(input).toHaveValue("120");
    fireEvent.blur(input);
    expect(onChange).toHaveBeenCalledTimes(2);
  });
});

describe("overlays", () => {
  it("Menu opens, selects an item and closes; Escape closes", () => {
    const onSelect = vi.fn();
    render(
      <Menu
        aria-label="File"
        trigger={(p) => <Button {...p}>File</Button>}
        items={[
          { id: "a", label: "New", onSelect },
          { id: "s", separator: true },
          { id: "b", label: "Gone", disabled: true, onSelect: () => undefined },
        ]}
      />,
    );
    const trigger = screen.getByRole("button", { name: "File" });
    expect(trigger).toHaveAttribute("aria-haspopup", "menu");
    fireEvent.click(trigger);
    expect(screen.getByRole("menuitem", { name: "New" })).toHaveFocus();
    fireEvent.click(screen.getByRole("menuitem", { name: "New" }));
    expect(onSelect).toHaveBeenCalled();
    expect(screen.queryByRole("menu")).toBeNull();
    fireEvent.click(trigger);
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("Popover renders in a portal on <body> (never clipped by panels) and closes on outside click", () => {
    const { container } = render(
      <div style={{ overflow: "hidden" }}>
        <Popover trigger={(p) => <Button {...p}>Open</Button>} aria-label="Pop">
          content
        </Popover>
      </div>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Open" }));
    const pop = screen.getByRole("dialog", { name: "Pop" });
    expect(container.contains(pop)).toBe(false);
    expect(pop.parentElement).toBe(document.body);
    fireEvent.pointerDown(pop);
    expect(screen.getByRole("dialog", { name: "Pop" })).toBeInTheDocument();
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole("dialog", { name: "Pop" })).toBeNull();
  });

  it("Dialog is labelled and closes on Escape", () => {
    const onClose = vi.fn();
    render(
      <Dialog open onClose={onClose} title="Settings">
        body
      </Dialog>,
    );
    const d = screen.getByRole("dialog", { name: "Settings" });
    expect(d).toHaveAttribute("aria-modal", "true");
    fireEvent.keyDown(d, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });

  it("Tooltip shows on hover and describes its trigger", () => {
    render(
      <Tooltip content="Loop">
        <button type="button">L</button>
      </Tooltip>,
    );
    const b = screen.getByRole("button", { name: "L" });
    fireEvent.pointerEnter(b.parentElement!);
    expect(b).toHaveAccessibleDescription("Loop");
    fireEvent.pointerLeave(b.parentElement!);
    expect(screen.queryByRole("tooltip")).toBeNull();
  });

  it("Badge renders its tone", () => {
    render(<Badge tone="warn">3</Badge>);
    expect(screen.getByText("3")).toHaveClass("eth-badge--warn");
  });
});

describe("theme switching", () => {
  it("applies the theme as <html data-theme> and persists it", () => {
    expect(getTheme()).toBe("dark");
    act(() => setTheme("light"));
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(localStorage.getItem("eth-theme")).toBe("light");
    act(() => setTheme("dark"));
    expect(getTheme()).toBe("dark");
  });
});
