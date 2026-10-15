import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FloatingWindow, resetFloatingWindows } from "@/kit";
import { useProjectStore } from "@/state";
import { quitBridge, setQuitBridge, type QuitBridge } from "./bridge";
import { QuitRoot } from "./QuitRoot";
import { useQuitConfirm } from "./store";

/** A shell stand-in: records the calls and lets the test emit its events. */
function mockBridge() {
  const quitHandlers = new Set<() => void>();
  const closeHandlers = new Set<() => void>();
  const bridge = {
    onQuitRequested: vi.fn(async (h: () => void) => {
      quitHandlers.add(h);
      return () => quitHandlers.delete(h);
    }),
    onCloseRequested: vi.fn(async (h: () => void) => {
      closeHandlers.add(h);
      return () => closeHandlers.delete(h);
    }),
    setGuard: vi.fn(async () => {}),
    quit: vi.fn(async () => {}),
    cancel: vi.fn(async () => {}),
  } satisfies QuitBridge;
  const emit = (set: Set<() => void>) => act(() => set.forEach((h) => h()));
  return { bridge, requestQuit: () => emit(quitHandlers), requestClose: () => emit(closeHandlers) };
}

let shell: ReturnType<typeof mockBridge>;

async function mount(extra?: React.ReactNode) {
  shell = mockBridge();
  setQuitBridge(shell.bridge);
  const r = render(
    <>
      <QuitRoot />
      {extra}
    </>,
  );
  // Listeners registered (async in Tauri).
  await act(async () => {});
  return r;
}

const dialog = () => screen.queryByRole("dialog", { name: /quit ethereal/i });

beforeEach(() => {
  useQuitConfirm.setState({ open: false });
  useProjectStore.setState({ dirty: false });
  resetFloatingWindows();
});

afterEach(() => {
  setQuitBridge(undefined);
});

describe("quit confirmation", () => {
  it("web build: no bridge, nothing rendered", () => {
    setQuitBridge(null);
    expect(quitBridge()).toBeNull();
    const { container } = render(<QuitRoot />);
    expect(container).toBeEmptyDOMElement();
  });

  it("tells the shell it listens, and asks only when the shell requests a quit", async () => {
    await mount();
    expect(shell.bridge.setGuard).toHaveBeenCalledWith(true);
    expect(dialog()).toBeNull();
    shell.requestQuit();
    expect(dialog()).not.toBeNull();
  });

  it("Quit quits", async () => {
    await mount();
    shell.requestQuit();
    fireEvent.click(screen.getByRole("button", { name: "quit" }));
    expect(shell.bridge.quit).toHaveBeenCalledTimes(1);
    expect(shell.bridge.cancel).not.toHaveBeenCalled();
    expect(useQuitConfirm.getState().open).toBe(false);
  });

  it("Cancel keeps running", async () => {
    await mount();
    shell.requestQuit();
    fireEvent.click(screen.getByRole("button", { name: "cancel" }));
    expect(shell.bridge.cancel).toHaveBeenCalledTimes(1);
    expect(shell.bridge.quit).not.toHaveBeenCalled();
    expect(useQuitConfirm.getState().open).toBe(false);
  });

  it("Enter quits, Escape cancels", async () => {
    await mount();
    shell.requestQuit();
    fireEvent.keyDown(dialog()!, { key: "Escape" });
    expect(shell.bridge.cancel).toHaveBeenCalledTimes(1);
    shell.requestQuit();
    fireEvent.keyDown(dialog()!, { key: "Enter" });
    expect(shell.bridge.quit).toHaveBeenCalledTimes(1);
  });

  it("Enter on the focused Cancel button cancels, not quits", async () => {
    await mount();
    shell.requestQuit();
    const cancel = screen.getByRole("button", { name: "cancel" });
    fireEvent.keyDown(cancel, { key: "Enter" });
    expect(shell.bridge.quit).not.toHaveBeenCalled();
  });

  it("mentions unsaved changes (saved on the way out)", async () => {
    useProjectStore.setState({ dirty: true });
    await mount();
    shell.requestQuit();
    expect(dialog()).toHaveTextContent(/saved before ethereal closes/);
  });

  it("stops listening on unmount", async () => {
    const r = await mount();
    r.unmount();
    shell.requestQuit();
    expect(useQuitConfirm.getState().open).toBe(false);
  });
});

describe("⌘W in the main window", () => {
  function Win({ id, onClose }: { id: string; onClose: () => void }) {
    return (
      <FloatingWindow id={id} title={id} label={id} onClose={onClose}>
        body
      </FloatingWindow>
    );
  }

  it("closes the topmost floating window only", async () => {
    const a = vi.fn();
    const b = vi.fn();
    await mount(
      <>
        <Win id="a" onClose={a} />
        <Win id="b" onClose={b} />
      </>,
    );
    shell.requestClose();
    expect(b).toHaveBeenCalledTimes(1);
    expect(a).not.toHaveBeenCalled();
    expect(shell.bridge.quit).not.toHaveBeenCalled();
  });

  it("does nothing without a floating window, or while the quit dialog shows", async () => {
    const a = vi.fn();
    await mount(<Win id="a" onClose={a} />);
    shell.requestQuit();
    shell.requestClose();
    expect(a).not.toHaveBeenCalled();
    resetFloatingWindows();
    useQuitConfirm.setState({ open: false });
    shell.requestClose();
    expect(a).not.toHaveBeenCalled();
    expect(shell.bridge.quit).not.toHaveBeenCalled();
  });
});
