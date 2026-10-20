// The mobile shell looks like the desktop editor: the desktop's tokens and kit surfaces,
// at touch size. No decoration of its own: gradients, glows, colored stripes or edges on cards,
// accent-filled buttons, its own blur.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const css = readFileSync(join(dirname(fileURLToPath(import.meta.url)), "mobile.css"), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

/** The declarations of the rules whose selector contains `selector` (ending with ` {`: whose selector ends with it). */
function declarations(selector: string): string[] {
  return [...css.matchAll(/([^{}]+)\{([^}]*)\}/g)]
    .filter((m) => (selector.endsWith(" {") ? `${m[1]!.trim()} {`.endsWith(selector) : m[1]!.includes(selector)))
    .map((m) => m[2]!);
}

describe("mobile.css: the desktop's look (#781)", () => {
  it("no gradients, glows, blur or decorative color of its own", () => {
    expect(css).not.toMatch(/gradient\(/);
    expect(css).not.toMatch(/glow/);
    expect(css).not.toMatch(/backdrop-filter|filter:/);
    // The track color is a swatch (as on the desktop), never a stripe or border on a card.
    for (const [, body] of css.matchAll(/\{([^}]*)\}/g)) {
      for (const decl of body!.split(";")) {
        if (/^\s*(border|box-shadow|outline)/.test(decl)) expect(decl).not.toContain("--eth-track-color");
      }
    }
  });

  it("transport buttons are the desktop's: no accent fill, play / record tinted only when on", () => {
    for (const body of [...declarations(".eth-mdock__play"), ...declarations(".eth-mdock__record")]) {
      expect(body).not.toMatch(/background:\s*var\(--eth-color-(accent|play|record)\)/);
      expect(body).not.toMatch(/--eth-color-text-inverse/);
    }
  });

  it("sheets and the clip action bar use the kit's dialog and popover surfaces", () => {
    const sheet = declarations(".eth-sheet {").join(";");
    expect(sheet).toContain("var(--dialog-bg)");
    expect(sheet).toContain("var(--dialog-shadow)");
    const bar = declarations(".eth-clip-actions {").join(";");
    expect(bar).toContain("var(--popover-bg)");
    expect(bar).toContain("var(--popover-radius)");
  });

  it("track and mixer toggles have no fill until on", () => {
    for (const sel of ['[data-layout^="phone"] .eth-arr-header__toggle {', '[data-layout="tablet"] .eth-arr-header__toggle {', ".eth-mstrip__toggle {"]) {
      const body = declarations(sel).join(";");
      expect(body, sel).not.toMatch(/background:\s*var\(--eth-color-bg-hover\)/);
    }
  });
});
