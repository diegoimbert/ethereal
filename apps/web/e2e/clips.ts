// Shared e2e gestures on arrangement clips. Import from here instead of re-deriving the
// gesture in each spec, so a UX change to how clips take the pointer is fixed in one place.
//
// The arranger draws its clips on one canvas (WebGL2, or the Canvas2D fallback): there is no
// element per clip, so the helpers aim through the arranger's test API
// (`window.__ether.arrangement`: clip geometry, hit test, the clips on screen). The
// Playwright project `webgl` runs the specs on WebGL (the default), `canvas2d` (or
// `ETHER_E2E_ARRANGER=canvas2d`) forces the fallback (`localStorage["eth.arr.renderer"]`).
import { inflateSync } from "node:zlib";
import { expect, type Page } from "@playwright/test";

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export type FadeHandle = "fade-in" | "fade-out" | "fade-in-curve" | "fade-out-curve";
export type ClipPart = "box" | "header" | "body" | "resize-start" | "resize-end" | FadeHandle;

/** A clip's in-clip editing overlays as drawn: fade curves, crossfade regions, title badges. */
export interface ClipDecor {
  lines: number;
  crossfadeIn: boolean;
  crossfadeOut: boolean;
  badges: string[];
}

/** The backend drawing the arranger. */
export type ArrangerBackend = "webgl" | "canvas2d";

interface Api {
  renderer(): ArrangerBackend | null;
  clipRect(id: string): Rect | null;
  handleRect(id: string, part: ClipPart): Rect | null;
  /** The text box of a clip's name (its line box; `width`: its room), client px. */
  nameRect(id: string): Rect | null;
  hitTest(x: number, y: number): { clip: string; part: string } | null;
  visibleClipIds(): string[];
  expectedVisibleClipIds(at?: "drawn"): string[];
  bodyInk(id: string): number;
  overInk(id: string): number;
  clipDecor(id: string): ClipDecor | null;
  stats(): unknown;
  loseContext(): boolean;
  peerMarks(id: string): { ring: [number, number, number] | null; editors: string[] } | null;
  newTrackGhostClipIds(): string[];
}

type ApiCall<K extends keyof Api> = Api[K] extends (...args: infer A) => infer R ? { args: A; result: R } : never;

/** Calls the arranger test API in the page. */
export function arrangerApi<K extends keyof Api>(page: Page, method: K, ...args: ApiCall<K>["args"]): Promise<ApiCall<K>["result"]> {
  return page.evaluate(
    ([m, a]) => {
      const api = (window as unknown as { __ether: { arrangement: Record<string, (...x: unknown[]) => unknown> } }).__ether.arrangement;
      return api[m as string]!(...(a as unknown[]));
    },
    [method, args] as const,
  ) as Promise<ApiCall<K>["result"]>;
}

/** The renderer in use (null before the arranger mounts). */
export async function arrangement(page: Page): Promise<{ renderer: ArrangerBackend | null }> {
  return { renderer: await arrangerApi(page, "renderer") };
}

/**
 * The backend a Playwright project runs the arranger on: `canvas2d` (or a project derived from
 * it, `canvas2d-<suffix>`, as a config built over this one names its own) forces the fallback.
 */
export function projectBackend(projectName: string): ArrangerBackend {
  return /^canvas2d(-|$)/.test(projectName) || process.env.ETHER_E2E_ARRANGER === "canvas2d" ? "canvas2d" : "webgl";
}

/** Checks the backend the spec runs on (`projectBackend`). */
export async function expectProjectRenderer(page: Page, projectName: string): Promise<void> {
  const want = projectBackend(projectName);
  await expect.poll(() => arrangerApi(page, "renderer")).toBe(want);
}

/** A clip's fade/crossfade overlay and badges as drawn (null until the clip is drawn). */
export function clipDecor(page: Page, clipId: string): Promise<ClipDecor | null> {
  return arrangerApi(page, "clipDecor", clipId);
}

/**
 * Switches the arranger's backend at runtime (the hidden debug override: "canvas2d" forces
 * the fallback, "webgl" goes back to the automatic choice), and waits for it.
 */
export async function setArrangerRenderer(page: Page, renderer: ArrangerBackend): Promise<void> {
  await page.evaluate((r) => (window as unknown as { __ether: { setArrangerRenderer(r: string): void } }).__ether.setArrangerRenderer(r), renderer);
  await expect.poll(() => arrangerApi(page, "renderer")).toBe(renderer);
}

/** A clip part's rect in page px, once the clip is laid out. */
export async function clipPart(page: Page, clipId: string, part: ClipPart = "box"): Promise<Rect> {
  let rect: Rect | null = null;
  await expect
    .poll(async () => {
      rect = await arrangerApi(page, "handleRect", clipId, part);
      return rect !== null;
    })
    .toBe(true);
  return rect!;
}

/** The center of a clip part (title band by default), in page px. */
export async function clipPoint(page: Page, clipId: string, part: ClipPart = "header", dx = 0.5): Promise<{ x: number; y: number }> {
  const r = await clipPart(page, clipId, part);
  return { x: r.x + r.width * dx, y: r.y + r.height / 2 };
}

/**
 * Opens a clip in its editor (piano roll for MIDI, the Warp tab for audio), the way a user
 * does: a double-click on the clip's title bar. Only the title bar (and the edge handles)
 * take the pointer; a clip's body lets presses through to the lane (locate, marquee), so
 * a double-click aimed at the body lands on the lane instead.
 */
export async function openClip(page: Page, clipId: string): Promise<void> {
  const p = await clipPoint(page, clipId);
  // Like a person's double-click: frames pass between its two clicks (the first selects the
  // track, and its row lays out again before the second; an instant double-click lands while
  // the browser still hit-tests the row without its content).
  await page.mouse.click(p.x, p.y);
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
  await page.mouse.click(p.x, p.y, { clickCount: 2 });
}

/** Selects a clip the way a user does: a click on its title bar (the body lets clicks through). */
export async function selectClip(page: Page, clipId: string, modifiers: Array<"Shift" | "Meta" | "Control"> = []): Promise<void> {
  const p = await clipPoint(page, clipId);
  for (const m of modifiers) await page.keyboard.down(m);
  await page.mouse.click(p.x, p.y);
  for (const m of modifiers.reverse()) await page.keyboard.up(m);
}

/** Opens a clip's context menu (a right-click on its title bar). */
export async function clipMenu(page: Page, clipId: string): Promise<void> {
  // A clip that just appeared (flattened, bounced) can still move while the rows settle: aim
  // once its rect holds still, and press again if no menu opened.
  await expect(async () => {
    const at = await clipPoint(page, clipId);
    await page.waitForTimeout(100);
    const p = await clipPoint(page, clipId);
    expect(p).toEqual(at);
    await page.mouse.click(p.x, p.y, { button: "right" });
    await expect(page.getByRole("menu")).toBeVisible({ timeout: 2_000 });
  }).toPass({ timeout: 20_000 });
}

/**
 * Drags a clip part (`header`: move, edges: resize) by `dx`/`dy` page px, in steps like a
 * user. `modifiers` are held for the whole drag (e.g. Meta/Control: copy, Alt: no snapping).
 */
export async function dragClip(
  page: Page,
  clipId: string,
  dx: number,
  dy = 0,
  opts: { part?: ClipPart; modifiers?: Array<"Shift" | "Meta" | "Control" | "Alt">; steps?: number; at?: number } = {},
): Promise<void> {
  const part = opts.part ?? "header";
  const p = await clipPoint(page, clipId, part, opts.at ?? (part === "header" ? 0.5 : 0.5));
  const mods = opts.modifiers ?? [];
  await page.mouse.move(p.x, p.y);
  for (const m of mods) await page.keyboard.down(m);
  await page.mouse.down();
  await page.mouse.move(p.x + dx, p.y + dy, { steps: opts.steps ?? 8 });
  await page.mouse.up();
  for (const m of [...mods].reverse()) await page.keyboard.up(m);
}

/** Whether the clip is selected (the UI selection). */
export function isClipSelected(page: Page, clipId: string): Promise<boolean> {
  return page.evaluate((id) => (window as unknown as { __ether: { selectedClips(): string[] } }).__ether.selectedClips().includes(id), clipId);
}

/** The page's pixel color at a page point (`[r, g, b]`, from a screenshot). */
export async function pixelAt(page: Page, x: number, y: number): Promise<[number, number, number]> {
  const png = await page.screenshot({ clip: { x: Math.floor(x), y: Math.floor(y), width: 1, height: 1 }, scale: "css" });
  const px = decodePng(png);
  return [px[0]!, px[1]!, px[2]!];
}

/**
 * The page's pixels in a rect (page px, rounded): RGBA rows. `scale: "device"`: at device
 * pixels (`width` and `height` then count device pixels); `png` is the screenshot.
 */
export async function pixelsIn(
  page: Page,
  r: Rect,
  scale: "css" | "device" = "css",
): Promise<{ width: number; height: number; data: Uint8Array; png: Buffer }> {
  const clip = { x: Math.floor(r.x), y: Math.floor(r.y), width: Math.max(1, Math.round(r.width)), height: Math.max(1, Math.round(r.height)) };
  const png = await page.screenshot({ clip, scale });
  const k = scale === "device" ? await page.evaluate(() => window.devicePixelRatio) : 1;
  return { width: Math.round(clip.width * k), height: Math.round(clip.height * k), data: decodePng(png), png };
}

/** Minimal PNG decoder (8-bit RGB/RGBA, as Chromium writes screenshots): the first rows' pixels. */
function decodePng(buf: Buffer): Uint8Array {
  let at = 8;
  let width = 0;
  let height = 0;
  let channels = 4;
  const idat: Buffer[] = [];
  while (at < buf.length) {
    const len = buf.readUInt32BE(at);
    const type = buf.toString("ascii", at + 4, at + 8);
    const data = buf.subarray(at + 8, at + 8 + len);
    if (type === "IHDR") {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      channels = data[9] === 2 ? 3 : 4;
    } else if (type === "IDAT") idat.push(data);
    at += 12 + len;
  }
  const raw = inflateSync(Buffer.concat(idat));
  const stride = width * channels;
  const out = new Uint8Array(width * height * 4);
  let prev = new Uint8Array(stride);
  for (let y = 0; y < height; y++) {
    const f = raw[y * (stride + 1)]!;
    const line = raw.subarray(y * (stride + 1) + 1, (y + 1) * (stride + 1));
    const cur = new Uint8Array(stride);
    for (let i = 0; i < stride; i++) {
      const a = i >= channels ? cur[i - channels]! : 0;
      const b = prev[i]!;
      const c = i >= channels ? prev[i - channels]! : 0;
      const p = a + b - c;
      const pa = Math.abs(p - a);
      const pb = Math.abs(p - b);
      const pc = Math.abs(p - c);
      const pred = f === 1 ? a : f === 2 ? b : f === 3 ? (a + b) >> 1 : f === 4 ? (pa <= pb && pa <= pc ? a : pb <= pc ? b : c) : 0;
      cur[i] = (line[i]! + pred) & 0xff;
    }
    for (let x = 0; x < width; x++) {
      for (let k = 0; k < 3; k++) out[(y * width + x) * 4 + k] = cur[x * channels + k]!;
      out[(y * width + x) * 4 + 3] = channels === 4 ? cur[x * channels + 3]! : 255;
    }
    prev = cur;
  }
  return out;
}

/**
 * Pixels of a clip's body drawing (waveform or notes), 0 until drawn: the pixels of its body
 * that differ from the body's background (its most common color).
 */
export function clipBodyInk(page: Page, clipId: string): Promise<number> {
  return arrangerApi(page, "bodyInk", clipId);
}

/**
 * Collaborators on a clip as drawn: the ring color at the middle of the clip's left edge and
 * the names in its editors chip.
 */
export function clipPeerMarks(page: Page, clipId: string): Promise<{ ring: [number, number, number] | null; editors: string[] } | null> {
  return arrangerApi(page, "peerMarks", clipId);
}

/** Whether `rgb` is `hex` (`#rrggbb`) within `tolerance` per channel. */
export function isColor(rgb: readonly number[] | null | undefined, hex: string, tolerance = 6): boolean {
  if (!rgb) return false;
  const want = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  return want.every((v, i) => Math.abs(v - rgb[i]!) <= tolerance);
}

/** Clips drawn in the new-track ghost lanes while dragging below the last track. */
export function newTrackGhostClips(page: Page): Promise<string[]> {
  return arrangerApi(page, "newTrackGhostClipIds");
}
