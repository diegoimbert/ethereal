/**
 * Keeps `tokens.css` in sync with `tokens.ts`:
 * - `tokensCssPlugin()` (wired in ui/vite.config.ts) regenerates it at startup and whenever
 *   tokens.ts / css.ts change, so the dev server restyles live on save.
 * - `writeTokensCss()` is used by `gen-css.mjs` (`just gen-tokens`).
 * Both load the TS sources through Vite's module runner (no Node type-stripping needed).
 */
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { runnerImport, type Plugin } from "vite";

const dir = fileURLToPath(new URL(".", import.meta.url));
const CSS_TS = `${dir}css.ts`;
const OUT = `${dir}tokens.css`;
const SOURCES = [`${dir}tokens.ts`, CSS_TS];

/** Regenerates tokens.css; returns true when the file changed. */
export async function writeTokensCss(): Promise<boolean> {
  const { module } = await runnerImport<typeof import("./css")>(CSS_TS);
  const next = module.renderTokensCss();
  let prev = "";
  try {
    prev = readFileSync(OUT, "utf8");
  } catch {
    // first run
  }
  if (prev === next) return false;
  writeFileSync(OUT, next);
  return true;
}

export function tokensCssPlugin(): Plugin {
  const regen = async (log: (msg: string) => void) => {
    try {
      if (await writeTokensCss()) log("tokens.css regenerated from tokens.ts");
    } catch (e) {
      log(`tokens.css NOT regenerated: ${e instanceof Error ? e.message : String(e)}`);
    }
  };
  return {
    name: "ethereal:tokens-css",
    // Not under vitest: tokens.test.ts must see the committed file to catch a stale one.
    apply: () => !process.env.VITEST,
    async buildStart() {
      await regen((m) => this.info(m));
    },
    configureServer(server) {
      server.watcher.add(SOURCES);
      server.watcher.on("change", (file) => {
        if (SOURCES.includes(file)) void regen((m) => server.config.logger.info(m, { timestamp: true }));
      });
    },
  };
}
