/// <reference types="vitest/config" />
import { fileURLToPath, URL } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { port as devPort } from "../scripts/dev-env.mjs";

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

export default defineConfig({
  plugins: [react()],
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
  },
});
