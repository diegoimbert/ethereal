// The native engine server (`ether-server`) for the specs that run against it (comping,
// live-record, remote-engine).
import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");

/**
 * Build `ether-server` and return the binary path (cargo tells where it is). The build matches
 * e2e/global-setup.ts exactly, so after its prebuild this is a no-op. `ETHER_E2E_SERVER_BIN`
 * names an already built binary instead (CI builds it once and hands it to every shard).
 */
export function buildServer(): string {
  const prebuilt = process.env.ETHER_E2E_SERVER_BIN;
  if (prebuilt) return resolve(prebuilt);
  const out = execFileSync(
    "cargo",
    // Every Cargo feature, like the shipped server (docs/ARCHITECTURE.md "Cargo features").
    ["build", "-p", "ether-server", "--bin", "ether-server", "--features", "full", "--message-format=json"],
    {
      cwd: repoRoot,
      env: { ...process.env, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "3" },
      encoding: "utf8",
      maxBuffer: 256 * 1024 * 1024,
      stdio: ["ignore", "pipe", "inherit"],
    },
  );
  for (const line of out.split("\n")) {
    if (!line.startsWith("{")) continue;
    const msg = JSON.parse(line) as { reason?: string; target?: { name: string }; executable?: string | null };
    if (msg.reason === "compiler-artifact" && msg.target?.name === "ether-server" && msg.executable) return msg.executable;
  }
  throw new Error("cargo did not report the ether-server binary");
}
