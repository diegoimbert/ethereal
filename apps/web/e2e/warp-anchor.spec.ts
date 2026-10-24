// Warp edits that must not move anything else (owner bugs), on the web build through the UI
// against the real engine (WasmTransport → controller Worker → AudioWorklet):
//
// 1. Enabling warp on a trimmed clip at another tempo (song 100 BPM, the 120 BPM loop's
//    start trimmed by 2 beats) keeps the clip where it is: same arrangement rect, same
//    source second under the clip start; off again restores the unwarped window.
// 2. Dragging the marker between two others only re-times the audio between them: the
//    other markers keep their screen x, the editor's scale doesn't change and the waveform
//    outside the two neighbours stays pixel-identical during and after the drag. It can't
//    cross a neighbour; the drag is one undo step. Dragging the end marker (which moves
//    the media end) keeps the scale too until release.
//
// No sleeps: every step waits on UI or engine state. `E2E_SHOTS=<dir>` saves PR stills.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Clip, Project, WarpMarker } from "@/generated";
import { arrangerApi, clipPart, openClip } from "./clips";
import { createTrack, expandFolder, launch, openLibrary, pickOption, setNumberField } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const clipOf = async (page: Page, id: string): Promise<Clip> => (await doc(page)).clips[id]!;

const markersOf = async (page: Page, clip: string): Promise<WarpMarker[]> =>
  Object.values((await doc(page)).warp_markers)
    .filter((m) => m.clip === clip)
    .sort((a, b) => a.beat - b.beat);

/** Source seconds the engine reads at content beat `c` (piecewise linear, edge slopes). */
function sourceAt(c: number, markers: WarpMarker[] | null, songBpm: number): number {
  if (!markers || markers.length < 2) return (c * 60) / songBpm;
  let i = markers.findIndex((m) => m.beat > c);
  i = Math.min(Math.max(i < 0 ? markers.length : i, 1), markers.length - 1);
  const a = markers[i - 1]!;
  const z = markers[i]!;
  return a.source + ((c - a.beat) * (z.source - a.source)) / (z.beat - a.beat);
}

/** Seconds under the clip start, as the engine plays it. */
async function startSource(page: Page, id: string): Promise<number> {
  const c = await clipOf(page, id);
  if (c.content.type !== "Audio") throw new Error("not audio");
  const markers = c.content.warp.enabled ? await markersOf(page, id) : null;
  return +sourceAt(c.offset, markers, 100).toFixed(6);
}

const r3 = (x: number) => +x.toFixed(3);
const rectOf = async (page: Page, id: string) => {
  const r = (await arrangerApi(page, "clipRect", id))!;
  return [r3(r.x), r3(r.width)];
};

/** A hash of the waveform canvas columns in [x0, x1) (CSS px). */
const columnsHash = (canvas: Locator, x0: number, x1: number) =>
  canvas.evaluate(
    (c: HTMLCanvasElement, [a, b]) => {
      const ctx = c.getContext("2d")!;
      const k = c.width / c.getBoundingClientRect().width;
      const left = Math.ceil(a * k);
      const w = Math.max(1, Math.floor(b * k) - left);
      const data = ctx.getImageData(left, 0, w, c.height).data;
      let h = 0;
      for (let i = 0; i < data.length; i++) h = (h * 31 + data[i]!) | 0;
      return h;
    },
    [x0, x1] as const,
  );

const shots = process.env.E2E_SHOTS;
const shot = async (target: Page | Locator, name: string) => {
  if (shots) await target.screenshot({ path: `${shots}/${name}.png` });
};

test("warp on keeps the clip's start; a middle marker only moves its own segments", async ({ page }) => {
  test.setTimeout(150_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page, `Warp anchor ${Date.now()}`);
  const tempo = page.getByRole("toolbar", { name: "transport" }).getByLabel("tempo", { exact: true });
  await tempo.click();
  await setNumberField(tempo, 100);
  await expect.poll(async () => Object.values((await doc(page)).tempo_points)[0]?.bpm).toBe(100);

  // --- An unwarped 120 BPM loop at 100 BPM, its start trimmed ------------------------------
  const audio = await createTrack(page, "Audio");
  await openLibrary(page);
  await expandFolder(page, "Demo Samples");
  const sample = page.getByRole("treeitem", { name: "Bass Loop 120.wav", exact: true });
  await sample.dragTo(page.locator(`[data-lane="${audio.id}"]`), { targetPosition: { x: 5, y: 20 } });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length, { timeout: 20_000 }).toBe(1);
  const id = Object.values((await doc(page)).clips)[0]!.id;
  const px = (await clipPart(page, id)).width / (await clipOf(page, id)).length;
  const h = await clipPart(page, id, "resize-start");
  await page.mouse.move(h.x + h.width / 2, h.y + h.height - 4);
  await page.mouse.down();
  await page.mouse.move(h.x + h.width / 2 + px, h.y + h.height - 4, { steps: 3 });
  await page.mouse.move(h.x + h.width / 2 + 2 * px, h.y + h.height - 4, { steps: 3 });
  await page.mouse.up();
  await expect.poll(async () => r3((await clipOf(page, id)).offset)).toBe(2);
  const unwarped = await clipOf(page, id);
  const rect = await rectOf(page, id);
  const s0 = await startSource(page, id);
  expect(s0).toBe(1.2); // 2 beats at 100 BPM

  // --- 1. Warp on: same rect, same audio under the clip start --------------------------------
  await openClip(page, id);
  const editor = page.getByTestId("warp-editor");
  await expect(editor).toHaveAttribute("data-clip-id", id);
  // The mode list: "none" is warp off; the clip's own mode warps it.
  const warpOf = async () => ((await clipOf(page, id)).content as Extract<Clip["content"], { type: "Audio" }>).warp;
  const warpOn = async () => pickOption(editor, "warp mode", { value: (await warpOf()).mode });
  const warpOff = async () => pickOption(editor, "warp mode", "none");
  await page.mouse.move(0, 0);
  await shot(page, "warp-on-before");
  await warpOn();
  await expect.poll(async () => (await warpOf()).enabled).toBe(true);
  await expect.poll(async () => (await markersOf(page, id)).length).toBe(2);
  const warped = await clipOf(page, id);
  expect([warped.start, warped.length]).toEqual([unwarped.start, unwarped.length]);
  expect(await startSource(page, id)).toBe(s0);
  expect(r3(warped.offset)).toBe(2.4); // 1.2 s at the loop's 120 BPM
  await expect.poll(() => rectOf(page, id)).toEqual(rect);
  await shot(page, "warp-on-after");

  // Off: back to the unwarped window.
  await warpOff();
  await expect.poll(async () => (await warpOf()).enabled).toBe(false);
  await expect.poll(async () => r3((await clipOf(page, id)).offset)).toBe(2);
  expect(await startSource(page, id)).toBe(s0);
  await warpOn();
  await expect.poll(async () => (await warpOf()).enabled).toBe(true);
  await expect.poll(async () => r3((await clipOf(page, id)).offset)).toBe(2.4);

  // --- 2. Markers at 0 | 1 | 2 | 4 | 8 (end): drag the one at 2 ---------------------------
  const view = page.getByTestId("warp-view");
  const ppb = Number(await view.getAttribute("data-px-per-beat"));
  const box = (await view.boundingBox())!;
  for (const beat of [1, 2, 4]) {
    const n = (await markersOf(page, id)).length;
    await view.dblclick({ position: { x: beat * ppb, y: box.height / 2 } });
    await expect.poll(async () => (await markersOf(page, id)).length).toBe(n + 1);
  }
  const all = await markersOf(page, id);
  const mid = all.find((m) => Math.abs(m.beat - 2) < 1e-9)!;
  const others = all.filter((m) => m.id !== mid.id);
  const handle = (m: WarpMarker) => page.locator(`[data-testid="warp-marker"][data-marker-id="${m.id}"] path`);
  const xs = () => Promise.all(others.map(async (m) => r3((await handle(m).boundingBox())!.x)));
  const xs0 = await xs();
  const canvas = page.getByTestId("warp-waveform");
  // The waveform outside the neighbours (before 1, after 4) never changes.
  const tail = async () => [await columnsHash(canvas, 0, ppb - 2), await columnsHash(canvas, 4 * ppb + 2, box.width - 2)];
  const tail0 = await tail();
  await page.mouse.move(0, 0);
  await shot(editor, "marker-drag-before");

  const hb = (await handle(mid).boundingBox())!;
  const y = hb.y + hb.height / 2;
  await page.mouse.move(hb.x + hb.width / 2, y);
  await page.mouse.down();
  for (const dx of [0.5, 1, 1.5]) {
    await page.mouse.move(hb.x + hb.width / 2 + dx * ppb, y);
    await expect.poll(async () => (await doc(page)).warp_markers[mid.id]!.beat).toBe(2 + dx);
    expect(Number(await view.getAttribute("data-px-per-beat"))).toBe(ppb);
    expect(await xs()).toEqual(xs0);
    expect(await tail()).toEqual(tail0);
  }
  await shot(editor, "marker-drag-during");
  // Past the next marker: clamped short of it.
  await page.mouse.move(hb.x + hb.width / 2 + 5 * ppb, y);
  await expect.poll(async () => (await doc(page)).warp_markers[mid.id]!.beat).toBeLessThan(4);
  expect((await doc(page)).warp_markers[mid.id]!.beat).toBeGreaterThan(3.9);
  await page.mouse.move(hb.x + hb.width / 2 + ppb, y);
  await expect.poll(async () => (await doc(page)).warp_markers[mid.id]!.beat).toBe(3);
  await page.mouse.up();
  expect(await xs()).toEqual(xs0);
  expect(await tail()).toEqual(tail0);
  const after = await markersOf(page, id);
  expect(after.filter((m) => m.id !== mid.id).map((m) => [m.beat, m.source])).toEqual(others.map((m) => [m.beat, m.source]));
  expect((await doc(page)).warp_markers[mid.id]!.source).toBe(mid.source);
  await page.mouse.move(0, 0);
  await shot(editor, "marker-drag-after");

  // One undo step.
  await page.getByRole("button", { name: "undo" }).click();
  await expect.poll(async () => (await doc(page)).warp_markers[mid.id]?.beat).toBe(2);

  // The end marker moves the media end: the view keeps its scale during the drag (the
  // other markers stay put on screen), and zooms out only once it is released.
  const last = all.at(-1)!;
  const rest = all.filter((m) => m.id !== last.id);
  const restXs = () => Promise.all(rest.map(async (m) => r3((await handle(m).boundingBox())!.x)));
  const restXs0 = await restXs();
  const lb = (await handle(last).boundingBox())!;
  await page.mouse.move(lb.x + lb.width / 2, lb.y + lb.height / 2);
  await page.mouse.down();
  for (const dx of [0.5, 1, 1.5]) {
    await page.mouse.move(lb.x + lb.width / 2 + dx * ppb, lb.y + lb.height / 2);
    await expect.poll(async () => (await doc(page)).warp_markers[last.id]!.beat).toBe(last.beat + dx);
    expect(Number(await view.getAttribute("data-px-per-beat"))).toBe(ppb);
    expect(await restXs()).toEqual(restXs0);
  }
  await page.mouse.up();
  await expect.poll(async () => Number(await view.getAttribute("data-px-per-beat"))).toBeLessThan(ppb);

  expect(errors).toEqual([]);
});
