import { act, createEvent, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ContextMenuHost } from "./ContextMenu";
import { openContextMenu, useContextMenuStore } from "./contextMenuStore";

afterEach(() => act(() => useContextMenuStore.getState().close()));

function setup(items: Parameters<typeof openContextMenu>[1]) {
  render(
    <>
      <div data-testid="target" onContextMenu={(e) => openContextMenu(e, items)} />
      <div data-testid="plain" />
      <input data-testid="text" />
      <ContextMenuHost />
    </>,
  );
}

describe("ContextMenu", () => {
  it("opens our menu on right-click and runs the chosen item", () => {
    const del = vi.fn();
    setup([{ label: "Delete", onSelect: del }]);
    fireEvent.contextMenu(screen.getByTestId("target"), { clientX: 10, clientY: 10 });
    fireEvent.click(screen.getByRole("menuitem", { name: "Delete" }));
    expect(del).toHaveBeenCalledOnce();
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("suppresses the native menu everywhere except text fields", () => {
    setup([]);
    const plain = createEvent.contextMenu(screen.getByTestId("plain"));
    fireEvent(screen.getByTestId("plain"), plain);
    expect(plain.defaultPrevented).toBe(true);
    const text = createEvent.contextMenu(screen.getByTestId("text"));
    fireEvent(screen.getByTestId("text"), text);
    expect(text.defaultPrevented).toBe(false);
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("closes on outside press and Escape, and moves focus with the arrows", () => {
    const a = vi.fn();
    const b = vi.fn();
    setup([{ label: "A", onSelect: a }, "separator", { label: "B", disabled: true, onSelect: b }, { label: "C", onSelect: b }]);
    fireEvent.contextMenu(screen.getByTestId("target"));
    fireEvent.pointerDown(screen.getByTestId("plain"));
    expect(screen.queryByRole("menu")).toBeNull();

    fireEvent.contextMenu(screen.getByTestId("target"));
    const menu = screen.getByRole("menu");
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "A" }));
    fireEvent.keyDown(menu, { key: "ArrowDown" }); // skips the disabled item
    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "C" }));
    fireEvent.click(document.activeElement!);
    expect(b).toHaveBeenCalledOnce();
    expect(a).not.toHaveBeenCalled();

    fireEvent.contextMenu(screen.getByTestId("target"));
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("wraps with the arrows and jumps with Home / End", () => {
    setup([
      { label: "A", onSelect: vi.fn() },
      { label: "B", onSelect: vi.fn() },
      { label: "C", onSelect: vi.fn() },
    ]);
    fireEvent.contextMenu(screen.getByTestId("target"));
    const item = (name: string) => screen.getByRole("menuitem", { name });
    fireEvent.keyDown(item("A"), { key: "ArrowUp" });
    expect(item("C")).toHaveFocus();
    fireEvent.keyDown(item("C"), { key: "ArrowDown" });
    expect(item("A")).toHaveFocus();
    fireEvent.keyDown(item("A"), { key: "End" });
    expect(item("C")).toHaveFocus();
    fireEvent.keyDown(item("C"), { key: "Home" });
    expect(item("A")).toHaveFocus();
  });

  it("a disabled item with a reason stays focusable, says why, and does nothing", () => {
    const del = vi.fn();
    setup([
      { label: "Copy", onSelect: vi.fn() },
      { label: "Delete", disabled: true, reason: "the first one can't be deleted", onSelect: del },
    ]);
    fireEvent.contextMenu(screen.getByTestId("target"));
    const item = screen.getByRole("menuitem", { name: "Delete" });
    expect(item).toHaveAttribute("aria-disabled", "true");
    expect(item).toHaveAttribute("title", "the first one can't be deleted");
    expect(item).toHaveAccessibleDescription("the first one can't be deleted");
    fireEvent.keyDown(screen.getByRole("menuitem", { name: "Copy" }), { key: "ArrowDown" });
    expect(item).toHaveFocus();
    fireEvent.click(item);
    expect(del).not.toHaveBeenCalled();
    expect(screen.getByRole("menu")).toBeInTheDocument();
  });

  it("marks userText item labels as user content (eth-user-text)", () => {
    setup([
      { label: "Rename", onSelect: () => undefined },
      { label: "Send to Bass DI", userText: true, onSelect: () => undefined },
    ]);
    fireEvent.contextMenu(screen.getByTestId("target"));
    const label = (name: string) => screen.getByRole("menuitem", { name }).querySelector(".eth-menu__label");
    expect(label("Send to Bass DI")).toHaveClass("eth-user-text");
    expect(label("Rename")).not.toHaveClass("eth-user-text");
  });
});
