// @vitest-environment node
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { formatFindings, scanDirs, scanSource } from "./hardcoded";

const root = fileURLToPath(new URL("../..", import.meta.url)); // ui/
const kinds = (file: string, src: string) => scanSource(file, src, { lengths: true }).map((f) => `${f.kind}:${f.text}`);

describe("no hard-coded design values", () => {
  it("kit, app shell, kit gallery and base.css read only tokens", () => {
    const findings = scanDirs(root, ["src/kit", "src/app", "src/features/kit-gallery", "src/theme/base.css"], {
      lengths: true,
    });
    expect(formatFindings(findings), "use tokens from ui/src/theme/tokens.ts instead").toBe("");
  });

  it("detects CSS colors, named colors, font sizes and lengths once each", () => {
    const css = [
      ".a { color: #fff; background: rgba(0, 0, 0, 0.5); font-size: 11px; width: 4px; } /* #abc 3px */",
      ".b { border: 1px solid black; margin: 1.5rem; padding: 2em; }",
      ".eth-red, .x { color: var(--eth-color-text); --my-white: 0; }",
    ].join("\n");
    expect(kinds("x.css", css)).toEqual([
      "color:#fff",
      "color:rgba(",
      "font-size:font-size: 11px",
      "length:4px",
      "named-color:black",
      "length:1px",
      "length:1.5rem",
      "length:2em",
    ]);
  });

  it("detects TS colors, size props, numeric inline styles and px strings", () => {
    const ts = [
      'const s = { color: "#ff0000", fontSize: 12 }; // #123456',
      'const ok = "var(--eth-color-text)"; const c = "white";',
      '<Knob size={32} /> <Fader height={120} /> <Knob size="sm" /> <Knob size={n} />',
      '<div style={{ left: 0, width: 40 }} /> <div style={{ top: "3px" }} /> <div style={{ height }} />',
    ].join("\n");
    expect(kinds("x.tsx", ts)).toEqual([
      "color:#ff0000",
      "font-size:fontSize: 12",
      "named-color:\"white\"",
      "size-prop:size={32}",
      "size-prop:height={120}",
      "inline-style:style={{ left: 0, width: 40",
      "inline-style:style={{ top: \"3px\"",
    ]);
  });
});
