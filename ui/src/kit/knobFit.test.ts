import { renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useCenterFit } from "./knobFit";

describe("useCenterFit", () => {
  it("observes the dial once per knob, not once per value change (performance audit #20)", () => {
    const observed: Element[] = [];
    const disconnects = { n: 0 };
    const RO = globalThis.ResizeObserver;
    globalThis.ResizeObserver = class {
      observe(el: Element) {
        observed.push(el);
      }
      unobserve() {}
      disconnect() {
        disconnects.n++;
      }
    } as unknown as typeof ResizeObserver;
    try {
      const refs = {
        dial: { current: document.createElement("span") },
        value: { current: document.createElement("span") },
        unit: { current: document.createElement("span") },
      };
      const { rerender, unmount } = renderHook(({ text }) => useCenterFit(text, refs), { initialProps: { text: "-18.0 dB" } });
      for (const text of ["-17.0 dB", "-16.0 dB", "-15.0 dB"]) rerender({ text });
      expect(observed).toEqual([refs.dial.current]);
      expect(disconnects.n).toBe(0);
      // No center text (small knob): nothing observed.
      rerender({ text: undefined as unknown as string });
      expect(disconnects.n).toBe(1);
      unmount();
      expect(disconnects.n).toBe(1);
    } finally {
      globalThis.ResizeObserver = RO;
    }
  });
});
