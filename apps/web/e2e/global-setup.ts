// `ETHER_E2E_PREBUILD=1` (the local merge gate): build the native binaries the server-backed
// specs start, once, before any spec runs. Each of those specs still runs its own `cargo
// build` in `beforeAll` (that is how it finds the binary), but against a fresh target it is a
// no-op. Without this, the first spec to need a binary compiles it inside its hook (3 jobs, so
// the browsers keep their cores) while the other specs' hooks wait on cargo's lock, and on a
// loaded machine that passes the hooks' 600 s timeout. Unset (a dev running a few specs):
// nothing is built ahead; a spec builds what it needs.
//
// The commands match the specs' exactly (package, target, features): another feature set
// would be another build, and the specs' own builds would not be no-ops.
import { execFileSync } from "node:child_process";
import { rmSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");

const BUILDS: string[][] = [
  // comping, live-record, remote-engine, tap-recording
  ["build", "-p", "ether-server", "--bin", "ether-server", "--features", "full"],
];

export default function globalSetup(): void {
  // The previous run's CSP violation reports (e2e/global-teardown.ts).
  const reports = process.env.ETHER_CSP_REPORTS;
  if (reports) rmSync(reports, { force: true });
  if (process.env.ETHER_E2E_PREBUILD !== "1") return;
  for (const args of BUILDS) {
    const t0 = Date.now();
    // Cargo's progress and errors go to stderr.
    execFileSync("cargo", args, { cwd: repoRoot, stdio: ["ignore", "ignore", "inherit"] });
    console.log(`e2e prebuild: cargo ${args.join(" ")}: ${Math.round((Date.now() - t0) / 1000)} s`);
  }
}
