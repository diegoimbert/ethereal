// @vitest-environment node
// #574: every shortcut hint comes from the keymap (`shortcutLabel` / `useShortcutLabel` /
// `<ShortcutHint>`), so a rebind or a preset switch shows everywhere. No hard-coded chords.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const src = fileURLToPath(new URL("../..", import.meta.url));
/** The kit gallery shows made-up menus (not the app's shortcuts). */
const SKIP = ["features/kit-gallery/"];

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sources(path);
    return /\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) ? [path] : [];
  });
}

/** A menu hint or a "(⌘K)"-style hint written out instead of read from the keymap. */
const HARD_CODED = [
  /shortcut:\s*`\$\{MOD_KEY\}/,
  /shortcut:\s*`[⇧⌥⌃]\$\{MOD_KEY\}/,
  /shortcut:\s*"(⌫|⌦|⌘|⇧|⌥|Ctrl\+|Space")/,
  /\(\$\{MOD_KEY\}[^)]*\)/,
  /\(\{MOD_KEY\}/,
];

describe("shortcut hints", () => {
  it("are never hard-coded: they follow the keymap", () => {
    const found: string[] = [];
    for (const file of sources(src)) {
      const rel = relative(src, file).replaceAll("\\", "/");
      if (SKIP.some((s) => rel.startsWith(s))) continue;
      readFileSync(file, "utf8")
        .split("\n")
        .forEach((line, i) => {
          if (HARD_CODED.some((re) => re.test(line))) found.push(`${rel}:${i + 1}: ${line.trim()}`);
        });
    }
    expect(found.join("\n"), "use shortcutLabel(actionId) / useShortcutLabel / <ShortcutHint>").toBe("");
  });
});
