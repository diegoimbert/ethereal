/** Renders `tokens.ts` into the CSS text of `tokens.css` (pure; used by gen-css.ts and tests). */
import {
  componentEntries,
  DEFAULT_THEME,
  sharedEntries,
  THEME_NAMES,
  themeOnlyEntries,
  themes,
  type ThemeName,
} from "./tokens.ts";

const HEADER = `/*
 * GENERATED from ui/src/theme/tokens.ts by the Vite plugin / \`just gen-tokens\`. Do not edit:
 * change tokens.ts and regenerate (tokens.test.ts fails when this file is stale).
 */`;

function block(selectors: string[], entries: Array<[string, string]>, extra: string[] = []): string {
  const lines = [...extra, ...entries.map(([k, v]) => `${k}: ${v};`)].map((l) => `  ${l}`);
  return `${selectors.join(",\n")} {\n${lines.join("\n")}\n}`;
}

function themeSelectors(theme: ThemeName): string[] {
  const attr = `[data-theme="${theme}"]`;
  return theme === DEFAULT_THEME ? [":root", attr] : [attr];
}

export function renderTokensCss(): string {
  const parts = [
    HEADER,
    `/* Theme-independent tokens. */\n${block([":root"], sharedEntries())}`,
    ...THEME_NAMES.map(
      (t) =>
        `/* Theme: ${t}${t === DEFAULT_THEME ? " (default)" : ""}. */\n` +
        block(themeSelectors(t), themeOnlyEntries(t), [`color-scheme: ${themes[t].colorScheme};`]),
    ),
    `/* Component tokens (re-declared on themed subtrees so they follow the theme). */\n` +
      block([":root", "[data-theme]"], componentEntries()),
  ];
  return parts.join("\n\n") + "\n";
}
