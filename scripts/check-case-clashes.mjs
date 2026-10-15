#!/usr/bin/env node
// Fails when tracked paths collide on case-insensitive file systems (macOS, Windows), which
// Linux checks can't see (#811; v0.6.0 broke with TS1149 on Markdown.tsx vs markdown.ts):
// - paths (files or directories) equal ignoring case but not exactly;
// - files in one directory whose names minus the extension match ignoring case but not
//   exactly (Markdown.tsx vs markdown.ts: module resolution sees both as "./markdown").
// Same stem, same case, different extension (foo.ts + foo.css) is fine, and so are stems that
// differ only in case when one side is not a script module (Toast.tsx + toast.css).
import { execFileSync } from "node:child_process";

const files = execFileSync("git", ["ls-files", "-z"], { encoding: "utf8" }).split("\0").filter(Boolean);

const paths = new Set(files);
for (const f of files) for (let i = f.indexOf("/"); i !== -1; i = f.indexOf("/", i + 1)) paths.add(f.slice(0, i));

const groups = (items, key) => {
  const map = new Map();
  for (const item of items) {
    const k = key(item);
    if (!map.has(k)) map.set(k, new Set());
    map.get(k).add(item);
  }
  return [...map.values()].filter((g) => g.size > 1);
};

const problems = [];
for (const g of groups(paths, (p) => p.toLowerCase())) problems.push(`same path ignoring case: ${[...g].join(", ")}`);

// Only extensions that imports may leave out: "./toast.css" next to Toast.tsx is fine.
const module = /(?<=[^/])\.(?:[cm]?[jt]sx?)$/;
const stem = (f) => f.replace(module, "");
for (const g of groups(
  files.filter((f) => module.test(f)),
  (f) => stem(f).toLowerCase(),
)) {
  if (new Set([...g].map(stem)).size > 1) problems.push(`same module name ignoring case: ${[...g].join(", ")}`);
}

if (problems.length) {
  console.error(`case clashes (they break checkouts or builds on macOS and Windows):\n  ${problems.join("\n  ")}`);
  process.exit(1);
}
console.log(`no case clashes in ${files.length} tracked files`);
