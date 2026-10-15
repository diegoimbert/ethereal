#!/usr/bin/env node
// Regenerates the user docs' screenshots (docs-site/content/<section>/images/*.jpg) from the
// real web app: runs apps/web/e2e/docs.shots.spec.ts with DOCS_SHOTS set. Build the wasm engine
// first (`just e2e-web` does, or `node apps/web/scripts/build-wasm.mjs`).
//   node scripts/docs/screenshots.mjs
import { spawnSync } from "node:child_process";
import { mkdirSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { basePort, instanceId } from "../dev-env.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const content = join(root, "docs-site/content");
const shots = ["basics/images/the-screen.jpg", "making-music/images/piano-roll.jpg", "mobile/images/phone.jpg"];
for (const s of shots) mkdirSync(dirname(join(content, s)), { recursive: true });

const run = spawnSync("npx", ["playwright", "test", "e2e/docs.shots.spec.ts", "--workers=2"], {
  cwd: join(root, "apps/web"),
  stdio: "inherit",
  env: { ...process.env, ETHER_INSTANCE: instanceId(), ETHER_DEV_PORT: String(basePort()), DOCS_SHOTS: content },
});
if (run.status !== 0) process.exit(run.status ?? 1);

for (const s of shots) {
  const kb = Math.round(statSync(join(content, s)).size / 1024);
  console.log(`${s}: ${kb} KB${kb > 200 ? "  (over 200 KB: lower the quality in docs.shots.spec.ts)" : ""}`);
}
