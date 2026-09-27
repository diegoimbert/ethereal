// @vitest-environment node
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { renderTokensCss } from "./css";
import { trackColor } from "./index";
import { componentEntries, themeEntries, THEME_NAMES, themes, TRACK_COLORS } from "./tokens";

// Read from disk: vitest stubs CSS imports (even `?raw`) when `test.css` is off.
const cssUrl = new URL("./tokens.css", import.meta.url);
const css = readFileSync(cssUrl, "utf8");

/** Parses `selector { --a: b; }` blocks of the generated CSS. */
function blocks(text: string): Map<string, Map<string, string>> {
  const out = new Map<string, Map<string, string>>();
  for (const m of text.matchAll(/([^{}]+)\{([^}]*)\}/g)) {
    const sel = m[1]!.replace(/\/\*[\s\S]*?\*\//g, "").replace(/\s+/g, " ").trim();
    const decls = new Map<string, string>();
    for (const d of m[2]!.matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/g)) decls.set(d[1]!, d[2]!.trim());
    out.set(sel, decls);
  }
  return out;
}

describe("design tokens", () => {
  it("tokens.css is generated from tokens.ts (run `just gen-tokens`)", () => {
    expect(css).toBe(renderTokensCss());
  });

  it("every token of every theme is declared in tokens.css with the same value", () => {
    const b = blocks(css);
    const merged = (theme: string) => {
      const m = new Map<string, string>();
      for (const [sel, decls] of b) {
        const applies = sel.split(",").some((s) => {
          const t = s.trim();
          return t === `[data-theme="${theme}"]` || t === "[data-theme]" || (t === ":root" && theme === "dark");
        });
        const shared = sel === ":root";
        if (applies || shared) for (const [k, v] of decls) m.set(k, v);
      }
      return m;
    };
    for (const theme of THEME_NAMES) {
      const declared = merged(theme);
      for (const [name, value] of themeEntries(theme)) expect(declared.get(name), `${theme} ${name}`).toBe(value);
    }
  });

  it("all themes define the same color and shadow keys", () => {
    const [first, ...rest] = THEME_NAMES.map((t) => themes[t]);
    for (const t of rest) {
      expect(Object.keys(t.color).sort()).toEqual(Object.keys(first!.color).sort());
      expect(Object.keys(t.shadow).sort()).toEqual(Object.keys(first!.shadow).sort());
    }
  });

  it("component tokens only reference declared global tokens", () => {
    const names = new Set(themeEntries().map(([k]) => k));
    for (const [name, value] of componentEntries()) {
      for (const ref of value.matchAll(/var\((--[a-z0-9-]+)/g)) expect(names.has(ref[1]!), `${name} → ${ref[1]}`).toBe(true);
    }
  });

  it("trackColor wraps around the palette", () => {
    expect(trackColor(0)).toBe(TRACK_COLORS[0]);
    expect(trackColor(TRACK_COLORS.length + 1)).toBe(TRACK_COLORS[1]);
    expect(trackColor(-1)).toBe(TRACK_COLORS[TRACK_COLORS.length - 1]);
  });
});
