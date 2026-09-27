/**
 * Finds hard-coded design values (colors, px font sizes, px lengths) that should be tokens.
 * Used by `hardcoded.test.ts` (kit + app shell must be clean) and `report-hardcoded.ts`
 * (inventory of ui/src/features for the design sweep). Plain erasable TS so `node` runs it.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

export type FindingKind = "color" | "font-size" | "length";

export interface Finding {
  file: string;
  line: number;
  kind: FindingKind;
  text: string;
}

export interface ScanOptions {
  /** Also flag px lengths in CSS (strict mode for kit/app). Default false. */
  lengths?: boolean;
}

const COLOR = /#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|hwb|oklch|oklab|lab|lch)\(/g;
const CSS_FONT_SIZE = /font-size\s*:\s*[\d.]+px/g;
const TS_FONT_SIZE = /fontSize\s*:\s*["'`]?[\d.]+/g;
const CSS_LENGTH = /(?<![\w-])-?\d*\.?\d+px\b/g;

/** Blanks out comments (keeping newlines so line numbers stay right). */
function stripComments(src: string, css: boolean): string {
  const re = css ? /\/\*[\s\S]*?\*\//g : /\/\*[\s\S]*?\*\/|(?<![:"'`\\])\/\/[^\n]*/g;
  return src.replace(re, (m) => m.replace(/[^\n]/g, " "));
}

export function scanSource(file: string, src: string, opts: ScanOptions = {}): Finding[] {
  const css = file.endsWith(".css");
  const lines = stripComments(src, css).split("\n");
  const out: Finding[] = [];
  lines.forEach((l, i) => {
    const add = (kind: FindingKind, re: RegExp) => {
      for (const m of l.matchAll(re)) out.push({ file, line: i + 1, kind, text: m[0] });
    };
    add("color", COLOR);
    add("font-size", css ? CSS_FONT_SIZE : TS_FONT_SIZE);
    if (css && opts.lengths) add("length", CSS_LENGTH);
  });
  return out;
}

const SCANNED = /\.(css|ts|tsx)$/;
const SKIPPED = /\.test\.(ts|tsx)$/;

function walk(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) return walk(p);
    return SCANNED.test(name) && !SKIPPED.test(name) ? [p] : [];
  });
}

/** Scans every .css/.ts/.tsx file (tests excluded) under `dirs`; paths relative to `root`. */
export function scanDirs(root: string, dirs: string[], opts: ScanOptions = {}): Finding[] {
  return dirs.flatMap((d) =>
    walk(join(root, d)).flatMap((f) => scanSource(relative(root, f), readFileSync(f, "utf8"), opts)),
  );
}

export function formatFindings(findings: Finding[]): string {
  return findings.map((f) => `${f.file}:${f.line}  ${f.kind.padEnd(9)} ${f.text}`).join("\n");
}
