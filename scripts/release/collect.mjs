#!/usr/bin/env node
// Collects release artifacts into one flat directory.
//
//   node scripts/release/collect.mjs desktop <target-triple> <out-dir>
//     Copies the installers Tauri produced under target/<triple>/release/bundle/ (dmg, deb,
//     AppImage, msi, NSIS setup.exe), zips the macOS .app, and checks that the sandbox
//     helper sidecar is inside the macOS/Linux bundles whenever tauri.conf.json bundles it.
//
//   node scripts/release/collect.mjs web <version> <out-dir>
//     Zips apps/web/dist + the COOP/COEP deployment files (scripts/release/web/) + notices
//     into Ethereal_<version>_web.zip.
import { execFileSync } from "node:child_process";
import { copyFileSync, cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const [kind, arg, outArg] = process.argv.slice(2);
if (!["desktop", "web"].includes(kind) || !arg || !outArg) {
  console.error("usage: collect.mjs desktop <target-triple> <out-dir> | web <version> <out-dir>");
  process.exit(2);
}
const outDir = resolve(outArg);
mkdirSync(outDir, { recursive: true });

const INSTALLER = /\.(dmg|deb|AppImage|msi|exe)$/;
const HELPER = "ether-sandbox-helper";

function walk(dir, depth = 2) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    const st = statSync(p);
    if (st.isDirectory() && !f.endsWith(".app") && depth > 0) return walk(p, depth - 1);
    return [p];
  });
}

function fail(msg) {
  console.error(`collect: ${msg}`);
  process.exit(1);
}

function desktop(triple) {
  const targetDir = process.env.CARGO_TARGET_DIR ? resolve(root, process.env.CARGO_TARGET_DIR) : join(root, "target");
  const bundleDir = join(targetDir, triple, "release", "bundle");
  const files = walk(bundleDir);
  if (files.length === 0) fail(`nothing under ${bundleDir}`);
  const conf = readFileSync(join(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8");
  const wantHelper = conf.includes(HELPER) && !triple.includes("windows");
  const collected = [];

  for (const f of files) {
    const name = basename(f);
    if (name.endsWith(".app")) {
      if (wantHelper && !existsSync(join(f, "Contents", "MacOS", HELPER))) fail(`${name} has no Contents/MacOS/${HELPER}`);
      const arch = triple.split("-")[0];
      const version = readFileSync(join(root, "VERSION"), "utf8").trim();
      const zip = join(outDir, `${name.replace(/\.app$/, "")}_${version}_${arch}.app.zip`);
      // ditto keeps the bundle's symlinks, permissions and code signature intact.
      execFileSync("ditto", ["-c", "-k", "--sequesterRsrc", "--keepParent", f, zip], { stdio: "inherit" });
      collected.push(zip);
    } else if (INSTALLER.test(name)) {
      if (wantHelper && name.endsWith(".deb")) {
        const list = execFileSync("dpkg-deb", ["-c", f], { encoding: "utf8" });
        if (!list.includes(`/usr/bin/${HELPER}`)) fail(`${name} has no /usr/bin/${HELPER}`);
      }
      const dest = join(outDir, name);
      copyFileSync(f, dest);
      collected.push(dest);
    }
  }
  if (collected.length === 0) fail(`no installers under ${bundleDir}`);
  for (const c of collected) console.log(`collect: ${c}`);
}

function web(version) {
  const dist = join(root, "apps/web/dist");
  if (!existsSync(join(dist, "index.html"))) fail(`${dist} is missing; run pnpm --filter @ethereal/web build`);
  const stage = join(mkdtempSync(join(tmpdir(), "ether-web-")), `Ethereal_${version}_web`);
  cpSync(dist, stage, { recursive: true });
  cpSync(join(root, "scripts/release/web"), stage, { recursive: true });
  for (const f of ["THIRD_PARTY_NOTICES.txt", "LICENSE"]) {
    if (!existsSync(join(root, f))) fail(`${f} is missing`);
    copyFileSync(join(root, f), join(stage, f === "LICENSE" ? "LICENSE.txt" : f));
  }
  const zip = join(outDir, `${basename(stage)}.zip`);
  execFileSync("zip", ["-qr", zip, basename(stage)], { cwd: dirname(stage), stdio: "inherit" });
  console.log(`collect: ${zip}`);
}

if (kind === "desktop") desktop(arg);
else web(arg);
