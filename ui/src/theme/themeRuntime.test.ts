import { afterEach, describe, expect, it, vi } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { readPalette } from "@/features/arrangement/gl/palette";
import { lanePalette } from "@/features/arrangement/gl/layers/lanePalette";
import { builtinTheme } from "./builtinThemes";
import {
  deleteCustomTheme,
  getGlow,
  getTheme,
  getThemeId,
  getThemeKey,
  getThemes,
  initThemeSelection,
  previewTheme,
  saveCustomTheme,
  setSchemeTheme,
  setTheme,
  setThemeId,
  setThemeMode,
  themeColorValue,
  useThemeColor,
} from "./index";

const root = document.documentElement;
const rgb = (hex: string) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);

function mockSystem(light: boolean) {
  const listeners: Array<() => void> = [];
  const mq = { matches: light, addEventListener: (_: string, l: () => void) => listeners.push(l) };
  vi.stubGlobal("matchMedia", () => mq);
  return (next: boolean) => {
    mq.matches = next;
    listeners.forEach((l) => l());
  };
}

afterEach(() => {
  previewTheme(null);
  for (const d of getThemes()) if (d.id.startsWith("custom-")) deleteCustomTheme(d.id);
  setThemeId("light");
  setThemeId("dark");
  vi.unstubAllGlobals();
});

describe("theme switching", () => {
  it("puts the scheme on data-theme and the theme on data-theme-id, and remembers both", () => {
    setThemeId("obsidian");
    expect(root.dataset.theme).toBe("dark");
    expect(root.dataset.themeId).toBe("obsidian");
    expect(getTheme()).toBe("dark");
    expect(localStorage.getItem("eth-theme")).toBe("dark");
    expect(localStorage.getItem("eth-theme-id")).toBe("obsidian");
    // The light/dark command comes back to the dark theme last chosen.
    setTheme("light");
    expect(root.dataset.themeId).toBe("light");
    expect(getGlow()).toBe(false);
    setTheme("dark");
    expect(getThemeId()).toBe("obsidian");
  });

  it("restores the remembered theme, unless something else wrote the scheme alone", () => {
    setThemeId("ember");
    initThemeSelection();
    expect(root.dataset.themeId).toBe("ember");
    // Tests and older versions write `eth-theme` only: that scheme's theme applies.
    localStorage.setItem("eth-theme", "light");
    initThemeSelection();
    expect(root.dataset.themeId).toBe("light");
    expect(root.dataset.theme).toBe("light");
  });

  it("applies the user's themes through a style of their own, and the editor's preview without choosing it", () => {
    const def = { id: "custom-test", name: "mine", base: "dark" as const, colors: { accent: "#40c0e0" } };
    saveCustomTheme(def);
    setThemeId(def.id);
    expect(root.dataset.themeId).toBe("custom-test");
    const style = document.getElementById("eth-theme-custom")!;
    expect(style.textContent).toContain('html[data-theme-id="custom-test"]');
    expect(style.textContent).toContain("--eth-color-accent: #40c0e0;");
    expect(JSON.parse(localStorage.getItem("eth-custom-themes")!)).toEqual([def]);

    const key = getThemeKey();
    previewTheme({ ...def, colors: { accent: "#ff8800" } });
    expect(root.dataset.themeId).toBe("preview");
    expect(themeColorValue("accent")).toBe("#ff8800");
    expect(getThemeKey()).not.toBe(key);
    expect(localStorage.getItem("eth-theme-id")).toBe("custom-test");
    previewTheme(null);
    expect(root.dataset.themeId).toBe("custom-test");

    // Deleting the theme on screen falls back to its scheme's built-in.
    deleteCustomTheme(def.id);
    expect(root.dataset.themeId).toBe("dark");
    expect(getThemes().some((d) => d.id === def.id)).toBe(false);
  });

  it("follows the system's light or dark with the theme picked for each", () => {
    const setSystem = mockSystem(false);
    setSchemeTheme("dark", "tide");
    setThemeMode("system");
    expect(root.dataset.themeId).toBe("tide");
    setSystem(true);
    expect(root.dataset.themeId).toBe("light");
    setSystem(false);
    setSchemeTheme("dark", "dusk");
    expect(root.dataset.themeId).toBe("dusk");
    // Choosing a theme stops following the system.
    setThemeId("obsidian");
    setSystem(true);
    expect(root.dataset.themeId).toBe("obsidian");
  });
});

describe("renderers follow the theme", () => {
  it("canvas colors (useThemeColor) update live, also while previewing", () => {
    const { result } = renderHook(() => useThemeColor("accent"));
    expect(result.current).toBe(builtinTheme("dark").color.accent);
    act(() => setThemeId("obsidian"));
    expect(result.current).toBe(builtinTheme("obsidian").color.accent);
    act(() => previewTheme({ id: "x", name: "x", base: "dark", colors: { accent: "#123456" } }));
    expect(result.current).toBe("#123456");
  });

  it("the arranger's WebGL palette and lane colors read the applied theme", () => {
    setThemeId("ember");
    const ember = builtinTheme("ember").color;
    const p = readPalette();
    expect(p.theme).toBe("dark");
    expect(p.bg.slice(0, 3).map((v) => v.toFixed(3))).toEqual(rgb(ember.bg).map((v) => v.toFixed(3)));
    expect(
      lanePalette(p)
        .accent.slice(0, 3)
        .map((v) => v.toFixed(3)),
    ).toEqual(rgb(ember.accent).map((v) => v.toFixed(3)));
  });
});
