// Browser host for the Ethereal UI. OWNERSHIP: the `wasm-host` node owns apps/web/**.
import { fileURLToPath, URL } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

/** Cross-origin isolation: required for SharedArrayBuffer (engine ↔ worklet rings). */
const crossOriginIsolationHeaders = {
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
};

// Port is assigned per instance by the justfile via ETHER_DEV_PORT; 5173 only when unset.
const port = process.env.ETHER_DEV_PORT ? Number(process.env.ETHER_DEV_PORT) : 5173;

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      // `@ethereal/ui` is consumed as source and uses `@/…` imports internally.
      "@": fileURLToPath(new URL("../../ui/src", import.meta.url)),
    },
  },
  server: {
    port,
    strictPort: true,
    headers: crossOriginIsolationHeaders,
  },
  preview: {
    port,
    strictPort: true,
    headers: crossOriginIsolationHeaders,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
  },
});
