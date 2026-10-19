#!/usr/bin/env node
// Downloads and decrypts the full-product CI's logs of one run (.github/workflows/full.yml):
// the `full-logs-*` artifacts, age-encrypted to the private repository's recipients.
//
//   node scripts/ci/full-logs.mjs <run-id> [out-dir]      (`just full-logs <run-id>`)
//
// Needs `gh` and `age`, and an age identity that is one of the recipients:
// $SOPS_AGE_KEY_FILE, else ~/.config/sops/age/keys.txt.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";

const [run, outArg] = process.argv.slice(2);
if (!/^\d+$/.test(run ?? "")) {
  console.error("usage: full-logs.mjs <run-id> [out-dir]");
  process.exit(2);
}
const repo = process.env.FEAT_OSS_REPO || "diegoimbert/ethereal";
const identity = process.env.SOPS_AGE_KEY_FILE || join(homedir(), ".config/sops/age/keys.txt");
if (!existsSync(identity)) {
  console.error(`full-logs: no age identity at ${identity} (set SOPS_AGE_KEY_FILE)`);
  process.exit(1);
}
const out = resolve(outArg || join(tmpdir(), `ether-full-logs-${run}`));
const dl = join(out, ".download");
mkdirSync(dl, { recursive: true });
execFileSync("gh", ["run", "download", run, "-R", repo, "-p", "full-logs-*", "-D", dl], { stdio: "inherit" });

for (const name of readdirSync(dl)) {
  for (const f of readdirSync(join(dl, name)).filter((f) => f.endsWith(".age"))) {
    const dest = join(out, name);
    mkdirSync(dest, { recursive: true });
    const tar = execFileSync("age", ["-d", "-i", identity, join(dl, name, f)], { maxBuffer: 1 << 30 });
    execFileSync("tar", ["-xzf", "-", "-C", dest], { input: tar });
    console.log(`${dest}: ${readdirSync(dest).join(", ")}`);
  }
}
