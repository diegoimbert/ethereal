#!/usr/bin/env node
// Per-instance dev environment: lets many worktrees/agents run dev servers and apps at the
// same time without clashing. Single source of truth for the instance id and ports.
//
//   ETHER_INSTANCE  default: repo (worktree) directory name, sanitized to [A-Za-z0-9_-]
//   ETHER_DEV_PORT  default: 20000 + fnv1a(instance) % 10000
//
// Port offsets from ETHER_DEV_PORT (base):
//   +0 UI / web dev server (Vite; Tauri devUrl)   +1 vite preview
//   +2 Playwright web server                      +3 reserved (collab server)
//
// CLI: `node scripts/dev-env.mjs` prints `export ...` lines for `eval` (used by the
// justfile); `node scripts/dev-env.mjs port` prints the base port.
import { basename, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const PORT_OFFSETS = { dev: 0, preview: 1, playwright: 2, collab: 3 };

export function sanitizeInstance(name) {
  return name.replace(/[^A-Za-z0-9_-]/g, "-");
}

function fnv1a(str) {
  let h = 0x811c9dc5;
  for (let i = 0; i < str.length; i++) {
    h ^= str.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h >>> 0;
}

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

export function instanceId(env = process.env) {
  const fromEnv = env.ETHER_INSTANCE && sanitizeInstance(env.ETHER_INSTANCE);
  return fromEnv || sanitizeInstance(basename(repoRoot)) || "default";
}

export function basePort(env = process.env) {
  if (env.ETHER_DEV_PORT) return Number(env.ETHER_DEV_PORT);
  return 20000 + (fnv1a(instanceId(env)) % 10000);
}

export function port(kind = "dev", env = process.env) {
  return basePort(env) + PORT_OFFSETS[kind];
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const arg = process.argv[2];
  if (arg === "port") {
    console.log(basePort());
  } else if (arg === "instance") {
    console.log(instanceId());
  } else {
    console.log(`export ETHER_INSTANCE=${instanceId()}`);
    console.log(`export ETHER_DEV_PORT=${basePort()}`);
  }
}
