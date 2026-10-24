import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { installReloadGuard, isReloadChord } from "./reloadGuard";
import { DEFAULT_KEYMAP, matchesAction, useKeymapStore, withBinding } from "./store";

const press = (key: string, init: KeyboardEventInit = {}, target: EventTarget = document.body) => {
  const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(e);
  return e;
};

let uninstall: () => void;
beforeEach(() => {
  uninstall = installReloadGuard();
});
afterEach(() => {
  uninstall();
  useKeymapStore.setState({ keymap: DEFAULT_KEYMAP });
});

describe("reload guard", () => {
  it("cancels ctrl/cmd+r, ctrl+shift+r and f5 everywhere, text fields too", () => {
    expect(press("r", { ctrlKey: true }).defaultPrevented).toBe(true);
    expect(press("r", { metaKey: true }).defaultPrevented).toBe(true);
    expect(press("R", { ctrlKey: true, shiftKey: true }).defaultPrevented).toBe(true);
    expect(press("F5").defaultPrevented).toBe(true);
    const input = document.body.appendChild(document.createElement("input"));
    expect(press("r", { ctrlKey: true }, input).defaultPrevented).toBe(true);
    input.remove();
  });

  it("leaves other keys alone (plain r is the reverse shortcut)", () => {
    expect(press("r").defaultPrevented).toBe(false);
    expect(press("t", { ctrlKey: true }).defaultPrevented).toBe(false);
    expect(isReloadChord({ key: "r", altKey: true, ctrlKey: false, metaKey: false, shiftKey: false })).toBe(false);
  });

  it("a chord the user bound still reaches its handler, then the reload is cancelled", () => {
    useKeymapStore.setState({ keymap: withBinding(DEFAULT_KEYMAP, "transport.loop", ["Mod+R"]) });
    let seen: boolean | null = null;
    const handler = (e: KeyboardEvent) => {
      if (matchesAction("transport.loop", e)) seen = e.defaultPrevented;
    };
    document.addEventListener("keydown", handler);
    const e = press("r", { ctrlKey: true });
    document.removeEventListener("keydown", handler);
    expect(seen).toBe(false);
    expect(e.defaultPrevented).toBe(true);
  });
});
