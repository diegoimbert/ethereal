/**
 * Catalog checks (run by catalogs.test.ts and `pnpm i18n check`) and the generated argument
 * types (`pnpm i18n gen`). docs/LOCALIZATION.md lists the rules.
 */
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import type { MessageFormatElement } from "@formatjs/icu-messageformat-parser";
import { parseMessage } from "./compile.ts";
import { PROPER_NOUNS } from "./text.ts";

export { PROPER_NOUNS };

export const I18N_DIR = fileURLToPath(new URL("..", import.meta.url));
export const LOCALES_DIR = join(I18N_DIR, "locales");
export const UI_DIR = join(I18N_DIR, "..", "..");
export const REPO_DIR = join(UI_DIR, "..");
export const ARGS_FILE = join(I18N_DIR, "args.gen.ts");
export const SOURCE_LOCALE = "en";

export type Catalog = Record<string, string>;

export function readCatalog(code: string): Catalog {
  return JSON.parse(readFileSync(join(LOCALES_DIR, `${code}.json`), "utf8")) as Catalog;
}

/** The locale codes with a catalog file. */
export function catalogCodes(): string[] {
  return readdirSync(LOCALES_DIR)
    .filter((f) => f.endsWith(".json"))
    .map((f) => f.slice(0, -5))
    .sort();
}

/** Argument kinds, as the generated types spell them. */
type ArgKind = "number" | "string | number" | "Date | number" | "string" | "tag";

function collectArgs(ast: MessageFormatElement[], out: Map<string, ArgKind>): void {
  const put = (name: string, kind: ArgKind) => {
    const prev = out.get(name);
    // A plain `{x}` next to `{x, plural…}` is still a number.
    if (!prev || prev === "string | number") out.set(name, kind);
  };
  for (const el of ast) {
    switch (el.type) {
      case 1:
        put(el.value, "string | number");
        break;
      case 2:
        put(el.value, "number");
        break;
      case 3:
      case 4:
        put(el.value, "Date | number");
        break;
      case 5:
        put(el.value, "string");
        for (const o of Object.values(el.options)) collectArgs(o.value, out);
        break;
      case 6:
        put(el.value, "number");
        for (const o of Object.values(el.options)) collectArgs(o.value, out);
        break;
      case 8:
        put(el.value, "tag");
        collectArgs(el.children, out);
        break;
    }
  }
}

export function argsOf(message: string): Map<string, ArgKind> {
  const out = new Map<string, ArgKind>();
  collectArgs(parseMessage(message), out);
  return out;
}

/** The literal text of a message (what a reader sees, arguments left out). */
export function literalText(ast: MessageFormatElement[]): string {
  let s = "";
  for (const el of ast) {
    if (el.type === 0) s += el.value;
    else if (el.type === 8) s += ` ${literalText(el.children)} `;
    else if (el.type === 5 || el.type === 6) for (const o of Object.values(el.options)) s += ` ${literalText(o.value)} `;
    else s += " ";
  }
  return s;
}

const PROPER_RE = new RegExp(`(?<![\\p{L}\\p{N}])(${PROPER_NOUNS.join("|")})(?![\\p{L}\\p{N}])`, "gu");

/** Capitals left once the proper nouns are taken out (the lowercase rule). */
export function capitalsIn(text: string): string[] {
  return [...text.replace(PROPER_RE, "").matchAll(/\p{Lu}\p{L}*/gu)].map((m) => m[0]);
}

/** A translation may format a plain `{x}` (string or number) as a number or a select (`{x, plural…}`). */
function compatible(source: ArgKind, translation: ArgKind): boolean {
  if (source === translation) return true;
  const loose = (k: ArgKind) => k === "string | number" || k === "number" || k === "string";
  return (source === "string | number" || translation === "string | number") && loose(source) && loose(translation);
}

/** Every rule a catalog breaks, as human-readable lines (empty: all good). */
export function checkCatalogs(catalogs: Record<string, Catalog> = Object.fromEntries(catalogCodes().map((c) => [c, readCatalog(c)]))): string[] {
  const errors: string[] = [];
  const source = catalogs[SOURCE_LOCALE];
  if (!source) return [`no ${SOURCE_LOCALE}.json`];
  const sourceArgs = new Map<string, Map<string, ArgKind>>();
  for (const [code, cat] of Object.entries(catalogs)) {
    for (const [key, msg] of Object.entries(cat)) {
      if (!/^[a-z0-9_]+(\.[a-z0-9_]+)+$/.test(key)) errors.push(`${code}: key "${key}" isn't <area>.<name> in lowercase snake_case`);
      if (typeof msg !== "string") {
        errors.push(`${code}: "${key}" isn't a string`);
        continue;
      }
      let ast: MessageFormatElement[];
      try {
        ast = parseMessage(msg);
      } catch (e) {
        errors.push(`${code}: "${key}" has bad ICU syntax: ${e instanceof Error ? e.message : String(e)}`);
        continue;
      }
      const text = literalText(ast);
      const caps = capitalsIn(text);
      if (caps.length) errors.push(`${code}: "${key}" has capitals (${caps.join(", ")}): app text is lowercase (#524); proper nouns go in PROPER_NOUNS`);
      if (msg !== msg.trim()) errors.push(`${code}: "${key}" starts or ends with a space`);
      if (code === "fr") {
        if (text.includes("'")) errors.push(`fr: "${key}" uses ' : write the typographic apostrophe ’`);
        if (/ [:;!?»%]/.test(text) || /« /.test(text))
          errors.push(`fr: "${key}" needs a no-break space (U+00A0, or U+202F before ; ! ?) before : ; ! ? % and inside « »`);
      }
      const args = new Map<string, ArgKind>();
      collectArgs(ast, args);
      if (code === SOURCE_LOCALE) sourceArgs.set(key, args);
    }
  }
  for (const [code, cat] of Object.entries(catalogs)) {
    if (code === SOURCE_LOCALE) continue;
    for (const key of Object.keys(source)) if (!(key in cat)) errors.push(`${code}: missing "${key}"`);
    for (const key of Object.keys(cat)) if (!(key in source)) errors.push(`${code}: "${key}" isn't in ${SOURCE_LOCALE}.json (removed or misspelled?)`);
    for (const [key, msg] of Object.entries(cat)) {
      const want = sourceArgs.get(key);
      if (!want) continue;
      let got: Map<string, ArgKind>;
      try {
        got = argsOf(msg);
      } catch {
        continue; // reported above
      }
      const fmt = (m: Map<string, ArgKind>) =>
        [...m]
          .map(([n, k]) => `${n}:${k}`)
          .sort()
          .join(", ");
      const same = want.size === got.size && [...want].every(([n, k]) => got.has(n) && compatible(k, got.get(n)!));
      if (!same) errors.push(`${code}: "${key}" takes {${fmt(got)}}, ${SOURCE_LOCALE} takes {${fmt(want)}}`);
    }
  }
  return errors;
}

/** Source files (non-test TS/TSX) under `dirs` (files are taken as is): by default everything that may name a key. */
export function sourceFiles(dirs = [join(UI_DIR, "src"), join(REPO_DIR, "apps", "web", "src")], extension = /\.(ts|tsx)$/): string[] {
  const out: string[] = [];
  const walk = (dir: string) => {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
      const p = join(dir, e.name);
      if (e.isDirectory()) {
        if (e.name !== "node_modules" && e.name !== "locales" && e.name !== "tooling" && e.name !== "tests") walk(p);
      } else if (extension.test(e.name) && !/\.test\.tsx?$/.test(e.name) && e.name !== "tests.rs" && p !== ARGS_FILE) out.push(p);
    }
  };
  for (const d of dirs) {
    if (statSync(d).isDirectory()) walk(d);
    else out.push(d);
  }
  return out;
}

/**
 * Where keys are named: the UI's sources, and the Rust that sends message ids (the
 * controller's undo labels, notices and errors, the hosts' notices, the desktop shell).
 */
export function keySourceFiles(): string[] {
  return [
    ...sourceFiles(),
    ...sourceFiles(
      ["ether-controller", "ether-native", "ether-wasm"]
        .map((c) => join(REPO_DIR, "crates", c, "src"))
        .concat(join(REPO_DIR, "apps", "desktop", "src-tauri", "src")),
      /\.rs$/,
    ),
  ];
}

/** Keys no source file names as a string literal. Keys are never built at runtime. */
export function unusedKeys(keys: string[], files = keySourceFiles()): string[] {
  const used = new Set<string>();
  const literal = /["'`]([a-z0-9_]+(?:\.[a-z0-9_]+)+)["'`]/g;
  for (const f of files) for (const m of readFileSync(f, "utf8").matchAll(literal)) used.add(m[1]!);
  return keys.filter((k) => !used.has(k));
}

/** `args.gen.ts`: the arguments of every English message that takes some. */
export function renderArgsFile(source: Catalog = readCatalog(SOURCE_LOCALE)): string {
  const lines = [
    "// Generated by `pnpm i18n gen` from locales/en.json (docs/LOCALIZATION.md): do not edit.",
    'import type { Tag } from "./types";',
    "",
    "export interface MessageArgs {",
  ];
  let usesTag = false;
  for (const key of Object.keys(source).sort()) {
    const args = argsOf(source[key]!);
    if (args.size === 0) continue;
    const fields = [...args]
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([name, kind]) => {
        if (kind === "tag") usesTag = true;
        return `${/^[A-Za-z_$][\w$]*$/.test(name) ? name : JSON.stringify(name)}: ${kind === "tag" ? "Tag" : kind}`;
      });
    lines.push(`  ${JSON.stringify(key)}: { ${fields.join("; ")} };`);
  }
  lines.push("}", "");
  if (!usesTag) lines.splice(1, 2);
  return lines.join("\n");
}
