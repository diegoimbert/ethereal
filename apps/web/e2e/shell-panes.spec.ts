// The shell's floating panes: pinned panes sit flush with the arrangement (no floating
// inset), a press in the arrangement collapses an unpinned left pane, and the editor
// drawer's left edge tracks the left pane as it opens and closes (no single-frame jump).
//
// `SHELL_PANES_SHOTS=<dir>` also saves the PR screenshots there.
import { expect, test, type Locator, type Page } from "@playwright/test";
import { createTrack, openEditor, selectTrack, launch } from "./ui";

test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

const shots = process.env.SHELL_PANES_SHOTS;
const shot = async (page: Page, name: string) => {
  if (shots) await page.screenshot({ path: `${shots}/${name}.png` });
};

const pane = (page: Page, side: "left" | "right" | "bottom") => page.locator(`[data-pane="${side}"]`);
const stage = (page: Page) => page.locator(".eth-workspace__stage");
const workspace = (page: Page) => page.locator(".eth-workspace");
const rail = (page: Page) => page.getByRole("navigation", { name: "Panels" });
const box = async (l: Locator) => (await l.boundingBox())!;

/** Toggles the Library pane from the rail. */
const library = (page: Page) => rail(page).getByRole("button", { name: "Library", exact: true }).click();

/** Resolves once `l` stops moving (pane transitions done). */
async function settled(l: Locator): Promise<void> {
  let last = "";
  await expect
    .poll(async () => {
      const b = JSON.stringify(await l.boundingBox());
      const same = b === last;
      last = b;
      return same;
    })
    .toBe(true);
}

/** Close enough to line up on screen (sub-pixel rounding). */
const near = (a: number, b: number) => expect(Math.abs(a - b)).toBeLessThanOrEqual(1);

/** A new project with one MIDI track; resolves with the track's name. */
async function start(page: Page): Promise<string> {
  await launch(page, "Panes");
  return (await createTrack(page, "Midi")).name;
}

test("pinned panes sit flush with the arrangement; unpinned ones float inset", async ({ page }) => {
  const track = await start(page);
  await library(page);
  await expect(pane(page, "left")).toBeVisible();
  await settled(pane(page, "left"));
  const ws = await box(workspace(page));
  const r = await box(rail(page));
  // Floating: inset from the workspace edges.
  let left = await box(pane(page, "left"));
  expect(left.y).toBeGreaterThan(ws.y + 4);
  expect(left.x).toBeGreaterThan(r.x + r.width + 4);
  await shot(page, "01-left-floating");

  await page.getByRole("button", { name: "Pin Library" }).click();
  await settled(stage(page));
  await settled(pane(page, "left"));
  left = await box(pane(page, "left"));
  let st = await box(stage(page));
  // Pinned: the same height as the arrangement, flush against the rail.
  near(left.y, st.y);
  near(left.y + left.height, st.y + st.height);
  near(left.x, r.x + r.width);

  // The inspector, pinned: flush with the workspace's right edge, as tall as the arrangement.
  await selectTrack(page, track);
  await expect(pane(page, "right")).toBeVisible();
  await page.getByRole("button", { name: "Pin Inspector" }).click();
  // The drawer, pinned: under the arrangement, its edges lined up with the arrangement's.
  await openEditor(page, "Piano Roll");
  await page.getByRole("button", { name: "Pin Editor" }).click();
  await settled(stage(page));
  await settled(pane(page, "bottom"));
  await settled(pane(page, "right"));
  st = await box(stage(page));
  const right = await box(pane(page, "right"));
  const bottom = await box(pane(page, "bottom"));
  left = await box(pane(page, "left"));
  near(right.y, st.y);
  near(right.x + right.width, ws.x + ws.width);
  near(right.y + right.height, ws.y + ws.height);
  near(left.y + left.height, ws.y + ws.height);
  near(bottom.x, st.x);
  near(bottom.x + bottom.width, st.x + st.width);
  near(bottom.y + bottom.height, ws.y + ws.height);
  await shot(page, "02-all-pinned");

  // Unpinned again: back to floating inset.
  await page.getByRole("button", { name: "Unpin Library" }).click();
  await settled(pane(page, "left"));
  left = await box(pane(page, "left"));
  expect(left.y).toBeGreaterThan(ws.y + 4);
});

test("a press in the arrangement collapses an unpinned left pane, not a pinned one", async ({ page }) => {
  const track = await start(page);
  const lanes = page.locator('[data-feature="arrangement"]');
  const position = page.getByTestId("position-time");

  // The lanes, the ruler and the markers lane each collapse it (the track headers too, in
  // the unit test: the floating pane covers them here).
  for (const target of [lanes, page.locator(".eth-arr__ruler"), page.locator('[data-slot="markers"]')]) {
    await library(page);
    await expect(pane(page, "left")).toBeVisible();
    await settled(pane(page, "left"));
    const b = await box(target);
    // Clear of the floating pane (left) and the inspector (right).
    await page.mouse.click(b.x + b.width / 2, b.y + Math.min(b.height / 2, 8));
    await expect(pane(page, "left")).toHaveCount(0);
  }
  await library(page);
  await settled(pane(page, "left"));
  await shot(page, "03-left-open");
  // The press still does its job: a click on the ruler moves the playhead there.
  const before = (await position.textContent()) ?? "";
  const rb = await box(page.locator(".eth-arr__ruler"));
  await page.mouse.click(rb.x + rb.width / 2 + 200, rb.y + Math.min(rb.height / 2, 8));
  await expect(pane(page, "left")).toHaveCount(0);
  await expect(position).not.toHaveText(before);
  await settled(stage(page));
  await shot(page, "04-left-collapsed");

  // Pinned, it stays.
  await library(page);
  await page.getByRole("button", { name: "Pin Library" }).click();
  await settled(stage(page));
  await selectTrack(page, track);
  const b = await box(lanes);
  await page.mouse.click(b.x + b.width / 2, b.y + b.height - 40);
  await expect(pane(page, "left")).toBeVisible();
});

/**
 * Toggles the left pane (in the page, so sampling starts on the same frame) and samples the
 * drawer's and the left pane's edges on every animation frame for `ms`.
 */
async function sampleToggle(page: Page, ms: number): Promise<{ drawer: number[]; pane: (number | null)[] }> {
  return page.evaluate(async (ms) => {
    const drawer = document.querySelector<HTMLElement>('[data-pane="bottom"]')!;
    const button = document.querySelector<HTMLElement>('nav[aria-label="Panels"] button[aria-label="Library"]')!;
    const out = { drawer: [] as number[], pane: [] as (number | null)[] };
    const sample = () => {
      out.drawer.push(drawer.getBoundingClientRect().left);
      const p = document.querySelector<HTMLElement>('[data-pane="left"]');
      out.pane.push(p ? p.getBoundingClientRect().right : null);
    };
    sample();
    button.click();
    const t0 = performance.now();
    await new Promise<void>((done) => {
      const frame = () => {
        sample();
        if (performance.now() - t0 < ms) requestAnimationFrame(frame);
        else done();
      };
      requestAnimationFrame(frame);
    });
    return out;
  }, ms);
}

/** The edge moved continuously from its first to its last value: several steps, none a jump. */
function expectSmooth(xs: number[]): void {
  const from = xs[0]!;
  const to = xs[xs.length - 1]!;
  const travel = Math.abs(to - from);
  expect(travel).toBeGreaterThan(100);
  const steps = xs.slice(1).map((x, i) => Math.abs(x - xs[i]!));
  // No single-frame jump (a teleport is the whole way in one frame; allow a dropped frame
  // under load), and several in-between positions.
  expect(Math.max(...steps)).toBeLessThan(travel * 0.75);
  expect(new Set(xs.map((x) => Math.round(x))).size).toBeGreaterThanOrEqual(5);
  // Monotonic: it follows the pane one way, never overshooting back.
  const dir = Math.sign(to - from);
  for (const s of xs.slice(1).map((x, i) => x - xs[i]!)) expect(Math.sign(s) * dir).toBeGreaterThanOrEqual(0);
}

test("the drawer's left edge follows the left pane as it collapses and expands @frames", async ({ page }) => {
  await start(page);
  await openEditor(page, "Piano Roll");
  await library(page);
  await settled(pane(page, "left"));
  await settled(pane(page, "bottom"));
  const open = await box(pane(page, "left"));
  const drawerOpen = await box(pane(page, "bottom"));
  // Next to the open pane, clear of it.
  expect(drawerOpen.x).toBeGreaterThan(open.x + open.width);

  // Collapse: the drawer's edge slides left, frame by frame.
  const collapse = await sampleToggle(page, 400);
  expectSmooth(collapse.drawer);
  await expect(pane(page, "left")).toHaveCount(0);
  if (shots) {
    // Mid-collapse frames for the PR: slow the transition down to catch them.
    await library(page);
    await settled(pane(page, "bottom"));
    await page.addStyleTag({ content: ".eth-float, .eth-float--bottom { transition-duration: 1.2s !important; animation-duration: 1.2s !important; }" });
    await rail(page).getByRole("button", { name: "Library", exact: true }).click();
    for (const [i, wait] of [150, 250, 300].entries()) {
      await page.waitForTimeout(wait);
      await shot(page, `05-drawer-mid-collapse-${i + 1}`);
    }
    await page.waitForTimeout(800);
    await page.evaluate(() => document.querySelectorAll("style").forEach((s) => s.textContent?.includes("1.2s") && s.remove()));
    await expect(pane(page, "left")).toHaveCount(0);
  }

  // Expand: the edge slides back right, frame by frame, ending clear of the pane.
  const expand = await sampleToggle(page, 400);
  expectSmooth(expand.drawer);
  await settled(pane(page, "bottom"));
  near((await box(pane(page, "bottom"))).x, drawerOpen.x);

  // Pinned drawer and pinned left pane: the drawer's edge stays lined up with the
  // arrangement's while the pane collapses (both move together).
  await page.getByRole("button", { name: "Pin Library" }).click();
  await page.getByRole("button", { name: "Pin Editor" }).click();
  await settled(stage(page));
  await settled(pane(page, "bottom"));
  const pinned = await page.evaluate(async () => {
    const drawer = document.querySelector<HTMLElement>('[data-pane="bottom"]')!;
    const stage = document.querySelector<HTMLElement>(".eth-workspace__stage")!;
    const button = document.querySelector<HTMLElement>('nav[aria-label="Panels"] button[aria-label="Library"]')!;
    const xs: number[] = [];
    const diffs: number[] = [];
    button.click();
    const t0 = performance.now();
    await new Promise<void>((done) => {
      const frame = () => {
        const d = drawer.getBoundingClientRect().left;
        xs.push(d);
        diffs.push(Math.abs(d - stage.getBoundingClientRect().left));
        if (performance.now() - t0 < 500) requestAnimationFrame(frame);
        else done();
      };
      requestAnimationFrame(frame);
    });
    return { xs, diffs };
  });
  expectSmooth(pinned.xs);
  expect(Math.max(...pinned.diffs)).toBeLessThanOrEqual(1);
});
