/**
 * `virtual:eth-locale/<code>` → `ui/src/i18n/locales/<code>.json`, compiled to plain strings /
 * ICU ASTs at build time (docs/LOCALIZATION.md). Wired in ui/vite.config.ts and
 * apps/web/vite.config.ts. In production builds the ICU parser is swapped for its no-parser
 * build: the app never parses a message at runtime.
 */
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { Plugin } from "vite";
import { compileCatalog } from "./compile.ts";

const PREFIX = "virtual:eth-locale/";
const RESOLVED = "\0eth-locale:";
const LOCALES_DIR = fileURLToPath(new URL("../locales/", import.meta.url));

export function i18nCatalogPlugin(): Plugin {
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
      const file = `${LOCALES_DIR}${id.slice(RESOLVED.length)}.json`;
      this.addWatchFile(file);
      return `export default ${JSON.stringify(compileCatalog(readFileSync(file, "utf8")))};`;
    },
  };
}
