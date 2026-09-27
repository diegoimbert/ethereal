import { act, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import {
  Badge,
  Button,
  Dialog,
  getTheme,
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
    render(<Toggle checked={false} onChange={onChange} label="Metronome" />);
    fireEvent.click(screen.getByRole("switch", { name: "Metronome" }));
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("Select reports the picked value", () => {
    const onChange = vi.fn();
    render(
      <Select
        aria-label="S"
        value="a"
        onChange={onChange}
        options={[
          { value: "a", label: "A" },
          { value: "b", label: "B" },
        ]}
      />,
    );
    fireEvent.change(screen.getByRole("combobox", { name: "S" }), { target: { value: "b" } });
    expect(onChange).toHaveBeenCalledWith("b");
  });

  it("TextInput marks invalid", () => {
    render(<TextInput aria-label="T" invalid size="sm" />);
    expect(screen.getByRole("textbox", { name: "T" })).toHaveAttribute("aria-invalid", "true");
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
