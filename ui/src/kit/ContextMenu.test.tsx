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

  it("closes on outside press and Escape, and navigates with arrows", () => {
    const a = vi.fn();
    const b = vi.fn();
    setup([{ label: "A", onSelect: a }, "separator", { label: "B", disabled: true, onSelect: b }, { label: "C", onSelect: b }]);
    fireEvent.contextMenu(screen.getByTestId("target"));
    fireEvent.pointerDown(screen.getByTestId("plain"));
    expect(screen.queryByRole("menu")).toBeNull();

    fireEvent.contextMenu(screen.getByTestId("target"));
    const menu = screen.getByRole("menu");
    fireEvent.keyDown(menu, { key: "ArrowDown" });
    fireEvent.keyDown(menu, { key: "ArrowDown" }); // skips the disabled item
    fireEvent.keyDown(menu, { key: "Enter" });
    expect(b).toHaveBeenCalledOnce();
    expect(a).not.toHaveBeenCalled();

    fireEvent.contextMenu(screen.getByTestId("target"));
    fireEvent.keyDown(screen.getByRole("menu"), { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();
  });
});
