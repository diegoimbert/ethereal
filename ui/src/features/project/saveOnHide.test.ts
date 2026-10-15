import { afterEach, describe, expect, it, vi } from "vitest";
import { bindSaveOnHide } from "./saveOnHide";

let visibility: DocumentVisibilityState = "visible";
Object.defineProperty(document, "visibilityState", { configurable: true, get: () => visibility });

afterEach(() => {
  visibility = "visible";
});

describe("bindSaveOnHide (#565)", () => {
  it("saves unsaved changes when the tab is hidden or closed, and nothing else", async () => {
    let dirty = true;
    const save = vi.fn();
    const wait = vi.fn();
    const off = bindSaveOnHide(() => dirty, save, wait);

    visibility = "visible";
    document.dispatchEvent(new Event("visibilitychange"));
    expect(save).not.toHaveBeenCalled();

    visibility = "hidden";
    document.dispatchEvent(new Event("visibilitychange"));
    expect(save).toHaveBeenCalledTimes(1);
    await Promise.resolve();
    // Hidden, the page lives on: no waiting.
    expect(wait).not.toHaveBeenCalled();

    // Closing: the save, then (once it reached the engine, a microtask later) the wait.
    window.dispatchEvent(new Event("pagehide"));
    expect(save).toHaveBeenCalledTimes(2);
    expect(wait).not.toHaveBeenCalled();
    await Promise.resolve();
    expect(wait).toHaveBeenCalledOnce();

    // Saved already: nothing to do.
    dirty = false;
    window.dispatchEvent(new Event("pagehide"));
    document.dispatchEvent(new Event("visibilitychange"));
    expect(save).toHaveBeenCalledTimes(2);

    dirty = true;
    off();
    window.dispatchEvent(new Event("pagehide"));
    document.dispatchEvent(new Event("visibilitychange"));
    expect(save).toHaveBeenCalledTimes(2);
  });
});
