/**
 * `virtual:eth-locale/<code>` → `ui/src/i18n/locales/<code>.json`, compiled to plain strings /
 * ICU ASTs at build time (docs/LOCALIZATION.md). Wired in ui/vite.config.ts and
 * apps/web/vite.config.ts. In production builds the ICU parser is swapped for its no-parser
 * build: the app never parses a message at runtime.
 *
 * `extraDirs`: an extension's own catalogs (`<dir>/<code>.json`, docs/EXTENSIONS.md), merged
 * in. Their keys must be new: a key the app's catalog has already is a build error.
 */
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import type { Plugin } from "vite";
import { compileCatalog } from "./compile.ts";

const PREFIX = "virtual:eth-locale/";
const RESOLVED = "\0eth-locale:";
const LOCALES_DIR = fileURLToPath(new URL("../locales/", import.meta.url));

export interface I18nCatalogOptions {
  /** Folders of extension catalogs merged into the app's (`<dir>/<code>.json`, when present). */
  extraDirs?: ReadonlyArray<string>;
}

/** The app's catalog `code` with the extensions' merged in (their keys must be new). */
export function mergedCatalogSource(code: string, extraDirs: ReadonlyArray<string> = []): { json: string; files: string[] } {
  const file = `${LOCALES_DIR}${code}.json`;
  const files = [file];
  const merged: Record<string, string> = JSON.parse(readFileSync(file, "utf8"));
  for (const dir of extraDirs) {
    const extra = join(dir, `${code}.json`);
    if (!existsSync(extra)) continue;
    files.push(extra);
    for (const [key, value] of Object.entries(JSON.parse(readFileSync(extra, "utf8")) as Record<string, string>)) {
      if (Object.hasOwn(merged, key)) throw new Error(`${extra}: "${key}" is already in the app's ${code} catalog`);
      merged[key] = value;
    }
  }
  return { json: JSON.stringify(merged), files };
}

export function i18nCatalogPlugin(options: I18nCatalogOptions = {}): Plugin {
  return {
    name: "eth-i18n-catalogs",
    enforce: "pre",
    config(_, { command }) {
      if (command !== "build") return;
      return {
        resolve: {
          alias: [{ find: /^@formatjs\/icu-messageformat-parser$/, replacement: "@formatjs/icu-messageformat-parser/no-parser.js" }],
        },
      };
    },
    resolveId(id) {
      if (!id.startsWith(PREFIX)) return null;
      const code = id.slice(PREFIX.length);
      if (!/^[a-z]{2,3}(-[A-Za-z0-9]+)*$/.test(code)) return null;
      return RESOLVED + code;
    },
    load(id) {
      if (!id.startsWith(RESOLVED)) return null;
      const { json, files } = mergedCatalogSource(id.slice(RESOLVED.length), options.extraDirs);
      for (const f of files) this.addWatchFile(f);
      return `export default ${JSON.stringify(compileCatalog(json))};`;
    },
  };
}
