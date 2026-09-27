// @vitest-environment node
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { formatFindings, scanDirs, scanSource } from "./hardcoded";

const root = fileURLToPath(new URL("../..", import.meta.url)); // ui/
const kinds = (file: string, src: string) => scanSource(file, src, { lengths: true }).map((f) => `${f.kind}:${f.text}`);

describe("no hard-coded design values", () => {
  it("kit, app shell, features, timeline and base.css read only tokens", () => {
    const findings = scanDirs(root, ["src/kit", "src/app", "src/features", "src/timeline", "src/theme/base.css"], {
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

  it("skips lines marked as an allowed exception (and the line after the marker)", () => {
    const css = [
      "/* eth-allow-hardcoded: container queries cannot use var() */",
      "@container x (max-width: 240px) {",
      "  .a { width: 4px; } /* eth-allow-hardcoded: reason */",
      "  .b { width: 5px; }",
      "}",
    ].join("\n");
    expect(kinds("x.css", css)).toEqual(["length:5px"]);
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

describe("token references", () => {
  it("every var(--eth-*) in ui/src is a token or a property the code declares (catches typos)", () => {
    const files: string[] = [];
    const walk = (d: string) => {
      for (const n of readdirSync(d)) {
        const p = join(d, n);
        if (statSync(p).isDirectory()) walk(p);
        else if (/\.(css|ts|tsx)$/.test(n)) files.push(p);
      }
    };
    walk(join(root, "src"));
    const tokens = new Set([...readFileSync(join(root, "src/theme/tokens.css"), "utf8").matchAll(/(--[a-z0-9-]+)\s*:/g)].map((m) => m[1]));
    const declared = new Set<string>();
    const used: Array<[string, string]> = [];
    for (const f of files) {
      const src = readFileSync(f, "utf8");
      for (const m of src.matchAll(/(--[a-z0-9_-]+)\s*:|["'`](--[a-z0-9_-]+)["'`]/g)) declared.add((m[1] ?? m[2])!);
      // Complete names only (template prefixes like `--eth-track-${i}` end with "-").
      for (const m of src.matchAll(/var\(\s*(--eth-[a-z0-9_-]*[a-z0-9_])[,)\s]/g)) used.push([f, m[1]!]);
    }
    const unknown = used.filter(([, n]) => !tokens.has(n) && !declared.has(n)).map(([f, n]) => `${f}: ${n}`);
    expect(unknown).toEqual([]);
  });
});
