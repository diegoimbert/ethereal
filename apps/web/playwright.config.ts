// Browser smoke tests for the web build (wasm engine). Run: `pnpm --filter @ethereal/web test:e2e`
// (builds crates/ether-wasm first). The web server uses this instance's Playwright port
// (scripts/dev-env.mjs), so parallel worktrees don't clash.
import { defineConfig, devices } from "@playwright/test";
import { port as devPort } from "../../scripts/dev-env.mjs";

const port = devPort("playwright");

export default defineConfig({
  testDir: "e2e",
  timeout: 60_000,
  fullyParallel: false,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: `http://127.0.0.1:${port}`,
    trace: "retain-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        launchOptions: {
          // Let the AudioContext start without a user gesture; no sound device needed.
          args: ["--autoplay-policy=no-user-gesture-required", "--mute-audio"],
        },
      },
    },
  ],
  webServer: {
    command: `pnpm exec vite --host 127.0.0.1 --port ${port} --strictPort`,
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
