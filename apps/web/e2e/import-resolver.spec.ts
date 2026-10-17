// import-resolver: "Resolve missing items" after an import, on the web build.
//
// 1. Real wasm engine: a REAPER project uses "Kontakt 7" (not installed) and a sample from
//    another machine. The web has no plugin host, so the test stands in an installed list
//    ("Kontakt" 8) through the resolver's e2e seam (`window.__etherResolverPlugins`); the
//    engine's deterministic rules propose it, the media search finds Kick.wav in the
//    Factory library, "Apply" fixes both in one undo step.
// 2. Real wasm engine: right-click the missing plugin's device header → "Resolve…" → the
//    dialog focused on it → apply.
// 3. Mock engine (`?mock`, where plugins load): right-click a missing plugin, Resolve…,
//    accept, and the replacement loads (its parameters show).
//
// `RESOLVER_SHOTS=<dir>` also writes the PR screenshots (1440x900, dark and light).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { launch, openDeviceTab, playButton, resolveLater, showDeviceParams } from "./ui";

const shots = process.env.RESOLVER_SHOTS;
test.use({ viewport: { width: 1440, height: 900 } });

const KONTAKT_8 = "AAAA0000BBBB1111CCCC2222DDDD3333";
const INSTALLED = [
  {
    format: "Vst3",
    id: KONTAKT_8,
    name: "Kontakt",
    vendor: "Native Instruments",
    version: "8.2.0",
    description: "",
    features: ["instrument"],
    category: "Instrument",
    path: "/Library/Audio/Plug-Ins/VST3/Kontakt.vst3",
    sidechain_inputs: 0,
  },
  {
    format: "Clap",
    id: "com.u-he.diva",
    name: "Diva",
    vendor: "u-he",
    version: "1.4.7",
    description: "",
    features: ["instrument"],
    category: "Instrument",
    path: "/Library/Audio/Plug-Ins/CLAP/Diva.clap",
    sidechain_inputs: 0,
  },
];

const RPP = `<REAPER_PROJECT 0.1 "7.0"
  TEMPO 120 4 4
  <TRACK
    NAME Keys
    <FXCHAIN
      BYPASS 0 0 0
      <VST "VST3i: Kontakt 7 (Native Instruments)" "Kontakt 7.vst3" 0 "" 1234{0123456789ABCDEF0123456789ABCDEF} ""
      >
    >
  >
  <TRACK
    NAME Drums
    <ITEM
      POSITION 0
      LENGTH 1
      <SOURCE WAVE
        FILE "/Users/someone/Old Machine/Audio/Kick.wav"
      >
    >
  >
>
`;

const project = (page: Page): Promise<Project> =>
  page.evaluate(() => (window as unknown as { __ether: { state(): { project: Project | null } } }).__ether.state().project!);

async function shot(page: Page, name: string): Promise<void> {
  if (!shots) return;
  for (const theme of ["dark", "light"] as const) {
    await page.evaluate((t) => {
      document.documentElement.dataset.theme = t;
    }, theme);
    await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
    await page.screenshot({ path: `${shots}/${name}-${theme}.png` });
  }
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "dark";
  });
}

/** Drop the REAPER project on the window; resolves on its import report. */
async function importSession(page: Page) {
  await page.evaluate((text) => {
    const dt = new DataTransfer();
    dt.items.add(new File([text], "Session.rpp", { type: "text/plain" }));
    const target = document.querySelector(".eth-shell") ?? document.body;
    target.dispatchEvent(new DragEvent("dragover", { dataTransfer: dt, bubbles: true, cancelable: true }));
    target.dispatchEvent(new DragEvent("drop", { dataTransfer: dt, bubbles: true, cancelable: true }));
  }, RPP);
  const report = page.getByRole("dialog", { name: "imported “Session”" });
  await expect(report).toBeVisible({ timeout: 30_000 });
  return report;
}

const kontakt = (p: Project) => Object.values(p.devices).find((d) => d.kind.type === "Plugin")!;
const pluginId = (p: Project) => {
  const d = kontakt(p);
  return d.kind.type === "Plugin" ? d.kind.plugin.plugin_id : null;
};
const kick = (p: Project) => Object.values(p.media).find((m) => m.name === "Kick.wav")!;

test.beforeEach(async ({ page }) => {
  await page.addInitScript((installed) => {
    (window as unknown as { __etherResolverPlugins: unknown }).__etherResolverPlugins = installed;
  }, INSTALLED);
});

test("after an import: deterministic matches for a missing plugin and sample, applied in one undo step", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await launch(page, "Before");
  const report = await importSession(page);
  const resolve = report.getByRole("button", { name: "resolve missing devices" });
  await expect(resolve).toBeVisible({ timeout: 15_000 });
  await expect(report.getByRole("button", { name: "resolve missing samples" })).toBeVisible();
  await shot(page, "resolver-report");
  await resolve.click();

  // The devices button opens the resolver on devices only; "Show all" brings the samples in.
  const devicesOnly = page.getByRole("dialog", { name: "resolve missing devices" });
  await expect(devicesOnly).toBeVisible();
  await expect(devicesOnly.getByTestId("resolver-media")).toHaveCount(0);
  await devicesOnly.getByRole("button", { name: /show all 2 missing items/ }).click();
  const dialog = page.getByRole("dialog", { name: "resolve missing items" });
  await expect(dialog).toBeVisible();
  const plugin = dialog.getByTestId("resolver-plugin");
  await expect(plugin).toContainText("Kontakt 7");
  await expect(plugin).toContainText("same family");
  await expect(plugin).toContainText("Kontakt 7 → Kontakt");
  await expect(plugin.getByRole("combobox", { name: "Replacement for Kontakt 7" })).toContainText("Kontakt · vst3 8.2.0");
  // The sample: found in the browser's Factory library by name.
  const sample = dialog.getByTestId("resolver-media");
  await expect(sample).toContainText("Kick.wav");
  await expect(sample.getByRole("combobox", { name: "File for Kick.wav" })).toContainText("factory/", { timeout: 30_000 });
  await expect(sample).toContainText("same file name");
  await page.mouse.move(0, 0);
  await shot(page, "resolver-dialog");

  const before = await project(page);
  expect(pluginId(before)).toBe("0123456789ABCDEF0123456789ABCDEF");
  expect(kick(before).location.type).toBe("External");
  await dialog.getByRole("button", { name: "apply 2 changes" }).click();
  await expect(dialog).toBeHidden();
  await expect.poll(async () => pluginId(await project(page))).toBe(KONTAKT_8);
  const after = await project(page);
  expect(kontakt(after).name).toBe("Kontakt");
  // The web engine can't reference OS files: the library file is copied into the project.
  expect(kick(after).location).toEqual({ type: "Project" });
  expect(kick(after).hash).not.toBeNull();

  // One undo step reverts both.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => pluginId(await project(page))).toBe("0123456789ABCDEF0123456789ABCDEF");
  expect(kick(await project(page)).location.type).toBe("External");
  expect(errors).toEqual([]);
});

test("right-click a missing plugin's device: Resolve…", async ({ page }) => {
  test.setTimeout(120_000);
  await launch(page, "Before");
  const report = await importSession(page);
  await report.getByRole("button", { name: "done" }).click();
  // Closing the report lists what is missing: later, from the device.
  await resolveLater(page);
  await openDeviceTab(page, "Keys");
  const header = page.getByTestId("inspector").getByRole("region", { name: "Kontakt 7" }).locator(".eth-device__header");
  await header.click({ button: "right" });
  await shot(page, "resolver-device-menu");
  await page.getByRole("menuitem", { name: "resolve…" }).click();
  const dialog = page.getByRole("dialog", { name: "resolve “Kontakt 7”" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId("resolver-plugin")).toHaveCount(1);
  await expect(dialog.getByTestId("resolver-media")).toHaveCount(0);
  await shot(page, "resolver-focused");
  await dialog.getByRole("button", { name: "apply 1 change" }).click();
  await expect(dialog).toBeHidden();
  await expect.poll(async () => pluginId(await project(page))).toBe(KONTAKT_8);
  await expect(page.getByTestId("inspector").getByRole("region", { name: "Kontakt" })).toBeVisible();
});

test("mock engine: a resolved plugin loads", async ({ page }) => {
  await page.goto("/?mock");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 15_000 }).toBe(true);
  const p = await project(page);
  const track = Object.values(p.tracks).find((t) => t.kind === "Audio")!;
  await openDeviceTab(page, track.name);
  const id = await page.evaluate(
    (t) => (window as unknown as { __etherMock: { resolver: { addMissingPlugin(t: string): string } } }).__etherMock.resolver.addMissingPlugin(t),
    track.id,
  );
  const card = page.locator(`section[data-device="${id}"]`);
  await expect(card).toBeVisible();
  await card.locator(".eth-device__header").click({ button: "right" });
  await page.getByRole("menuitem", { name: "resolve…" }).click();
  const dialog = page.getByRole("dialog", { name: "resolve “Mock Mega 1”" });
  await expect(dialog.getByTestId("resolver-plugin")).toContainText("same family");
  await dialog.getByRole("button", { name: "accept all" }).click();
  await expect(dialog).toBeHidden();
  // Mock Mega's parameters load (the card's inline list).
  await showDeviceParams(page, id);
  await expect(card.locator("[data-param]").first()).toBeVisible();
  await expect(card).toHaveAttribute("aria-label", "Mock Mega");
});
