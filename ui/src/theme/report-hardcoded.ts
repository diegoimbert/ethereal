// Lists hard-coded colors / px font sizes / px lengths left in feature code, for the design
// sweep. Run: `node ui/src/theme/report-hardcoded.ts [dir ...]` (default: ui/src/features).
// Always exits 0: it is an inventory, not a gate (the gate is hardcoded.test.ts).
import { fileURLToPath } from "node:url";
import { formatFindings, scanDirs, type FindingKind } from "./hardcoded.ts";

const root = fileURLToPath(new URL("../..", import.meta.url)); // ui/
const dirs = process.argv.slice(2).map((d) => d.replace(/^ui\//, ""));
const findings = scanDirs(root, dirs.length ? dirs : ["src/features"], { lengths: true });

console.log(formatFindings(findings));
const byKind = new Map<FindingKind, number>();
const byFile = new Map<string, number>();
for (const f of findings) {
  byKind.set(f.kind, (byKind.get(f.kind) ?? 0) + 1);
  byFile.set(f.file, (byFile.get(f.file) ?? 0) + 1);
}
console.log(`\n${findings.length} hard-coded values in ${byFile.size} files`);
for (const [k, n] of byKind) console.log(`  ${k}: ${n}`);
