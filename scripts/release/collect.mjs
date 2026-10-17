#!/usr/bin/env node
// Collects release artifacts into one flat directory.
//
//   node scripts/release/collect.mjs desktop <target-triple> <out-dir>
//     Copies the installers Tauri produced under target/<triple>/release/bundle/ (dmg, deb,
//     AppImage, msi, NSIS setup.exe), zips the macOS .app, and checks that the sandbox
//     helper and the MCP server (`ether-mcp`) are inside the macOS .app and the .deb whenever
//     tauri.conf.json bundles them. The app license (LICENSE) and the third-party notices
//     (THIRD-PARTY-NOTICES.md) are copied next to the installers (the installers also bundle
//     them; see tauri.conf.json).
//     On macOS it also checks the .app's identity as System Settings shows it (name, bundle
//     id, icon, usage strings, signing identifier).
//     Updater bundles (auto-update; only built when TAURI_SIGNING_PRIVATE_KEY is set) are
//     collected with their `.sig`: the macOS `Ethereal.app.tar.gz` is renamed
//     `Ethereal_<v>_<arch>.app.tar.gz` (both macOS builds name it the same), the AppImage /
//     setup.exe / msi signatures keep their installer's name + `.sig`.
//
//   node scripts/release/collect.mjs web <version> <out-dir>
//     Zips apps/web/dist + the COOP/COEP deployment files (scripts/release/web/) + LICENSE +
//     THIRD-PARTY-NOTICES.md into Ethereal_<version>_web.zip.
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
// Signatures of the updater bundles (`tauri-plugin-updater`); `.deb.sig` is not used.
const UPDATER_SIG = /\.(AppImage|msi|exe)\.sig$/;
const HELPERS = ["ether-sandbox-helper", "ether-mcp"];

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

// What macOS shows in System Settings › Privacy & Security (Screen & System Audio
// Recording, Microphone) comes from the app's Info.plist and code signature: check that the
// built .app names Ethereal, has its icon and the usage strings of src-tauri/Info.plist, and
// is signed as the bundle id (docs/RELEASING.md "macOS permissions").
function checkMacIdentity(app, conf) {
  const plist = join(app, "Contents", "Info.plist");
  const read = (key) => {
    try {
      return execFileSync("plutil", ["-extract", key, "raw", "-o", "-", plist], { encoding: "utf8" }).trim();
    } catch {
      return "";
    }
  };
  const expect = { CFBundleName: conf.productName, CFBundleIdentifier: conf.identifier, CFBundleExecutable: conf.mainBinaryName ?? "ether-desktop" };
  for (const [key, want] of Object.entries(expect)) if (read(key) !== want) fail(`${basename(app)}: ${key} is '${read(key)}', expected '${want}'`);
  const icon = read("CFBundleIconFile");
  if (!icon || !existsSync(join(app, "Contents", "Resources", icon))) fail(`${basename(app)}: no CFBundleIconFile in Contents/Resources`);
  const usage = readFileSync(join(root, "apps/desktop/src-tauri/Info.plist"), "utf8").match(/NS\w+UsageDescription/g) ?? [];
  for (const key of usage) if (!read(key)) fail(`${basename(app)}: ${key} (apps/desktop/src-tauri/Info.plist) is missing from the bundle`);
  // codesign prints its details on stderr.
  const out = execFileSync("sh", ["-c", 'codesign -dv "$1" 2>&1', "sh", app], { encoding: "utf8" });
  const id = out.match(/^Identifier=(.*)$/m)?.[1];
  if (id !== conf.identifier) fail(`${basename(app)}: signed as '${id}', expected '${conf.identifier}'`);
  const adhoc = /^Signature=adhoc$/m.test(out);
  console.log(
    `collect: ${basename(app)}: ${conf.identifier}, icon ${icon}, ${usage.join(", ")}; ${adhoc ? "ad-hoc signature (privacy grants reset on every update)" : "Developer ID signature"}`,
  );
}

function desktop(triple) {
  const targetDir = process.env.CARGO_TARGET_DIR ? resolve(root, process.env.CARGO_TARGET_DIR) : join(root, "target");
  const bundleDir = join(targetDir, triple, "release", "bundle");
  const files = walk(bundleDir);
  if (files.length === 0) fail(`nothing under ${bundleDir}`);
  const conf = readFileSync(join(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8");
  const helpers = triple.includes("windows") ? [] : HELPERS.filter((h) => conf.includes(`"binaries/${h}"`));
  const collected = [];

  for (const f of files) {
    const name = basename(f);
    if (name.endsWith(".app")) {
      for (const h of helpers) if (!existsSync(join(f, "Contents", "MacOS", h))) fail(`${name} has no Contents/MacOS/${h}`);
      checkMacIdentity(f, JSON.parse(conf));
      const arch = triple.split("-")[0];
      const version = (process.env.ETHER_RELEASE_VERSION || readFileSync(join(root, "VERSION"), "utf8")).trim();
      const zip = join(outDir, `${name.replace(/\.app$/, "")}_${version}_${arch}.app.zip`);
      // ditto keeps the bundle's symlinks, permissions and code signature intact.
      execFileSync("ditto", ["-c", "-k", "--sequesterRsrc", "--keepParent", f, zip], { stdio: "inherit" });
      collected.push(zip);
    } else if (name.endsWith(".app.tar.gz") || name.endsWith(".app.tar.gz.sig")) {
      // auto-update: the macOS updater bundle (and its signature), named per arch.
      const arch = triple.split("-")[0];
      const version = (process.env.ETHER_RELEASE_VERSION || readFileSync(join(root, "VERSION"), "utf8")).trim();
      const dest = join(outDir, name.replace(/^(.*)\.app\.tar\.gz/, `$1_${version}_${arch}.app.tar.gz`));
      copyFileSync(f, dest);
      collected.push(dest);
    } else if (UPDATER_SIG.test(name)) {
      const dest = join(outDir, name);
      copyFileSync(f, dest);
      collected.push(dest);
    } else if (INSTALLER.test(name) && !name.startsWith("rw.")) {
      if (helpers.length > 0 && name.endsWith(".deb")) {
        const list = execFileSync("dpkg-deb", ["-c", f], { encoding: "utf8" });
        // Entries print as `./usr/bin/x` or `usr/bin/x` depending on how the tar was made.
        const has = (bin) => new RegExp(`(^|[\\s/])usr/bin/${bin}$`, "m").test(list);
        const missing = helpers.filter((h) => !has(h));
        if (missing.length > 0) {
          console.error(list);
          fail(`${name} has no /usr/bin/${missing.join(", /usr/bin/")}`);
        }
      }
      const dest = join(outDir, name);
      copyFileSync(f, dest);
      collected.push(dest);
    }
  }
  if (collected.length === 0) fail(`no installers under ${bundleDir}`);
  // The app's own license and the third-party notices, next to the installers.
  for (const name of ["LICENSE", "THIRD-PARTY-NOTICES.md"]) {
    const src = join(root, name);
    if (existsSync(src)) {
      const dest = join(outDir, name);
      copyFileSync(src, dest);
      collected.push(dest);
    }
  }
  for (const c of collected) console.log(`collect: ${c}`);
}

function web(version) {
  const dist = join(root, "apps/web/dist");
  if (!existsSync(join(dist, "index.html"))) fail(`${dist} is missing; run pnpm --filter @ethereal/web build`);
  const stage = join(mkdtempSync(join(tmpdir(), "ether-web-")), `Ethereal_${version}_web`);
  cpSync(dist, stage, { recursive: true });
  cpSync(join(root, "scripts/release/web"), stage, { recursive: true });
  for (const name of ["LICENSE", "THIRD-PARTY-NOTICES.md"]) cpSync(join(root, name), join(stage, name));
  const zip = join(outDir, `${basename(stage)}.zip`);
  execFileSync("zip", ["-qr", zip, basename(stage)], { cwd: dirname(stage), stdio: "inherit" });
  console.log(`collect: ${zip}`);
}

if (kind === "desktop") desktop(arg);
else web(arg);
