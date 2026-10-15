// Localization tooling (docs/LOCALIZATION.md). Run from the repo root as `pnpm i18n <cmd>`:
//   gen                    regenerate src/i18n/engineText.gen.ts from engine-text.json (and the
//                          `engine.*` keys of en.json: new ones in English, gone ones dropped
//                          everywhere), then src/i18n/args.gen.ts from locales/en.json
//   check                  the catalog checks (also run by vitest): syntax, missing/extra keys,
//                          arguments, lowercase, French typography, unused keys, stale args.gen.ts
//   report [--all] [dir]   hard-coded UI text per file and the share already localized
//                          (developer-only paths, DEV_ONLY_PATHS in src/i18n/localized.ts, left out)
//   po <lang>              write <lang>.po (msgctxt = key) for a translation tool
//   po-import <lang> <po>  merge a translated .po back into locales/<lang>.json
// Works on Node >= 20: the TS tooling is loaded through Vite's module runner.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { runnerImport } from "vite";

const tooling = (name) => runnerImport(new URL(`../src/i18n/tooling/${name}.ts`, import.meta.url).pathname).then((r) => r.module);
const check = await tooling("check");
const [cmd, ...args] = process.argv.slice(2);

if (cmd === "gen") {
  const engine = await tooling("engineText");
  const entries = engine.engineEntries();
  for (const code of check.catalogCodes()) {
    const current = check.readCatalog(code);
    const synced = engine.syncEngineKeys(current, entries, code === check.SOURCE_LOCALE);
    if (JSON.stringify(synced) !== JSON.stringify(current)) {
      writeFileSync(join(check.LOCALES_DIR, `${code}.json`), `${JSON.stringify(synced, null, 2)}\n`);
      console.log(`locales/${code}.json: engine.* keys updated`);
    }
  }
  const engineFile = engine.renderEngineTextFile(entries);
  if (!existsSync(engine.ENGINE_TEXT_FILE) || readFileSync(engine.ENGINE_TEXT_FILE, "utf8") !== engineFile) {
    writeFileSync(engine.ENGINE_TEXT_FILE, engineFile);
    console.log("engineText.gen.ts regenerated");
  }
  const next = check.renderArgsFile();
  const prev = readFileSync(check.ARGS_FILE, "utf8");
  if (prev !== next) writeFileSync(check.ARGS_FILE, next);
  console.log(prev === next ? "args.gen.ts already up to date" : "args.gen.ts regenerated");
} else if (cmd === "check") {
  const errors = check.checkCatalogs();
  const keys = Object.keys(check.readCatalog(check.SOURCE_LOCALE));
  for (const k of check.unusedKeys(keys)) errors.push(`unused key "${k}" (no source file names it)`);
  if (readFileSync(check.ARGS_FILE, "utf8") !== check.renderArgsFile()) errors.push("args.gen.ts is stale: run `pnpm i18n gen`");
  const engine = await tooling("engineText");
  errors.push(...engine.engineTextErrors());
  console.log(errors.length ? errors.join("\n") : "catalogs ok");
  process.exitCode = errors.length ? 1 : 0;
} else if (cmd === "report") {
  const scan = await tooling("scan");
  const all = args.includes("--all");
  const dir = args.find((a) => !a.startsWith("--"));
  const { DEV_ONLY_PATHS } = await runnerImport(new URL("../src/i18n/localized.ts", import.meta.url).pathname).then((r) => r.module);
  const devOnly = DEV_ONLY_PATHS.map((p) => join(check.UI_DIR, p));
  const files = check.sourceFiles(dir ? [join(process.cwd(), dir)] : undefined).filter((f) => !devOnly.some((d) => f === d || f.startsWith(`${d}/`)));
  const findings = scan.scanFiles(files);
  const byFile = new Map();
  for (const f of findings) byFile.set(f.file, (byFile.get(f.file) ?? 0) + 1);
  const rows = [...byFile].sort((a, b) => b[1] - a[1]);
  for (const [file, n] of all ? rows : rows.slice(0, 40)) console.log(`${String(n).padStart(5)}  ${file}`);
  if (!all && rows.length > 40) console.log(`  … ${rows.length - 40} more files (--all)`);
  const keyRe = /\bt\(\s*["']([a-z0-9_.]+)["']|\bk="([a-z0-9_.]+)"/g;
  let localized = 0;
  for (const f of files) localized += [...readFileSync(f, "utf8").matchAll(keyRe)].length;
  const total = localized + findings.length;
  console.log(
    `\n${findings.length} hard-coded strings in ${byFile.size} files; ${localized} localized call sites (${total ? ((100 * localized) / total).toFixed(1) : 100} %)`,
  );
  if (dir && all) console.log(scan.formatFindings(findings));
} else if (cmd === "po") {
  const [lang] = args;
  if (!lang) throw new Error("usage: pnpm i18n po <lang>");
  const po = await tooling("po");
  const out = `${lang}.po`;
  writeFileSync(out, po.toPo(check.readCatalog(check.SOURCE_LOCALE), check.readCatalog(lang), lang));
  console.log(`wrote ${relative(process.cwd(), out)}`);
} else if (cmd === "po-import") {
  const [lang, file] = args;
  if (!lang || !file) throw new Error("usage: pnpm i18n po-import <lang> <file.po>");
  const po = await tooling("po");
  const source = check.readCatalog(check.SOURCE_LOCALE);
  const current = check.readCatalog(lang);
  const imported = po.fromPo(readFileSync(file, "utf8"));
  // English key order; keys gone from English are dropped.
  const merged = Object.fromEntries(Object.keys(source).flatMap((k) => (k in imported || k in current ? [[k, imported[k] ?? current[k]]] : [])));
  writeFileSync(join(check.LOCALES_DIR, `${lang}.json`), `${JSON.stringify(merged, null, 2)}\n`);
  console.log(`merged ${Object.keys(imported).length} entries into locales/${lang}.json; run \`pnpm i18n check\``);
} else {
  console.log("usage: pnpm i18n gen | check | report [--all] [dir] | po <lang> | po-import <lang> <file.po>");
  process.exitCode = cmd ? 1 : 0;
}
