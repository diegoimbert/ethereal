#!/usr/bin/env node
// Release debugging (OSS #201): how the full product's node_modules links resolve on this
// runner. Prints one JSON line per path (lstat, readlink, existence, both realpaths) and then
// what a require / import of a few packages from ee/app resolves to. Run through
// private-step.mjs: the output names private paths, so it belongs in the encrypted log only.
//
//   node scripts/ci/link-diag.mjs
import { execFileSync } from "node:child_process";
import { existsSync, lstatSync, readdirSync, readlinkSync, realpathSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "../..");
const paths = [
  "ui/node_modules",
  "ui/node_modules/vite",
  "ui/node_modules/@testing-library/jest-dom",
  "apps/desktop/node_modules/@tauri-apps/cli",
  "ee/app/node_modules",
  "ee/app/node_modules/vite",
  "ee/app/node_modules/vite/client.d.ts",
  "ee/app/node_modules/@testing-library/jest-dom/vitest.d.ts",
  "ee/app/web/node_modules",
  "ee/apps/desktop/node_modules",
  "ee/apps/desktop/node_modules/@tauri-apps/cli/tauri.js",
];
const attempt = (f) => {
  try {
    return f();
  } catch (e) {
    return `${e.code ?? "error"}: ${e.message.split("\n")[0]}`;
  }
};
for (const p of paths) {
  const abs = join(root, p);
  const st = attempt(() => lstatSync(abs));
  console.log(
    JSON.stringify({
      p,
      lstat: typeof st === "string" ? st : { link: st.isSymbolicLink(), dir: st.isDirectory(), file: st.isFile() },
      readlink: typeof st !== "string" && st.isSymbolicLink() ? attempt(() => readlinkSync(abs)) : undefined,
      exists: existsSync(abs),
      real: attempt(() => realpathSync(abs)),
      realNative: attempt(() => realpathSync.native(abs)),
    }),
  );
}
const req = createRequire(join(root, "ee/app/package.json"));
for (const m of ["vite/package.json", "react", "@testing-library/jest-dom/package.json", "typescript"]) {
  console.log(JSON.stringify({ require: m, from: "ee/app", resolved: attempt(() => req.resolve(m)) }));
}
console.log(JSON.stringify({ uiNodeModules: attempt(() => readdirSync(join(root, "ui/node_modules")).slice(0, 12)) }));
console.log(JSON.stringify({ eeAppNodeModules: attempt(() => readdirSync(join(root, "ee/app/node_modules")).slice(0, 12)) }));
// Windows: the reparse point kinds (<JUNCTION> / <SYMLINKD> / <SYMLINK>) and their targets.
if (process.platform === "win32") {
  for (const d of ["ui\\node_modules", "ui\\node_modules\\@testing-library", "ee\\app", "ee\\app\\web", "ee\\apps\\desktop"]) {
    console.log(`--- dir /AL ${d}`);
    console.log(attempt(() => execFileSync("cmd", ["/c", "dir", "/AL", join(root, d)], { encoding: "utf8" })));
  }
}
