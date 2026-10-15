// Lists hard-coded design values left in feature code, for the design sweep.
// Run: `just report-hardcoded [dir ...]` (default: ui/src/features; paths relative to ui/ or repo).
// Always exits 0: it is an inventory, not a gate (the gate is hardcoded.test.ts).
// Works on Node >= 20 (TS loaded through Vite's module runner).
import { fileURLToPath } from "node:url";
import { runnerImport } from "vite";

const { module: h } = await runnerImport(fileURLToPath(new URL("./hardcoded.ts", import.meta.url)));
const root = fileURLToPath(new URL("../..", import.meta.url)); // ui/
const dirs = process.argv.slice(2).map((d) => d.replace(/^(\.\/)?ui\//, ""));
const findings = h.scanDirs(root, dirs.length ? dirs : ["src/features"], { lengths: true });
console.log(h.formatFindings(findings));
console.log(`\n${h.summarize(findings)}`);
