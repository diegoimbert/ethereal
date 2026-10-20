// Perf: dragging a track header's volume fader with many tracks selected moves them
// all (`groupVolumes`). It used to send one `Mixer::SetVolume` per selected track per
// pointer move, uncoalesced: 64 tracks × every move made thousands of transactions, patches
// and store updates, and the faders lagged behind the pointer. Now each animation frame sends
// one `Edit::Batch` for the whole selection.
//
// - Structural, always asserted: commands posted to the engine per pointer move, and the
//   patches it sends back, stay at most one per move (not one per track).
// - Timing, logged: main-thread script time per move and how long the document takes to
//   catch up with the pointer after the last move.
import { expect, test, type Page } from "@playwright/test";
import { launch, newProject } from "./ui";
import { buildPerfProject, doc, metrics } from "./perfProject";

test.use({ viewport: { width: 1440, height: 900 }, trace: "off" });

const TRACKS = 64;
const MOVES = 40;

interface Counters {
  client: number;
  setVolume: number;
  patches: number;
}

/** Counts what the page posts to the engine's controller worker, and the patches it sends back. */
async function countWorkerTraffic(page: Page): Promise<void> {
  await page.evaluate(() => {
    const w = window as unknown as {
      __traffic: Counters;
      __etherEngine: { handles(): { controller: Worker } | null };
    };
    w.__traffic = { client: 0, setVolume: 0, patches: 0 };
    const controller = w.__etherEngine.handles()!.controller;
    const post = controller.postMessage.bind(controller) as (m: unknown, t?: Transferable[]) => void;
    controller.postMessage = ((m: { type?: string; json?: string }, t?: Transferable[]) => {
      if (m?.type === "client" && m.json) {
        w.__traffic.client++;
        w.__traffic.setVolume += m.json.split('"SetVolume"').length - 1;
      }
      post(m, t);
    }) as Worker["postMessage"];
    controller.addEventListener("message", (e: MessageEvent<{ type: string; json?: string }>) => {
      if (e.data.type === "server" && e.data.json) w.__traffic.patches += e.data.json.split('"type":"Patch"').length - 1;
    });
  });
}

const traffic = (page: Page): Promise<Counters> => page.evaluate(() => ({ ...(window as unknown as { __traffic: Counters }).__traffic }));

test(`${TRACKS} selected tracks: a volume drag sends one command per move @frames`, async ({ page }) => {
  test.setTimeout(240_000);
  await launch(page);
  await newProject(page, `Multi volume perf ${Date.now()}`);
  await buildPerfProject(page, TRACKS);

  // Select every track: a click on the first header, a shift-click on the last.
  const headers = page.locator(".eth-arr-header");
  await headers.first().click({ position: { x: 30, y: 8 } });
  await page.locator(".eth-arr__scroll").evaluate((el) => (el.scrollTop = el.scrollHeight));
  await headers.last().click({ position: { x: 30, y: 8 }, modifiers: ["Shift"] });
  await expect(page.locator(".eth-arr-header--selected")).not.toHaveCount(0);
  await page.locator(".eth-arr__scroll").evaluate((el) => (el.scrollTop = 0));

  const fader = page.locator(".eth-arr-vol").first();
  const box = (await fader.boundingBox())!;
  const dragged = await fader.evaluate((el) => el.closest("[data-track]")?.getAttribute("data-track") ?? null);
  const before = await doc(page);

  await countWorkerTraffic(page);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");
  const x0 = box.x + box.width * 0.8;
  const y = box.y + box.height / 2;
  await page.mouse.move(x0, y);
  await page.mouse.down();
  const m0 = await metrics(cdp);
  const t0 = Date.now();
  for (let i = 1; i <= MOVES; i++) await page.mouse.move(x0 - (i * box.width * 0.5) / MOVES, y);
  const m1 = await metrics(cdp);
  const moved = Date.now() - t0;
  // The document catches up: every selected track's volume has moved.
  const target = (p: Awaited<ReturnType<typeof doc>>) =>
    Object.values(p.tracks).filter((t) => t.mixer.volume < (before.tracks[t.id]?.mixer.volume ?? 0) - 1).length;
  await expect.poll(async () => target(await doc(page)), { timeout: 60_000, intervals: [16] }).toBeGreaterThanOrEqual(TRACKS);
  const settled = Date.now() - t0;
  await page.mouse.up();
  const c = await traffic(page);
  console.log(
    `multi-volume perf (${TRACKS} tracks, ${MOVES} moves, dragged ${dragged ?? "?"}): ` +
      `${(c.client / MOVES).toFixed(2)} commands/move, ${(c.setVolume / MOVES).toFixed(1)} SetVolume/move, ${(c.patches / MOVES).toFixed(2)} patches/move; ` +
      `script ${(((m1.script - m0.script) * 1000) / MOVES).toFixed(2)} ms/move, moves took ${moved} ms, document settled after ${settled} ms`,
  );

  // One `Edit::Batch` per frame at most (pointer moves can outpace frames: fewer is fine).
  expect(c.client).toBeLessThanOrEqual(MOVES + 2);
  expect(c.patches).toBeLessThanOrEqual(MOVES + 2);
  // One undo step for the whole drag.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => target(await doc(page))).toBe(0);
});
