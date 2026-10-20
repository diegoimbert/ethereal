import { afterEach, describe, expect, it } from "vitest";
import { cascade } from "./DeviceWindow";

describe("device window cascade", () => {
  afterEach(() => document.body.replaceChildren());

  it("opens below the top bar, also when a band sits above the shell", () => {
    const shell = document.createElement("div");
    shell.className = "eth-shell";
    document.body.append(shell);
    const atTop = cascade(0, 400).y;
    // The web's "Open on desktop" band pushes the shell, and its transport, down 34 px:
    // the window opens as far below it, not over the transport.
    shell.getBoundingClientRect = () => ({ top: 34, left: 0, right: 0, bottom: 0, width: 0, height: 0, x: 0, y: 34, toJSON: () => ({}) });
    expect(cascade(0, 400).y).toBe(atTop + 34);
    // The cascade steps down from there.
    expect(cascade(1, 400).y).toBeGreaterThan(atTop + 34);
  });
});
