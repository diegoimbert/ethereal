/**
 * Finds hard-coded design values that should be tokens: literal colors (hex, rgb()/hsl()/…,
 * named colors), px font sizes, px/em/rem lengths, numeric size props (`height={120}`) and
 * numeric inline styles (`style={{ width: 40 }}`). Each source span is reported once.
 * Used by `hardcoded.test.ts` (kit + app shell + gallery + base.css must be clean) and
 * `report-hardcoded.mjs` (inventory of ui/src/features for the design sweep).
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

export type FindingKind = "color" | "named-color" | "font-size" | "length" | "size-prop" | "inline-style";

export interface Finding {
  file: string;
  line: number;
  kind: FindingKind;
  text: string;
}

export interface ScanOptions {
  /** Also flag literal lengths (strict mode for kit/app). Default false. */
  lengths?: boolean;
}

const NAMED =
  "white|black|red|green|blue|yellow|orange|purple|pink|gray|grey|silver|maroon|navy|teal|olive|lime|aqua|cyan|magenta|fuchsia|brown|gold|violet|indigo|crimson|coral|salmon|tomato";
const COLOR = /#[0-9a-fA-F]{3,8}\b|\b(?:rgba?|hsla?|hwb|oklch|oklab|lab|lch)\(/g;
// CSS: a named color as a declaration value word (not part of a class/var name).
const CSS_NAMED = new RegExp(`(?<=:[^;{}]*?)(?<![\\w.#-])(?:${NAMED})(?![\\w-])`, "gi");
// TS: a named color as a whole string literal.
const TS_NAMED = new RegExp(`(["'\`])(?:${NAMED})\\1`, "g");
const CSS_FONT_SIZE = /font-size\s*:\s*[\d.]+(?:px|r?em)\b/g;
const TS_FONT_SIZE = /fontSize\s*:\s*["'`]?[\d.]+/g;
const CSS_LENGTH = /(?<![\w-])-?\d*\.?\d+(?:px|r?em)\b/g;
const TS_LENGTH = /(["'`])-?\d*\.?\d+(?:px|r?em)\1/g;
const SIZE_PROP = /\b(?:size|height|width|diameter|thickness|radius|gap|fontSize)=\{\s*-?[\d.]+\s*\}/g;
const INLINE_STYLE = /style=\{\{[^}]*?:\s*(?:-?(?:[1-9][\d.]*|0?\.\d+)(?=\s*[,}])|["'`][\d.]+(?:px|r?em)["'`])/g;

/** Blanks out comments (keeping newlines so line numbers stay right). */
function stripComments(src: string, css: boolean): string {
  const re = css ? /\/\*[\s\S]*?\*\//g : /\/\*[\s\S]*?\*\/|(?<![:"'`\\])\/\/[^\n]*/g;
  return src.replace(re, (m) => m.replace(/[^\n]/g, " "));
}

/**
 * Marks an allowed exception: findings on the line with this marker (in a comment), or on
 * the line after a comment line holding only the marker, are not reported. Always give the reason next to it, e.g.
 * `/* eth-allow-hardcoded: container query conditions cannot use var() *\/`.
 */
export const ALLOW_MARKER = "eth-allow-hardcoded";

export function scanSource(file: string, src: string, opts: ScanOptions = {}): Finding[] {
  const css = file.endsWith(".css");
  const raw = src.split("\n");
  const markerOnly = (l: string | undefined) => !!l && /^\s*(\/\*|\/\/)/.test(l) && l.includes(ALLOW_MARKER);
  const allowed = (i: number) => raw[i]!.includes(ALLOW_MARKER) || markerOnly(raw[i - 1]);
  const lines = stripComments(src, css).split("\n");
  const out: Finding[] = [];
  lines.forEach((l, i) => {
    if (allowed(i)) return;
    const taken: Array<[number, number]> = [];
    // Earlier (more specific) patterns win; a span already reported is not counted again.
    const add = (kind: FindingKind, re: RegExp) => {
      for (const m of l.matchAll(re)) {
        const a = m.index;
        const b = a + m[0].length;
        if (taken.some(([x, y]) => a < y && b > x)) continue;
        taken.push([a, b]);
        out.push({ file, line: i + 1, kind, text: m[0] });
      }
    };
    add("color", COLOR);
    if (css) {
      add("font-size", CSS_FONT_SIZE);
      add("named-color", CSS_NAMED);
      if (opts.lengths) add("length", CSS_LENGTH);
    } else {
      add("font-size", TS_FONT_SIZE);
      add("named-color", TS_NAMED);
      add("size-prop", SIZE_PROP);
      add("inline-style", INLINE_STYLE);
      if (opts.lengths) add("length", TS_LENGTH);
    }
  });
  return out;
}

const SCANNED = /\.(css|ts|tsx)$/;
const SKIPPED = /\.test\.(ts|tsx)$/;

function walk(path: string): string[] {
  if (!statSync(path).isDirectory()) return [path];
  return readdirSync(path).flatMap((name) => {
    const p = join(path, name);
    if (statSync(p).isDirectory()) return walk(p);
    return SCANNED.test(name) && !SKIPPED.test(name) ? [p] : [];
  });
}

/** Scans .css/.ts/.tsx files (tests excluded) under `paths` (dirs or files), relative to `root`. */
export function scanDirs(root: string, paths: string[], opts: ScanOptions = {}): Finding[] {
  return paths.flatMap((d) =>
    walk(join(root, d)).flatMap((f) => scanSource(relative(root, f), readFileSync(f, "utf8"), opts)),
  );
}

export function formatFindings(findings: Finding[]): string {
  return findings.map((f) => `${f.file}:${f.line}  ${f.kind.padEnd(12)} ${f.text}`).join("\n");
}

/** Findings grouped for the sweep report. */
export function summarize(findings: Finding[]): string {
  const byKind = new Map<string, number>();
  const byFile = new Map<string, number>();
  for (const f of findings) {
    byKind.set(f.kind, (byKind.get(f.kind) ?? 0) + 1);
    byFile.set(f.file, (byFile.get(f.file) ?? 0) + 1);
  }
  const lines = [`${findings.length} hard-coded values in ${byFile.size} files`];
  for (const [k, n] of byKind) lines.push(`  ${k}: ${n}`);
  lines.push("", "by file:");
  for (const [f, n] of [...byFile].sort((a, b) => b[1] - a[1])) lines.push(`  ${String(n).padStart(4)}  ${f}`);
  return lines.join("\n");
}
