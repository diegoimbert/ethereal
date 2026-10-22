// @vitest-environment node
import { describe, expect, it } from "vitest";
import { BUILTIN_THEMES, builtinTheme } from "./builtinThemes";
import { contrast, parseColor } from "./color";
import {
  COLOR_KEYS,
  contrastFailures,
  CORE_KEYS,
  deriveColors,
  coreColors,
  parseThemeFile,
  resolveColors,
  serializeTheme,
  THEME_FORMAT,
  THEME_VERSION,
} from "./themeModel";
import { darkColors, lightColors } from "./tokens";

describe("color math", () => {
  it("parses the token syntaxes and computes WCAG contrast", () => {
    expect(parseColor("#fff")).toEqual({ r: 255, g: 255, b: 255, a: 1 });
    expect(parseColor("rgba(212, 143, 208, 0.14)")).toEqual({ r: 212, g: 143, b: 208, a: 0.14 });
    expect(parseColor("rgb(0 0 0 / 0.35)")).toEqual({ r: 0, g: 0, b: 0, a: 0.35 });
    expect(parseColor("transparent")?.a).toBe(0);
    expect(parseColor("red")).toBeNull();
    expect(parseColor("#12345")).toBeNull();
    expect(contrast("#000000", "#ffffff")).toBeCloseTo(21, 5);
    expect(contrast("#777777", "#777777")).toBeCloseTo(1, 5);
  });
});

describe("theme model", () => {
  it("has six built-in themes, mostly dark, with distinct accents", () => {
    expect(BUILTIN_THEMES).toHaveLength(6);
    expect(BUILTIN_THEMES.filter((t) => t.base === "dark").length).toBeGreaterThanOrEqual(4);
    const accents = new Set(BUILTIN_THEMES.filter((t) => t.id !== "light").map((t) => builtinTheme(t.id).color.accent));
    expect(accents.size).toBe(5);
  });

  it("dark and light resolve to their hand-tuned tokens exactly", () => {
    expect(builtinTheme("dark").color).toEqual(darkColors);
    expect(builtinTheme("light").color).toEqual(lightColors);
  });

  it("derives every token from the main colors", () => {
    for (const scheme of ["dark", "light"] as const) {
      const colors = deriveColors(coreColors({ base: scheme, colors: {} }), scheme);
      expect(Object.keys(colors).sort()).toEqual([...COLOR_KEYS].sort());
      for (const k of COLOR_KEYS) expect(parseColor(colors[k]), `${scheme} ${k}`).not.toBeNull();
    }
    // A new accent carries into the tokens made from it.
    const c = resolveColors({ base: "dark", colors: { accent: "#40c0e0" } });
    expect(c.selection).toBe("#40c0e0");
    expect(c.accentSubtle).toBe("rgba(64, 192, 224, 0.14)");
    // An explicit token wins over the derived one.
    expect(resolveColors({ base: "dark", colors: { accent: "#40c0e0", selection: "#ffffff" } }).selection).toBe("#ffffff");
  });

  it.each(BUILTIN_THEMES.map((t) => t.id))("built-in theme %s passes WCAG AA contrast", (id) => {
    const failures = contrastFailures(builtinTheme(id).color).map((f) => `${f.fg} on ${f.bg}: ${f.ratio.toFixed(2)} < ${f.min}`);
    expect(failures).toEqual([]);
  });
});

describe("theme files", () => {
  it("round-trips export → import", () => {
    const def = { name: "my theme", base: "dark" as const, colors: { ...coreColors({ base: "dark", colors: {} }), accent: "#40c0e0" } };
    const text = serializeTheme(def);
    const json = JSON.parse(text);
    expect(json.format).toBe(THEME_FORMAT);
    expect(json.version).toBe(THEME_VERSION);
    expect(Object.keys(json.colors)).toEqual(CORE_KEYS.filter((k) => k in def.colors));
    const back = parseThemeFile(text);
    expect(back).toEqual({ ok: true, theme: def, warnings: [] });
  });

  it("refuses what isn't a theme, a newer version and a missing base", () => {
    expect(parseThemeFile("{")).toEqual({ ok: false, error: { code: "not_json" } });
    expect(parseThemeFile('{"name":"x"}')).toEqual({ ok: false, error: { code: "not_theme" } });
    const file = (o: object) => JSON.stringify({ format: THEME_FORMAT, version: 1, name: "x", base: "dark", colors: {}, ...o });
    expect(parseThemeFile(file({ version: 2 }))).toEqual({ ok: false, error: { code: "newer_version", version: 2 } });
    expect(parseThemeFile(file({ version: "1" }))).toEqual({ ok: false, error: { code: "bad_version" } });
    expect(parseThemeFile(file({ base: "sepia" }))).toEqual({ ok: false, error: { code: "bad_base" } });
  });

  it("repairs unknown tokens, bad colors and a missing name, and says so", () => {
    const text = JSON.stringify({
      format: THEME_FORMAT,
      version: 1,
      base: "light",
      colors: { accent: "#123456", sparkle: "#fff", text: "blue", bg: 3 },
    });
    expect(parseThemeFile(text)).toEqual({
      ok: true,
      theme: { name: "", base: "light", colors: { accent: "#123456" } },
      warnings: [{ code: "no_name" }, { code: "unknown_tokens", tokens: ["sparkle"] }, { code: "invalid_colors", tokens: ["text", "bg"] }],
    });
  });
});
