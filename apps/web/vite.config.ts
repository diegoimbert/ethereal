/// <reference types="vitest/config" />
// Browser host for the Ethereal UI.
import { fileURLToPath, URL } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { port as devPort } from "../../scripts/dev-env.mjs";
import { cspPreview } from "./csp.ts";
import { i18nCatalogPlugin } from "../../ui/src/i18n/tooling/vitePlugin.ts";

/** Cross-origin isolation: required for SharedArrayBuffer (engine ↔ worklet rings). */
const crossOriginIsolationHeaders = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

// Per-instance port (ETHER_DEV_PORT or derived from ETHER_INSTANCE); see scripts/dev-env.mjs.
const port = devPort("dev");
const previewPort = devPort("preview");

export default defineConfig({
  // Absolute asset URLs (/assets/…): the app is also served at deep paths (/p/<slug>/<id>,
  // /s/<id>, /join/…), where relative ones would break.
  base: "/",
  // cspPreview: `vite preview` serves the deployed Content-Security-Policy (csp.ts).
  // i18nCatalogPlugin: the UI's compiled translation catalogs (docs/LOCALIZATION.md).
  plugins: [react(), cspPreview(), i18nCatalogPlugin()],
  resolve: {
    alias: {
      // `@ethereal/ui` is consumed as source and uses `@/…` imports internally.
      "@": fileURLToPath(new URL("../../ui/src", import.meta.url)),
      // wasm-bindgen output of crates/ether-wasm (scripts/build-wasm.mjs; git-ignored).
      "@ether-wasm": fileURLToPath(new URL("./src/wasm/pkg", import.meta.url)),
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
  // Worker and worklet bundles are ES modules (the worklet is loaded with addModule).
  worker: { format: "es" },
  test: {
    // Unit tests of the engine glue that don't need a browser (the e2e covers the rest).
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
  },
});
