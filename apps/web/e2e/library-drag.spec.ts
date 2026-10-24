// Dragging a sample from the library onto the arrangement, from the folder tree and from a
// search result; again with the drag's custom types hidden from the drop target, as Safari
// can do. Also on Playwright's WebKit (a stand-in for Safari): `ETHER_E2E_WEBKIT=1`
// defines the `webkit` project (playwright.config.ts), run against the production bundle
// (`ETHER_E2E_PREVIEW=1`: WebKit refuses the dev server's module workers under
// cross-origin isolation).
//
// No sleeps: every step waits on UI or engine state.
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test as base, type BrowserContextOptions, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { expandFolder, launch, openLibrary } from "./ui";

/**
 * WebKit's ephemeral contexts have no working OPFS (creating a project fails), so on WebKit
 * the page lives in a persistent context of its own, seeded with the project's localStorage.
 */
const test = base.extend<{ page: Page }>({
  page: async ({ page, browserName, playwright, baseURL }, provide, info) => {
    if (browserName !== "webkit") return provide(page);
    const { storageState, viewport } = info.project.use as BrowserContextOptions;
    const dir = await mkdtemp(join(tmpdir(), "ether-webkit-"));
    const ctx = await playwright.webkit.launchPersistentContext(dir, { baseURL, viewport });
    const seed = typeof storageState === "object" ? (storageState.origins?.[0]?.localStorage ?? []) : [];
    await ctx.addInitScript((items) => {
      for (const { name, value } of items) if (localStorage.getItem(name) === null) localStorage.setItem(name, value);
    }, seed);
    try {
      await provide(ctx.pages()[0] ?? (await ctx.newPage()));
    } finally {
      await ctx.close();
      await rm(dir, { recursive: true, force: true });
    }
  },
});

interface Handle {
  state(): { project: Project | null };
}

async function doc(page: Page): Promise<Project> {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
}

const clipCount = async (page: Page) => Object.keys((await doc(page)).clips).length;

/**
 * What Safari can show a drop target of an in-page drag: no custom types in `types` (only
 * the plain text, and sometimes "Files") and no data for them. The app then relies on the
 * drag it keeps in memory (`dragPayload.ts`, "In-page fallback").
 */
function hideCustomDragTypes() {
  const proto = DataTransfer.prototype;
  const types = Object.getOwnPropertyDescriptor(proto, "types")!.get!;
  const getData = proto.getData;
  const custom = (t: string) => t.startsWith("application/x-ethereal");
  Object.defineProperty(proto, "types", {
    configurable: true,
    get(this: DataTransfer) {
      const all = types.call(this) as readonly string[];
      return all.some(custom) ? [...all.filter((t) => !custom(t)), "Files"] : all;
    },
  });
  proto.getData = function (this: DataTransfer, format: string) {
    return custom(format) ? "" : getData.call(this, format);
  };
}

for (const variant of ["", " (custom drag types hidden, as on safari)"]) {
  test(`library drag: a sample from the folder tree and from a search result lands on a track${variant}`, async ({ page }) => {
    if (variant) await page.addInitScript(hideCustomDragTypes);
    await dragTwoSamples(page);
  });
}

async function dragTwoSamples(page: Page) {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.setViewportSize({ width: 1440, height: 900 });

  await launch(page, `Library drag ${Date.now()}`);
  for (let i = 0; i < 2; i++) {
    await page.getByRole("button", { name: "add track", exact: true }).click();
    await page.getByRole("menuitem", { name: "create audio track" }).click();
  }
  await expect.poll(async () => Object.values((await doc(page)).tracks).filter((t) => t.kind === "Audio").length).toBe(2);
  const [first, second] = Object.values((await doc(page)).tracks)
    .filter((t) => t.kind === "Audio")
    .sort((a, b) => (a.order < b.order ? -1 : 1));
  const lane = (id: string) => page.locator(`[data-lane="${id}"]`);

  // --- From the folder tree --------------------------------------------------------------------
  await openLibrary(page);
  await expandFolder(page, "Demo Samples");
  await page.getByRole("treeitem", { name: "Bass Loop 120.wav", exact: true }).dragTo(lane(first!.id), { targetPosition: { x: 5, y: 20 } });
  await expect.poll(() => clipCount(page), { timeout: 20_000 }).toBe(1);

  // --- From a search result ----------------------------------------------------------------------
  const panel = page.locator('[data-feature="browser"][data-browser="v2"]');
  await panel.getByRole("searchbox", { name: "search files" }).fill("loop");
  const row = panel.getByRole("list", { name: "search results" }).getByRole("button", { name: "Chords Loop 120.wav", exact: true });
  await expect(row).toBeVisible({ timeout: 30_000 });
  await row.dragTo(lane(second!.id), { targetPosition: { x: 5, y: 20 } });
  await expect.poll(() => clipCount(page), { timeout: 20_000 }).toBe(2);

  expect(errors).toEqual([]);
}
