import { afterEach, describe, expect, it } from "vitest";
import { enableMobileUi } from "@/features/mobile/mode";
import { renderDpr, setRenderDprCap } from "./renderDpr";

describe("renderDpr", () => {
  const setDpr = (v: number) => Object.defineProperty(window, "devicePixelRatio", { value: v, configurable: true });
  afterEach(() => {
    setRenderDprCap(Infinity);
    setDpr(1);
  });

  it("is the device's on the desktop, capped at 2 in the mobile UI", () => {
    setDpr(3);
    expect(renderDpr()).toBe(3);
    enableMobileUi(true);
    expect(renderDpr()).toBe(2);
    setDpr(1.5);
    expect(renderDpr()).toBe(1.5);
    enableMobileUi(false);
    setDpr(3);
    expect(renderDpr()).toBe(3);
  });
});
