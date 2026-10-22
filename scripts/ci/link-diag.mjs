#!/usr/bin/env node
// Release debugging (OSS #201): how the full product's node_modules links resolve on this
// runner. Prints one JSON line per path (lstat, readlink, existence, both realpaths) and then
// what a require / import of a few packages from ee/app resolves to. Run through
// private-step.mjs: the output names private paths, so it belongs in the encrypted log only.
//
//   node scripts/ci/link-diag.mjs
import { execFileSync } from "node:child_process";
import { cpSync, existsSync, lstatSync, mkdirSync, readdirSync, readlinkSync, realpathSync, rmdirSync, symlinkSync, unlinkSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, relative, resolve } from "node:path";

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
// Windows: the candidate kinds of ee/app/node_modules, tried in place. Does the OS (realpath
// native, which TypeScript uses) reach a file behind pnpm's relative links through it?
if (process.platform === "win32") {
  const link = join(root, "ee/app/node_modules");
  const target = join(root, "ui/node_modules");
  const probe = () => ({
    vite: attempt(() => realpathSync.native(join(link, "vite/client.d.ts"))),
    jestDom: attempt(() => realpathSync.native(join(link, "@testing-library/jest-dom/vitest.d.ts"))),
    require: attempt(() => createRequire(join(root, "ee/app/package.json")).resolve("react")),
  });
  const variants = {
    "symlink dir, absolute": () => symlinkSync(target, link, "dir"),
    "symlink dir, relative": () => symlinkSync(relative(dirname(link), target), link, "dir"),
    "mirror: a real dir of junctions to each entry's real path": () => {
      mkdirSync(link);
      for (const e of readdirSync(target, { withFileTypes: true })) {
        const from = join(target, e.name);
        if (e.name === ".bin") cpSync(from, join(link, e.name), { recursive: true });
        else if (e.name.startsWith("@")) {
          mkdirSync(join(link, e.name));
          for (const s of readdirSync(from)) symlinkSync(realpathSync(join(from, s)), join(link, e.name, s), "junction");
        } else if (e.isSymbolicLink()) symlinkSync(realpathSync(from), join(link, e.name), "junction");
      }
    },
  };
  // Removes links as links (never through them: a recursive rm could empty their targets).
  const remove = (p) => {
    const st = attempt(() => lstatSync(p));
    if (typeof st === "string") return;
    if (st.isSymbolicLink()) {
      if (typeof attempt(() => unlinkSync(p)) === "string") rmdirSync(p);
    } else if (st.isDirectory()) {
      for (const e of readdirSync(p)) remove(join(p, e));
      rmdirSync(p);
    } else unlinkSync(p);
  };
  for (const [name, make] of Object.entries(variants)) {
    remove(link);
    const made = attempt(make);
    console.log(JSON.stringify({ variant: name, made: made ?? "ok", ...probe() }));
  }
  remove(link);
  // Back to what link.mjs makes.
  console.log(attempt(() => execFileSync(process.execPath, [join(root, "ee/app/link.mjs")], { encoding: "utf8" })));
  console.log(JSON.stringify({ restored: probe() }));
}
// Windows: the reparse point kinds (<JUNCTION> / <SYMLINKD> / <SYMLINK>) and their targets.
if (process.platform === "win32") {
  for (const d of ["ui\\node_modules", "ui\\node_modules\\@testing-library", "ee\\app", "ee\\app\\web", "ee\\apps\\desktop"]) {
    console.log(`--- dir /AL ${d}`);
    console.log(attempt(() => execFileSync("cmd", ["/c", "dir", "/AL", join(root, d)], { encoding: "utf8" })));
  }
}
