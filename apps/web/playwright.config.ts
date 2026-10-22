// Browser smoke tests for the web build (wasm engine). Run: `pnpm --filter @ethereal/web test:e2e`
// (builds crates/ether-wasm first). The web server uses this instance's Playwright port
// (scripts/dev-env.mjs), so parallel worktrees don't clash.
import { tmpdir } from "node:os";
import { join } from "node:path";
import { defineConfig, devices } from "@playwright/test";
import { port as devPort } from "../../scripts/dev-env.mjs";

const port = devPort("playwright");
/** `ETHER_E2E_PREVIEW=1`: test the production bundle (`vite build` output) instead of dev. */
const preview = process.env.ETHER_E2E_PREVIEW === "1";
/**
 * The preview server serves the deployed Content-Security-Policy (csp.ts) and collects the
 * browsers' violation reports in this file; the run fails on any (e2e/global-teardown.ts).
 */
if (preview) process.env.ETHER_CSP_REPORTS ??= join(tmpdir(), `ether-csp-reports-${port}.jsonl`);

// Let the AudioContext start without a user gesture; no sound device needed. WebGL2 on
// GPU-less hosts: SwiftShader.
const CHROME_ARGS = ["--autoplay-policy=no-user-gesture-required", "--mute-audio", "--enable-unsafe-swiftshader"];

/**
 * The arranger draws with WebGL2, or its Canvas2D fallback without a working WebGL2. The
 * `webgl` project runs every spec on the default (WebGL); `canvas2d` runs the arranger specs
 * again with the fallback forced by its hidden debug override (`eth.arr.renderer`), and
 * `ETHER_E2E_ARRANGER=canvas2d` runs every spec on it.
 */
/**
 * Every spec starts from an empty project: "New project" otherwise creates the starter one
 * (a MIDI track "Synth"), which specs that count or name tracks don't expect.
 */
const EMPTY_PROJECTS = { name: "eth.project.empty", value: "1" };
/**
 * The color blobs (behind the arrangement and modals) are off in tests: they would tint pixel
 * comparisons, and their blurred, always-running animation repaints the page every frame (on
 * SwiftShader, ~13 fps at rest). Settings > Color blobs (`eth-color-blobs` since 56216079).
 */
const NO_BLOBS = { name: "eth-color-blobs", value: "off" };
/**
 * Playwright's Chromium refuses persistent storage: the one-time "projects may be
 * cleared" notice is marked as shown, so it doesn't cover the page in every spec
 * (storage-persist.spec.ts tests it).
 */
const PERSIST_NOTICE_SHOWN = { name: "eth.storage.persist-notice", value: "1" };
/** Settings > Advanced shows the engine server form only once turned on; specs use it. */
const ENGINE_SERVER_SHOWN = { name: "eth.engine-server", value: "on" };
const storage = (extra: Array<{ name: string; value: string }> = []) => ({
  cookies: [],
  origins: [{ origin: `http://127.0.0.1:${port}`, localStorage: [EMPTY_PROJECTS, NO_BLOBS, PERSIST_NOTICE_SHOWN, ENGINE_SERVER_SHOWN, ...extra] }],
});
const canvas2dArranger = storage([{ name: "eth.arr.renderer", value: "canvas2d" }]);
const allCanvas2d = process.env.ETHER_E2E_ARRANGER === "canvas2d";

/**
 * The guided tours' walkthrough (every tour, every step, three window sizes; about a minute):
 * opt-in, not in the default run. `ETHEREAL_TOURS=1` defines the `tours` project that runs
 * it (`just e2e-tours`); docs/TUTORIALS.md says when to run it.
 */
const TOURS_WALKTHROUGH = "tours.walkthrough.spec.ts";
const toursOn = process.env.ETHEREAL_TOURS === "1";

/** The arranger specs, run on both backends (perf and never-disappear tests stay on WebGL). */
const ARRANGER_SPECS = [
  "webgl-presence.spec.ts",
  "webgl-arranger-text.spec.ts",
  "webgl-arranger.spec.ts",
  "track-header-meter.spec.ts",
  "clip-extend-left.spec.ts",
  "track-header-polish.spec.ts",
  "track-header-slots.spec.ts",
  "slip-edit.spec.ts",
  "multi-clip-edit.spec.ts",
  "multi-clip-transpose.spec.ts",
  "clip-drag-new-track.spec.ts",
  "time-edits.spec.ts",
  "section-edit.spec.ts",
  "flow.spec.ts",
  "freeze.spec.ts",
  // Phase 2c (presence, drops, recording):
  "collab.spec.ts",
  "collab-presence.spec.ts",
  "file-import.spec.ts",
  "folder-import.spec.ts",
  "live-record.spec.ts",
  "recording.spec.ts",
  // In-clip editing overlays (phase 2a): fades, crossfades, clip gain, badges.
  "clip-editing.spec.ts",
  "crossfades.spec.ts",
  "clip-gain-waveform.spec.ts",
  // Lanes besides clips (phase 2b): automation, comping, group summaries.
  "webgl-lanes.spec.ts",
  "automation-ux.spec.ts",
  "automation-lane-select.spec.ts",
  "comping.spec.ts",
  "groups.spec.ts",
  // A click on a lane's empty space selects the track.
  "lane-click-select.spec.ts",
  // Collapsing the side panes animates the arrangement's size.
  "collapse-panels.spec.ts",
];

export default defineConfig({
  testDir: "e2e",
  // ETHER_E2E_PREBUILD=1: the specs' native binaries are built once, up front (see the file).
  globalSetup: "./e2e/global-setup.ts",
  // Fails the run on Content-Security-Policy violations (preview runs).
  globalTeardown: "./e2e/global-teardown.ts",
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
      name: "webgl",
      testIgnore: TOURS_WALKTHROUGH,
      use: { ...devices["Desktop Chrome"], launchOptions: { args: CHROME_ARGS }, storageState: allCanvas2d ? canvas2dArranger : storage() },
    },
    {
      // The arranger specs again on the Canvas2D fallback.
      name: "canvas2d",
      testMatch: ARRANGER_SPECS,
      use: { ...devices["Desktop Chrome"], launchOptions: { args: CHROME_ARGS }, storageState: canvas2dArranger },
    },
    ...(toursOn
      ? [
          {
            name: "tours",
            testMatch: TOURS_WALKTHROUGH,
            use: { ...devices["Desktop Chrome"], launchOptions: { args: CHROME_ARGS }, storageState: storage() },
          },
        ]
      : []),
  ],
  webServer: {
    // Run vite directly (not through pnpm) so Playwright can terminate it.
    command: `node node_modules/vite/bin/vite.js ${preview ? "preview " : ""}--host 127.0.0.1 --port ${port} --strictPort`,
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: false,
    timeout: 120_000,
  },
});
