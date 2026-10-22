// eth-allow-case-file: theme ids and token values.
/**
 * The built-in themes, as theme definitions (`themeModel.ts`). `dark` (the default) and
 * `light` spell out every token (`tokens.ts`); the others give their main colors and derive
 * the rest. Each passes the contrast checks (`themeModel.test.ts`). tokens.css declares them
 * all (`[data-theme-id="…"]`), so switching needs no script.
 */
import { resolveTheme, type ResolvedTheme, type ThemeDefinition } from "./themeModel";
import { darkColors, lightColors } from "./tokens";

export const BUILTIN_THEMES = [
  { id: "dark", name: "dark", base: "dark", colors: darkColors },
  {
    // Near black, neutral, with an electric cyan accent: the darkest, for dim rooms and OLED screens.
    id: "obsidian",
    name: "obsidian",
    base: "dark",
    colors: {
      bg: "#0a0a0c",
      bgPanel: "#101013",
      bgRaised: "#17171b",
      text: "#e4e4e9",
      textDim: "#8f8f9a",
      accent: "#3fc8e4",
      ok: "#6fd39a",
      warn: "#e6c46a",
      danger: "#f07c7c",
      info: "#7ea8f2",
    },
  },
  {
    // Warm brown charcoal with an amber accent.
    id: "ember",
    name: "ember",
    base: "dark",
    colors: {
      bg: "#191513",
      bgPanel: "#201b18",
      bgRaised: "#29231f",
      text: "#ece2d8",
      textDim: "#a6978a",
      accent: "#f0a35e",
      ok: "#97cf8a",
      warn: "#e9cb72",
      danger: "#ee8474",
      info: "#8fb6dc",
    },
  },
  {
    // Deep navy with a periwinkle accent.
    id: "tide",
    name: "tide",
    base: "dark",
    colors: {
      bg: "#0e1521",
      bgPanel: "#131b29",
      bgRaised: "#1a2435",
      text: "#d9e2f0",
      textDim: "#8a99b0",
      accent: "#8aa4ff",
      ok: "#6fd0a8",
      warn: "#e6c56f",
      danger: "#ef8686",
      info: "#6cc4ef",
    },
  },
  {
    // Soft: lighter slate surfaces and gentler text (less glare), with a rose accent.
    id: "dusk",
    name: "dusk",
    base: "dark",
    colors: {
      bg: "#24232b",
      bgPanel: "#2a2932",
      bgRaised: "#312f3a",
      text: "#d6d3de",
      textDim: "#a29eae",
      accent: "#ec9bb0",
      ok: "#8fd0a2",
      warn: "#ddc27c",
      danger: "#ec8e8e",
      info: "#91b6df",
    },
  },
  { id: "light", name: "light", base: "light", colors: lightColors },
] as const satisfies readonly ThemeDefinition[];

export type BuiltinThemeId = (typeof BUILTIN_THEMES)[number]["id"];

export const BUILTIN_THEME_IDS = BUILTIN_THEMES.map((t) => t.id) as BuiltinThemeId[];

/** The built-in theme a scheme falls back to (`<html data-theme>` alone, nothing remembered). */
export const SCHEME_DEFAULT_THEME = { dark: "dark", light: "light" } as const satisfies Record<string, BuiltinThemeId>;

const resolved = new Map<string, ResolvedTheme>(BUILTIN_THEMES.map((d) => [d.id, resolveTheme(d)]));

export function isBuiltinTheme(id: string): id is BuiltinThemeId {
  return resolved.has(id);
}

/** A built-in theme with every token resolved. */
export function builtinTheme(id: BuiltinThemeId): ResolvedTheme {
  return resolved.get(id)!;
}
