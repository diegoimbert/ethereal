// @vitest-environment node
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { formatFindings, scanDirs, scanSource } from "./hardcoded";

const root = fileURLToPath(new URL("../..", import.meta.url)); // ui/

describe("no hard-coded design values", () => {
  it("kit, app shell and kit gallery read only tokens (no literal colors, px font sizes or px lengths)", () => {
    const findings = scanDirs(root, ["src/kit", "src/app", "src/features/kit-gallery"], { lengths: true });
    expect(formatFindings(findings), "use tokens from ui/src/theme/tokens.ts instead").toBe("");
  });

  it("detects the patterns it is meant to catch", () => {
    const css = ".a { color: #fff; background: rgba(0, 0, 0, 0.5); font-size: 11px; width: 4px; } /* #abc 3px */";
    expect(scanSource("x.css", css, { lengths: true }).map((f) => `${f.kind}:${f.text}`)).toEqual([
      "color:#fff",
      "color:rgba(",
      "font-size:font-size: 11px",
      "length:11px",
      "length:4px",
    ]);
    const ts = 'const s = { color: "#ff0000", fontSize: 12 }; // #123456\nconst ok = "var(--eth-color-text)";';
    expect(scanSource("x.tsx", ts).map((f) => f.kind)).toEqual(["color", "font-size"]);
  });
});
