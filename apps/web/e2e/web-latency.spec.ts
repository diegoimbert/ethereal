// web-latency (CONTRACTS.md §13.13): plugin delay compensation follows a built-in's latency
// change in the browser. The real wasm engine runs in the AudioWorklet; when the Gate's
// Lookahead knob moves, the worklet reports the node's new latency to the controller Worker,
// whose tick republishes the graph, so the engine's graph latency (PDC included) becomes the
// new lookahead. The graph latency is read from the worklet's tap clock (the f64 header it
// publishes every render quantum, `ui/src/features/collab/host/tapClock.ts`). Before this
// node, the Worker never heard about the change and the latency stayed at its first value.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { addDevice, createTrack, newProject, openDeviceTab, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
}
interface EngineHandle {
  handles(): { tapClock: SharedArrayBuffer } | null;
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

/** `[sample rate, graph latency in samples]` of the last rendered quantum. */
const engineLatency = (page: Page): Promise<[number, number]> =>
  page.evaluate(() => {
    const h = (window as unknown as { __etherEngine: EngineHandle }).__etherEngine.handles();
    if (!h) return [0, -1] as [number, number];
    // tapClock.ts layout: f64 slot 1 = sample rate, slots 2..8 = latest state (latency last).
    const f = new Float64Array(h.tapClock);
    return [f[1]!, f[2 + 5]!] as [number, number];
  });

/** Drag a knob vertically by `dy` pixels (negative = up = increase). */
async function dragKnob(page: Page, knob: Locator, dy: number) {
  await knob.scrollIntoViewIfNeeded();
  const box = (await knob.boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  for (let i = 1; i <= 20; i++) await page.mouse.move(x, y + (dy * i) / 20);
  await page.mouse.up();
}

test("a Gate lookahead change republishes delay compensation in the browser", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Web Latency ${Date.now()}`);

  const audio = await createTrack(page, "Audio");
  await openDeviceTab(page, audio.name);
  await addDevice(page, "Gate");
  await expect
    .poll(async () => Object.values((await doc(page)).devices).filter((d) => d.track === audio.id).length)
    .toBe(1);
  const gate = Object.values((await doc(page)).devices).find((d) => d.track === audio.id)!.id;
  // Lookahead (param 8) defaults to 0: no latency.
  await expect.poll(async () => (await engineLatency(page))[1], { timeout: 10_000 }).toBe(0);
  const [rate] = await engineLatency(page);
  expect(rate).toBeGreaterThan(0);

  // Lookahead all the way up (10 ms): the graph latency follows.
  const lookahead = page.locator(`[data-device="${gate}"]`).getByRole("slider", { name: "Lookahead" });
  await dragKnob(page, lookahead, -600);
  await expect.poll(async () => (await doc(page)).devices[gate]?.params[8], { timeout: 5_000 }).toBe(10);
  await expect.poll(async () => (await engineLatency(page))[1], { timeout: 5_000 }).toBe(Math.round(0.01 * rate));

  // And back down to 0.
  await dragKnob(page, lookahead, 600);
  await expect.poll(async () => (await doc(page)).devices[gate]?.params[8], { timeout: 5_000 }).toBe(0);
  await expect.poll(async () => (await engineLatency(page))[1], { timeout: 5_000 }).toBe(0);

  expect(errors).toEqual([]);
});
