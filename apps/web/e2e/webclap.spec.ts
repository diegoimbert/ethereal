// WebCLAP in the web version, against the real engine: install the repository's test plugin
// (crates/ether-webclap-fixture) from a file and Signalsmith Basics (a bundle with web uis)
// from an address, play the test synth through the test gain, hear it on the meter, follow a
// param change, open the gain's web ui (a sandboxed frame: no access to the app) and edit
// from it, render offline with automation, and reopen the project with its state.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { expect, test, type Frame, type Page } from "@playwright/test";
import type { Project } from "@/generated";
// @ts-expect-error: a plain .mjs build script
import { buildFixture } from "../scripts/build-webclap-fixture.mjs";
import { doc, installProbe, send, sendBatched, ulid } from "./perfProject";
import { deviceCard, launch, newProject, openDeviceTab, openDeviceWindow, openOnLaunch, pickOption, projectMenu, settingsPage } from "./ui";

test.use({ viewport: { width: 1440, height: 900 } });

const BASICS_URL = "https://plugins.example/basics.wclap.tar.gz";
const basics = fileURLToPath(new URL("../test-fixtures/webclap/examples/signalsmith-basics/basics.wclap.tar.gz", import.meta.url));

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
}

const peak = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: Handle }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

const param = async (page: Page, device: string, id: number) => (await doc(page)).devices[device]?.params[id];

async function openPluginSettings(page: Page) {
  await page.getByRole("button", { name: "settings", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "settings" });
  await settingsPage(dialog, "plugins");
  await expect(dialog.locator('[data-feature="webclap-settings"]')).toBeVisible();
  return dialog;
}

/** The sandboxed frame of the plugin ui in `device`'s window. */
async function uiFrame(page: Page, device: string): Promise<Frame> {
  const el = page.locator(`[data-device-window="${device}"] iframe`);
  await expect(el).toHaveAttribute("sandbox", "allow-scripts");
  let frame: Frame | null = null;
  await expect
    .poll(async () => {
      frame = await (await el.elementHandle())!.contentFrame();
      return frame ? frame.evaluate(() => (document.getElementById("half") ? "loaded" : "")).catch(() => "") : "";
    })
    .not.toBe("");
  return frame!;
}

test("webclap plugins: install, play, edit from their ui, render offline, reopen", async ({ page }) => {
  test.setTimeout(300_000);
  const fixture = buildFixture({ ifMissing: true }) as string;
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  // A plugin served from another origin, with CORS (what a plugin host must send).
  await page.route(BASICS_URL, (route) =>
    route.fulfill({ body: readFileSync(basics), headers: { "content-type": "application/gzip", "access-control-allow-origin": "*" } }),
  );

  await launch(page);
  const name = `Webclap ${Date.now()}`;
  await newProject(page, name);
  await installProbe(page);

  // ── Install: a file, then an address ──
  const settings = await openPluginSettings(page);
  await settings.getByTestId("webclap-file").setInputFiles(fixture);
  const fixtureRow = settings.locator("[data-webclap-module]").filter({ hasText: "ether-test.wclap.wasm" });
  await expect(fixtureRow).toContainText("3 plugins", { timeout: 30_000 });
  await expect(fixtureRow).toContainText("Test Gain");
  await settings.getByRole("textbox", { name: "address of a webclap plugin (.wasm or .wclap.tar.gz)" }).fill(BASICS_URL);
  await settings.getByRole("button", { name: "install", exact: true }).click();
  await expect(settings.locator("[data-webclap-module]").filter({ hasText: "basics.wclap.tar.gz" })).toContainText("plugins", { timeout: 30_000 });
  await page.keyboard.press("Escape");

  // ── A synth (velocity as DC) through a gain ──
  const track = ulid(1, 0);
  const synth = ulid(8, 0);
  const gain = ulid(8, 1);
  const clip = ulid(3, 0);
  const plugin = (plugin_id: string) => ({ type: "Plugin" as const, plugin_id, format: "Webclap" as const, sandboxed: null });
  await sendBatched(page, [
    { domain: "Track", command: { type: "Create", id: track, kind: "Midi", name: "Web", color: null, parent: null, before: null } },
    { domain: "Device", command: { type: "Insert", id: synth, track, device: plugin("ether.test.synth"), before: null } },
    { domain: "Device", command: { type: "Insert", id: gain, track, device: plugin("ether.test.gain"), before: null } },
    { domain: "Clip", command: { type: "CreateMidi", id: clip, track, start: 0, length: 16, name: null } },
    { domain: "Note", command: { type: "Add", clip, notes: [{ id: ulid(4, 0), pitch: 60, velocity: 127, start: 0, duration: 16 }] } },
  ]);
  const p = await doc(page);
  expect(p.devices[synth]?.name).toBe("Test Synth");
  expect(p.devices[gain]?.kind).toMatchObject({ type: "Plugin", plugin: { format: "Webclap", plugin_id: "ether.test.gain" } });
  // Mirrored from the instance: the gain's params.
  await expect.poll(() => param(page, gain, 1)).toBe(1);
  await expect(deviceCard(page, gain).getByText("missing")).toHaveCount(0);

  // ── Hear it, then follow a param change ──
  await send(page, { domain: "Transport", command: { type: "Play" } });
  await expect.poll(() => peak(page, track), { timeout: 15_000 }).toBeGreaterThan(0.5);
  await send(page, { domain: "Device", command: { type: "SetParam", device: gain, param: 1, value: 0.25 } });
  await expect.poll(() => peak(page, track), { timeout: 15_000 }).toBeLessThan(0.4);
  await send(page, { domain: "Transport", command: { type: "Stop" } });

  // ── Its web ui: sandboxed, wired to the plugin by messages ──
  await openDeviceTab(page, "Web");
  const win = await openDeviceWindow(page, gain);
  const frame = await uiFrame(page, gain);
  await expect.poll(() => frame.evaluate(() => document.body.dataset.last)).toBe("gain:0.25");
  // No way into the app from there: an opaque origin (no storage, no cookies, no parent).
  expect(
    await frame.evaluate(() => {
      const tries = [() => localStorage.length, () => document.cookie, () => parent.document.title, () => indexedDB.open("x")];
      return tries.map((f) => {
        try {
          f();
          return "open";
        } catch {
          return "blocked";
        }
      });
    }),
  ).toEqual(["blocked", "blocked", "blocked", "blocked"]);
  expect(await frame.evaluate(() => origin)).toBe("null");
  // An edit in the ui is the plugin's: an undoable param edit in the document.
  await frame.locator("#half").click();
  await expect.poll(() => param(page, gain, 1), { timeout: 15_000 }).toBe(0.5);
  await expect.poll(() => frame.evaluate(() => document.body.dataset.last)).toBe("gain:0.5");
  await win
    .locator(":scope > header")
    .getByRole("button", { name: /^close .* window$/ })
    .click();

  // ── A bundle's ui (Signalsmith Basics): its page and scripts come from the bundle ──
  const basicsId = await page.evaluate(
    () =>
      (window as unknown as { __etherEngine: { webclap: { modules(): { name: string; plugins: { id: string }[] }[] } } }).__etherEngine.webclap
        .modules()
        .find((m) => m.name === "basics.wclap.tar.gz")!.plugins[0]!.id,
  );
  const fx = ulid(8, 2);
  await send(page, { domain: "Device", command: { type: "Insert", id: fx, track, device: plugin(basicsId), before: null } });
  const fxWin = await openDeviceWindow(page, fx);
  const fxEl = fxWin.locator("iframe");
  await expect(fxEl).toHaveAttribute("sandbox", "allow-scripts");
  const title = async () => {
    const f = await (await fxEl.elementHandle())?.contentFrame();
    return f ? f.evaluate(() => document.title).catch(() => "") : "";
  };
  await expect.poll(title, { timeout: 15_000 }).toBe("Generic STFX UI");
  await fxWin
    .locator(":scope > header")
    .getByRole("button", { name: /^close .* window$/ })
    .click();
  await send(page, { domain: "Device", command: { type: "Remove", id: fx } });

  // ── Offline render with automation: gain 1 until beat 4, then 0.25 ──
  const lane = ulid(0, 0);
  await sendBatched(page, [
    {
      domain: "Automation",
      command: { type: "CreateLane", id: lane, owner: { type: "Track", track }, target: { type: "DeviceParam", device: gain, param: 1 } },
    },
    {
      domain: "Automation",
      command: {
        type: "AddPoints",
        lane,
        points: [
          { id: ulid(2, 0), time: 0, value: 0.5, curve: { type: "Step" } },
          { id: ulid(2, 1), time: 4, value: 0.125, curve: { type: "Step" } },
        ],
      },
    },
  ]);
  await projectMenu(page, "export audio…");
  const dialog = page.getByRole("dialog", { name: "export audio" });
  await pickOption(dialog, "range", { value: "loop" });
  await pickOption(dialog, "format", { value: "Wav" });
  await pickOption(dialog, "bit depth", { value: "Int16" });
  await dialog.getByLabel("file name").fill("webclap render");
  const download = page.waitForEvent("download", { timeout: 120_000 });
  await dialog.getByRole("button", { name: "export", exact: true }).click();
  const wav = readFileSync(await (await download).path());
  await dialog.getByRole("button", { name: "close" }).click();
  const rate = wav.readUInt32LE(24);
  const data = wav.indexOf("data", 12, "latin1") + 8;
  const at = (seconds: number) => wav.readInt16LE(data + Math.round(seconds * rate) * 4) / 32767;
  // Beat 2 (1 s) at gain 1, beat 6 (3 s) at 0.25: the synth's DC is its velocity (1.0).
  expect(at(1)).toBeGreaterThan(0.5);
  expect(at(3)).toBeGreaterThan(0.05);
  expect(at(3)).toBeLessThan(0.4);
  expect(at(3) / at(1)).toBeCloseTo(0.25, 1);

  // ── Save and reopen: the plugins load again with their state (`mode`, a stepped param
  // not automated, comes back from it: values are mirrored from the new instance) ──
  await send(page, { domain: "Device", command: { type: "SetParam", device: gain, param: 2, value: 2 } });
  await send(page, { domain: "Project", command: { type: "Save" } });
  await page.reload();
  await openOnLaunch(page, name);
  await expect.poll(async () => (await doc(page)).devices[gain]?.params[2], { timeout: 30_000 }).toBe(2);
  await expect(deviceCard(page, gain).getByText("missing")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("inserting a native plugin in the web version fails cleanly", async ({ page }) => {
  await launch(page);
  await installProbe(page);
  const track = ulid(1, 1);
  await send(page, { domain: "Track", command: { type: "Create", id: track, kind: "Audio", name: "Native", color: null, parent: null, before: null } });
  // Inserting a desktop plugin fails cleanly (the browser can't load it).
  const reply = await page.evaluate(
    (args: [string, string]) =>
      new Promise<string>((resolve) => {
        const [track, id] = args;
        const ep = (window as unknown as { __etherEngine: { handles(): { controller: Worker } | null; post(json: string): void } }).__etherEngine;
        const reqId = 1_999_000_001;
        ep.handles()!.controller.addEventListener("message", (e: MessageEvent<{ type: string; json?: string }>) => {
          if (e.data.type !== "server" || !e.data.json) return;
          for (const m of JSON.parse(e.data.json) as { kind: string; body: { id: number; result: { status: string; error?: { message: string } } } }[]) {
            if (m.kind === "Reply" && m.body.id === reqId) resolve(m.body.result.error?.message ?? m.body.result.status);
          }
        });
        ep.post(
          JSON.stringify({
            id: reqId,
            gesture: null,
            command: {
              domain: "Device",
              command: { type: "Insert", id, track, device: { type: "Plugin", plugin_id: "com.u-he.diva", format: "Clap", sandboxed: null }, before: null },
            },
          }),
        );
      }),
    [track, ulid(8, 99)] as [string, string],
  );
  expect(reply).toContain("not available in the browser");
});
