import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { act, screen, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import type { MockTransport } from "@/transport/mock";
import { renderWithMock, resetStores, stubPointerCapture } from "@/features/devices/testUtils";
import { DeviceChain } from "./index";
import { resetCollapsed } from "./collapsed";
import { DeviceWindowsRoot, resetDeviceWindows } from "./window";

// The header's hover-only controls (presets, "+" modulator) take no room until the
// header is hovered or focused, so the device name gets the whole width. jsdom has no layout
// (and no :hover), so this checks the cascade: hidden = no width; focused = shown. The e2e
// (device-header.spec.ts) measures the name's width with and without hover.
let mock: MockTransport | undefined;
let sheet: HTMLStyleElement;
beforeAll(() => {
  stubPointerCapture();
  sheet = document.createElement("style");
  sheet.textContent = readFileSync(resolve(__dirname, "devices.css"), "utf8");
  document.head.append(sheet);
});
afterAll(() => sheet.remove());
afterEach(() => {
  resetStores(mock);
  mock = undefined;
  resetDeviceWindows();
  resetCollapsed();
});

describe("device header hover-only controls (#790)", () => {
  it("take no width until the header is focused, then show presets and +", async () => {
    mock = await renderWithMock(
      <>
        <DeviceChain />
        <DeviceWindowsRoot />
      </>,
    );
    const card = await screen.findByRole("region", { name: "Synth" });
    const slot = within(card).getByTestId("device-on-hover");
    // Right after the name, so growing in only shrinks the name.
    expect(slot.previousElementSibling).toHaveClass("eth-device__name");
    // Hidden: no width, the header's gap taken back.
    // jsdom caches computed styles until the DOM changes (focus doesn't): re-set the sheet.
    const style = () => {
      sheet.textContent = `${sheet.textContent}`;
      return getComputedStyle(slot);
    };
    expect(style().maxWidth).toMatch(/^0(px)?$/);
    expect(style().marginInlineStart).toContain("* -1");
    expect(style().opacity).toBe("0");
    // Owner: no animation, they just appear.
    expect(style().transitionProperty).toMatch(/^(all)?$/);
    expect(style().transitionDuration).toMatch(/^(0s)?$/);

    // Present, and reachable by keyboard: focusing one shows them.
    expect(within(slot).getByRole("button", { name: "presets for Synth" })).toBeTruthy();
    const add = await within(slot).findByRole("button", { name: "add modulator to Synth" });
    act(() => add.focus());
    expect(style().maxWidth).toBe("none");
    expect(style().opacity).toBe("1");
    act(() => add.blur());
    expect(style().maxWidth).toMatch(/^0(px)?$/);
  });
});
