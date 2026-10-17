// "Replace plugin…" (owner request: swap any plugin device's plugin, not only a missing one,
// with "Retain automations and parameters").
//
// 1. Real wasm engine: a REAPER project's "Kontakt 7" device gets values and an automation
//    lane. The web has no plugin host, so the test stands in an installed list (Kontakt 7
//    itself, Kontakt 8, Diva) through the resolver's e2e seam (`window.__etherResolverPlugins`):
//    the device is not missing, yet its header offers "Replace plugin…". Retaining, Kontakt 8
//    (the same plugin family and format) keeps the state, values and lane; one undo step puts
//    Kontakt 7 back. Without retaining, Diva starts at its defaults and the lane goes; undo
//    brings it back.
// 2. Mock engine (`?mock`, where plugins load): Mock Mega → Mock Mini, parameters matched by
//    name; the dialog lists what can't be kept, the new plugin's parameters show.
//
// `REPLACE_SHOTS=<dir>` also writes the PR screenshots (1440x900, dark and light).
import { expect, test, type Page } from "@playwright/test";
import type { Command, Project } from "@/generated";
import { installProbe, sendBatched, ulid } from "./perfProject";
import { launch, openDeviceTab, playButton, showDeviceParams } from "./ui";
import { HAS_IMPORTER, NO_IMPORTER } from "./importer";

const shots = process.env.REPLACE_SHOTS;
test.use({ viewport: { width: 1440, height: 900 } });

const KONTAKT_7 = "0123456789ABCDEF0123456789ABCDEF";
const KONTAKT_8 = "AAAA0000BBBB1111CCCC2222DDDD3333";
const plugin = (format: string, id: string, name: string, vendor: string, version: string) => ({
  format,
  id,
  name,
  vendor,
  version,
  description: "",
  features: ["instrument"],
  category: "Instrument",
  path: `/Library/Audio/Plug-Ins/${format}/${name}`,
  sidechain_inputs: 0,
});
const INSTALLED = [
  plugin("Vst3", KONTAKT_7, "Kontakt 7", "Native Instruments", "7.10.0"),
  plugin("Vst3", KONTAKT_8, "Kontakt 8", "Native Instruments", "8.2.0"),
  plugin("Clap", "com.u-he.diva", "Diva", "u-he", "1.4.7"),
];

const RPP = `<REAPER_PROJECT 0.1 "7.0"
  TEMPO 120 4 4
  <TRACK
    NAME Keys
    <FXCHAIN
      BYPASS 0 0 0
      <VST "VST3i: Kontakt 7 (Native Instruments)" "Kontakt 7.vst3" 0 "" 1234{${KONTAKT_7}} ""
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
    // The web host's "plugins are not available in the browser" toasts are not the subject.
    await page.screenshot({ path: `${shots}/${name}-${theme}.png`, style: ".eth-toast-stack { display: none; }" });
  }
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "dark";
  });
}

async function importSession(page: Page): Promise<void> {
  await page.evaluate((text) => {
    const dt = new DataTransfer();
    dt.items.add(new File([text], "Session.rpp", { type: "text/plain" }));
    const target = document.querySelector(".eth-shell") ?? document.body;
    target.dispatchEvent(new DragEvent("dragover", { dataTransfer: dt, bubbles: true, cancelable: true }));
    target.dispatchEvent(new DragEvent("drop", { dataTransfer: dt, bubbles: true, cancelable: true }));
  }, RPP);
  const report = page.getByRole("dialog", { name: "imported “Session”" });
  await expect(report).toBeVisible({ timeout: 30_000 });
  await report.getByRole("button", { name: "done" }).click();
}

const pluginDevice = (p: Project) => Object.values(p.devices).find((d) => d.kind.type === "Plugin")!;
const pluginId = (p: Project) => {
  const d = pluginDevice(p);
  return d.kind.type === "Plugin" ? d.kind.plugin.plugin_id : null;
};

async function openReplace(page: Page, deviceName: string, menuShot?: string) {
  const header = page.getByTestId("inspector").getByRole("region", { name: deviceName }).locator(".eth-device__header");
  await header.click({ button: "right" });
  if (menuShot) await shot(page, menuShot);
  await page.getByRole("menuitem", { name: "replace plugin…" }).click();
  const dialog = page.getByRole("dialog", { name: `replace “${deviceName}”` });
  await expect(dialog).toBeVisible();
  return dialog;
}

async function choose(page: Page, dialog: ReturnType<Page["getByRole"]>, option: string): Promise<void> {
  // The shared searchable picker: rows are named by plugin ("Diva · CLAP 1.4.7" -> "Diva").
  await dialog.getByRole("button", { name: "replacement plugin" }).click();
  const picker = page.getByRole("dialog", { name: "replacement plugin" });
  await expect(picker.getByRole("searchbox")).toBeVisible();
  await page.mouse.move(0, 0);
  await shot(page, "replace-picker");
  await picker.getByRole("button", { name: option.split(" · ")[0]!, exact: true }).click();
}

// It imports a REAPER project to get a plugin to replace.
test.describe("real engine", () => {
  test.skip(!HAS_IMPORTER, NO_IMPORTER);
  test.beforeEach(async ({ page }) => {
    await page.addInitScript((installed) => {
      (window as unknown as { __etherResolverPlugins: unknown }).__etherResolverPlugins = installed;
    }, INSTALLED);
  });

  test("swap an installed plugin, retaining or not, one undo step each", async ({ page }) => {
    test.setTimeout(120_000);
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await launch(page, "Before");
    await importSession(page);
    await openDeviceTab(page, "Keys");

    // Values and a lane on the device (through the engine).
    await installProbe(page);
    const d = pluginDevice(await project(page));
    const lane = ulid(7, 1);
    const cmds: Command[] = [
      { domain: "Device", command: { type: "SetParam", device: d.id, param: 3, value: 0.25 } },
      {
        domain: "Automation",
        command: { type: "CreateLane", id: lane, owner: { type: "Track", track: d.track }, target: { type: "DeviceParam", device: d.id, param: 3 } },
      },
      {
        domain: "Automation",
        command: { type: "AddPoints", lane, points: [{ id: ulid(7, 2), time: 1, value: 0.75, curve: { type: "Linear" } }] },
      },
    ];
    await sendBatched(page, cmds);
    await expect.poll(async () => Object.keys((await project(page)).automation_lanes)).toContain(lane);
    const before = await project(page);

    // Retain (default): Kontakt 8 is the same plugin family in the same format.
    let dialog = await openReplace(page, "Kontakt 7");
    await expect(dialog.getByRole("switch", { name: "retain automations and parameters" })).toHaveAttribute("aria-checked", "true");
    await choose(page, dialog, "Kontakt 8 · VST3 8.2.0");
    await expect(dialog.getByTestId("replace-summary")).toHaveText("same plugin (or another version of it): its state and every parameter are kept.");
    await page.mouse.move(0, 0);
    await shot(page, "replace-retain");
    await dialog.getByRole("button", { name: "replace", exact: true }).click();
    await expect(dialog).toBeHidden();
    await expect.poll(async () => pluginId(await project(page))).toBe(KONTAKT_8);
    let after = await project(page);
    expect(pluginDevice(after).id).toBe(d.id);
    expect(pluginDevice(after).name).toBe("Kontakt 8");
    expect(pluginDevice(after).order).toBe(d.order);
    expect(pluginDevice(after).params).toEqual(before.devices[d.id]!.params);
    expect(after.automation_lanes[lane]).toEqual(before.automation_lanes[lane]);
    await page.keyboard.press("ControlOrMeta+z");
    await expect.poll(async () => pluginId(await project(page))).toBe(KONTAKT_7);
    expect((await project(page)).devices[d.id]).toEqual(before.devices[d.id]);

    // Without retaining: Diva at its defaults, the lane goes.
    dialog = await openReplace(page, "Kontakt 7");
    await choose(page, dialog, "Diva · CLAP 1.4.7");
    await dialog.getByRole("switch", { name: "retain automations and parameters" }).click();
    await expect(dialog.getByTestId("replace-summary")).toContainText(
      "the new plugin starts at its defaults. 1 automation lane and the parameter mappings are removed",
    );
    await shot(page, "replace-reset");
    await dialog.getByRole("button", { name: "replace", exact: true }).click();
    await expect.poll(async () => pluginId(await project(page))).toBe("com.u-he.diva");
    after = await project(page);
    expect(pluginDevice(after).params).toEqual({});
    expect(after.automation_lanes[lane]).toBeUndefined();
    expect(Object.keys(after.automation_points)).toHaveLength(0);
    await page.keyboard.press("ControlOrMeta+z");
    await expect.poll(async () => pluginId(await project(page))).toBe(KONTAKT_7);
    after = await project(page);
    expect(after.automation_lanes).toEqual(before.automation_lanes);
    expect(after.automation_points).toEqual(before.automation_points);
    expect(after.devices[d.id]).toEqual(before.devices[d.id]);
    expect(errors).toEqual([]);
  });
});

test("mock engine: parameters matched by name, the new plugin loads", async ({ page }) => {
  await page.goto("/?mock");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 15_000 }).toBe(true);
  const p = await project(page);
  const track = Object.values(p.tracks).find((t) => t.kind === "Audio")!;
  await openDeviceTab(page, track.name);
  const id = "01J9" + "0".repeat(21) + "1";
  const lane = "01J9" + "0".repeat(21) + "2";
  await page.evaluate(
    async ([track, id, lane]) => {
      const m = (window as unknown as { __etherMock: { send(c: unknown): Promise<unknown> } }).__etherMock;
      await m.send({
        domain: "Device",
        command: { type: "Insert", id, track, device: { type: "Plugin", plugin_id: "dev.ethereal.mock.mega", format: "Clap", sandboxed: null }, before: null },
      });
      await m.send({ domain: "Device", command: { type: "SetParam", device: id, param: 1, value: 0.4 } });
      await m.send({ domain: "Device", command: { type: "SetParam", device: id, param: 5, value: 0.6 } });
      await m.send({
        domain: "Automation",
        command: { type: "CreateLane", id: lane, owner: { type: "Track", track }, target: { type: "DeviceParam", device: id, param: 1 } },
      });
    },
    [track.id, id, lane] as const,
  );
  const card = page.locator(`section[data-device="${id}"]`);
  await expect(card).toBeVisible();
  const dialog = await openReplace(page, "Mock Mega", "replace-menu");
  await choose(page, dialog, "Mock Mini · CLAP 1.0.0");
  const summary = dialog.getByTestId("replace-summary");
  await expect(summary).toContainText("parameters matched by name: 1 parameter kept.");
  await expect(summary).toContainText("can’t keep: Release 5");
  await page.mouse.move(0, 0);
  await shot(page, "replace-by-name");
  await dialog.getByRole("button", { name: "replace", exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(card).toHaveAttribute("aria-label", "Mock Mini");
  const after = await project(page);
  expect(after.devices[id]!.params).toEqual({ 101: 0.4 });
  expect(after.automation_lanes[lane]!.target).toEqual({ type: "DeviceParam", device: id, param: 101 });
  await showDeviceParams(page, id);
  await expect(card.locator("[data-param]").first()).toBeVisible();
});
