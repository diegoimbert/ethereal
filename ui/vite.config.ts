/// <reference types="vitest/config" />
import { fileURLToPath, URL } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { port as devPort } from "../scripts/dev-env.mjs";
import { tokensCssPlugin } from "./src/theme/tokens-plugin.ts";
import { i18nCatalogPlugin } from "./src/i18n/tooling/vitePlugin.ts";

/**
 * Cross-origin isolation headers. Required for `SharedArrayBuffer` (web engine rings).
 * Exported so hosts (apps/web) can reuse them.
 */
export const crossOriginIsolationHeaders = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

// Per-instance port (ETHER_DEV_PORT or derived from ETHER_INSTANCE); see scripts/dev-env.mjs.
const port = devPort("dev");
const previewPort = devPort("preview");

/**
 * `ETHER_GATE=1`: the local merge gate, where several gates and agents share one machine.
 * The suite then runs on a few workers (`VITEST_MAX_WORKERS` still overrides) instead of one
 * per core, so parallel runs don't oversubscribe the box.
 */
const gate = process.env.ETHER_GATE === "1";

export default defineConfig({
  // i18nCatalogPlugin: the compiled translation catalogs (docs/LOCALIZATION.md).
  // tokensCssPlugin: regenerates src/theme/tokens.css when src/theme/tokens.ts changes.
  plugins: [react(), tokensCssPlugin(), i18nCatalogPlugin()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  server: {
    port,
    strictPort: true,
    headers: crossOriginIsolationHeaders,
  },
  preview: {
    port: previewPort,
    strictPort: true,
    headers: crossOriginIsolationHeaders,
  },
  // Tauri expects a fixed dist dir and doesn't need to clear the screen.
  clearScreen: false,
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./vitest.setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
    css: false,
    // Many tests here are integration-sized (the app's stores, the MockTransport, a rendered
    // panel): fast alone, but a loaded machine stretches them past vitest's 5 s default.
    testTimeout: 20_000,
    hookTimeout: 20_000,
    ...(gate ? { maxWorkers: 8 } : {}),
  },
});
