import { afterEach, describe, expect, it } from "vitest";
import { getGlow, getGlowSetting, setGlow, setTheme } from "./index";

afterEach(() => {
  setTheme("dark");
  setGlow(true);
});

describe("glow", () => {
  it("never glows in the light theme, whatever the setting; the setting is kept for the dark one", () => {
    setGlow(true);
    setTheme("light");
    expect(getGlow()).toBe(false);
    expect(document.documentElement.dataset.glow).toBe("off");
    expect(getGlowSetting()).toBe(true);
    setTheme("dark");
    expect(getGlow()).toBe(true);
    setGlow(false);
    expect(getGlow()).toBe(false);
  });
});
