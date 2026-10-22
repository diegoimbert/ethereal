// The arranger's clip interactions, on the backend the project runs (`webgl`: WebGL2, the
// default; `canvas2d`: the fallback, forced by its debug override). Every gesture goes
// through the pointer at the clip's parts as the test API places them
// (`window.__ether.arrangement`), and is checked on the engine's document:
// - click / shift-click / cmd-click select, marquee, double-click opens the piano roll;
// - move, copy-drag, resize both edges, extend left, move to another track;
// - rename in place, the clip menu, hover cursors;
// - (WebGL) clips never disappear while panning and zooming, and the context restores;
// - the Canvas2D fallback draws what WebGL draws (clips, text, colors close).
import { expect, test, type Page } from "@playwright/test";
import type { Clip, Project } from "@/generated";
import {
  arrangement,
  arrangerApi,
  clipBodyInk,
  clipMenu,
  clipPart,
  clipPoint,
  dragClip,
  isClipSelected,
  openClip,
  pixelAt,
  pixelsIn,
  projectBackend,
  selectClip,
  setArrangerRenderer,
} from "./clips";
import { buildMixedProject, doc, importBassLoop, installProbe, send, setViewport, ulid } from "./perfProject";
import { launch } from "./ui";

test.use({ viewport: { width: 1440, height: 900 } });

const clipOf = async (page: Page, id: string): Promise<Clip> => (await doc(page)).clips[id]!;
const selected = (page: Page) => page.evaluate(() => (window as unknown as { __ether: { selectedClips(): string[] } }).__ether.selectedClips().sort());

/** Two MIDI tracks: clips A (0–4), B (6–10) and C (12–14) on the first, D (0–4) on the second. */
async function smallProject(page: Page): Promise<{ t1: string; t2: string; a: string; b: string; c: string; d: string }> {
  await installProbe(page);
  const t1 = ulid(8, 1);
  const t2 = ulid(8, 2);
  for (const [id, name] of [
    [t1, "Keys 1"],
    [t2, "Keys 2"],
  ] as const) {
    await send(page, { domain: "Track", command: { type: "Create", id, kind: "Midi", name, color: null, parent: null, before: null } });
  }
  const mk = async (n: number, track: string, start: number, length: number) => {
    const id = ulid(2, n);
    await send(page, { domain: "Clip", command: { type: "CreateMidi", id, track, start, length, name: `clip ${n}` } });
    await send(page, {
      domain: "Note",
      command: { type: "Add", clip: id, notes: [{ id: ulid(9, n), pitch: 60, velocity: 0.8, start: 0, duration: 1 }] },
    });
    return id;
  };
  const a = await mk(1, t1, 0, 4);
  const b = await mk(2, t1, 6, 4);
  const c = await mk(3, t1, 12, 2);
  const d = await mk(4, t2, 0, 4);
  await setViewport(page, 24, 0);
  return { t1, t2, a, b, c, d };
}

test("clip selection, marquee, editor, menu, rename and hover cursors", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await launch(page, `Arranger clips ${Date.now()}`);
  const { a, b, c, d } = await smallProject(page);
  // The project's backend from the first mount (`canvas2d`: the stored debug override).
  expect((await arrangement(page)).renderer).toBe(projectBackend(test.info().project.name));

  // Click, shift-click (add), cmd-click (toggle).
  await selectClip(page, a);
  await expect.poll(() => selected(page)).toEqual([a]);
  await selectClip(page, b, ["Shift"]);
  await expect.poll(() => selected(page)).toEqual([a, b].sort());
  await selectClip(page, a, ["Control"]);
  await expect.poll(() => selected(page)).toEqual([b]);
  // The hit test agrees: title band, edge handles, body.
  const h = await clipPoint(page, c);
  expect(await arrangerApi(page, "hitTest", h.x, h.y)).toEqual({ clip: c, part: "header" });
  const end = await clipPoint(page, c, "resize-end");
  expect(await arrangerApi(page, "hitTest", end.x, end.y)).toEqual({ clip: c, part: "resize-end" });

  // Marquee from empty space past C (first lane) back over B, down into the second lane
  // (empty there): B and C.
  const cb = await clipPart(page, c, "body");
  const bb = await clipPart(page, b, "body");
  const db = await clipPart(page, d, "body");
  await page.mouse.move(cb.x + cb.width + 20, cb.y + 4);
  await page.mouse.down();
  await page.mouse.move(bb.x + 10, db.y + db.height - 4, { steps: 8 });
  await page.mouse.up();
  await expect.poll(() => selected(page)).toEqual([b, c].sort());

  // Double-click on the title band: the piano roll opens on the clip.
  await openClip(page, a);
  await expect(page.getByRole("tab", { name: "piano roll" })).toHaveAttribute("aria-selected", "true");

  // Context menu → Rename: the name field over the title band; Enter renames.
  await clipMenu(page, b);
  await page.getByRole("menuitem", { name: "rename" }).click();
  const field = page.getByRole("textbox", { name: "rename clip 2" });
  await expect(field).toBeFocused();
  await field.fill("Chorus");
  await field.press("Enter");
  await expect.poll(async () => (await clipOf(page, b)).name).toBe("Chorus");
  await expect(field).toHaveCount(0);

  // With several selected, Rename names them all: one field over the clicked clip, one undo step.
  await selectClip(page, b);
  await selectClip(page, d, ["Shift"]);
  await expect.poll(() => selected(page)).toEqual([b, d].sort());
  await clipMenu(page, d);
  await page.getByRole("menuitem", { name: "rename 2 clips" }).click();
  const many = page.getByRole("textbox", { name: "rename 2 clips" });
  await expect(many).toHaveValue("clip 4");
  await many.fill("Verse");
  await many.press("Enter");
  await expect.poll(async () => [(await clipOf(page, b)).name, (await clipOf(page, d)).name]).toEqual(["Verse", "Verse"]);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => [(await clipOf(page, b)).name, (await clipOf(page, d)).name]).toEqual(["Chorus", "clip 4"]);

  // Hover cursors: grab on the title band, ew-resize on the edges.
  const cursorAt = async (p: { x: number; y: number }) => {
    await page.mouse.move(p.x, p.y);
    return page.evaluate(([x, y]) => getComputedStyle(document.elementFromPoint(x, y)!).cursor, [p.x, p.y] as const);
  };
  await expect.poll(async () => cursorAt(await clipPoint(page, c))).toBe("grab");
  await expect.poll(async () => cursorAt(await clipPoint(page, c, "resize-start"))).toBe("ew-resize");
  expect(errors).toEqual([]);
});

test("clip move, copy-drag, resize, extend left and move between tracks", async ({ page }) => {
  test.setTimeout(120_000);
  await launch(page, `Arranger drags ${Date.now()}`);
  const { t2, a, b, c } = await smallProject(page);
  const ppb = 24;
  const r3 = (x: number) => +x.toFixed(3);
  const bounds = async (id: string) => {
    const k = await clipOf(page, id);
    return [r3(k.start), r3(k.length), r3(k.offset), k.track];
  };
  const [, , , t1] = await bounds(a);

  // Move C right by 2 beats.
  await dragClip(page, c, 2 * ppb);
  await expect.poll(() => bounds(c)).toEqual([14, 2, 0, t1]);
  // An off-grid clip keeps its offset within the (1-beat) grid: 14.34 → 16.34, not 16.
  await send(page, { domain: "Clip", command: { type: "Move", moves: [{ id: c, track: (await clipOf(page, c)).track, start: 14.34 }] } });
  await expect.poll(() => bounds(c)).toEqual([14.34, 2, 0, t1]);
  await dragClip(page, c, 2 * ppb + 5);
  await expect.poll(() => bounds(c)).toEqual([16.34, 2, 0, t1]);
  await dragClip(page, c, -2 * ppb);
  await expect.poll(() => bounds(c)).toEqual([14.34, 2, 0, t1]);
  await send(page, { domain: "Clip", command: { type: "Move", moves: [{ id: c, track: (await clipOf(page, c)).track, start: 14 }] } });
  await expect.poll(() => bounds(c)).toEqual([14, 2, 0, t1]);
  // Copy-drag B right by 12 beats (Ctrl/Cmd held): a copy at 18, B stays.
  const before = Object.keys((await doc(page)).clips).length;
  await selectClip(page, b);
  await dragClip(page, b, 12 * ppb, 0, { modifiers: ["Control"] });
  await expect.poll(async () => Object.keys((await doc(page)).clips).length).toBe(before + 1);
  expect(await bounds(b)).toEqual([6, 4, 0, t1]);
  const copy = Object.values((await doc(page)).clips).find((k) => !Object.values([a, b, c]).includes(k.id) && Math.abs(k.start - 18) < 1e-6);
  expect(copy).toBeDefined();
  // Resize B's end 1 beat shorter, its start 1 beat later.
  await dragClip(page, b, -ppb, 0, { part: "resize-end" });
  await expect.poll(() => bounds(b)).toEqual([6, 3, 0, t1]);
  await dragClip(page, b, ppb, 0, { part: "resize-start" });
  await expect.poll(() => bounds(b)).toEqual([7, 2, 1, t1]);
  // Extend left past the content start: a leading gap (negative offset).
  await dragClip(page, b, -3 * ppb, 0, { part: "resize-start" });
  await expect.poll(() => bounds(b)).toEqual([4, 5, -2, t1]);
  // Move A down to the second track.
  const rowGap = (await clipPart(page, ulid(2, 4))).y - (await clipPart(page, a)).y;
  await dragClip(page, a, 0, rowGap);
  await expect.poll(async () => (await clipOf(page, a)).track).toBe(t2);
  expect(await isClipSelected(page, a)).toBe(true);
});

test("small clips (1 beat, 24 px) resize from both edges", async ({ page }) => {
  test.setTimeout(120_000);
  await launch(page, `Arranger small resize ${Date.now()}`);
  const { a } = await smallProject(page);
  const ppb = 24;
  const t1 = (await clipOf(page, a)).track;
  const e = ulid(2, 5);
  const f = ulid(2, 6);
  await send(page, { domain: "Clip", command: { type: "CreateMidi", id: e, track: t1, start: 20, length: 1, name: "e" } });
  await send(page, { domain: "Clip", command: { type: "CreateMidi", id: f, track: t1, start: 24, length: 1, name: "f" } });
  const bounds = async (id: string) => {
    const k = await clipOf(page, id);
    return [+k.start.toFixed(3), +k.length.toFixed(3), +k.offset.toFixed(3)];
  };
  await expect.poll(async () => (await clipPart(page, e, "resize-end")).width).toBe(6);
  // The hit test, the cursor and the drag agree on the edges; the middle moves.
  const end = await clipPoint(page, e, "resize-end");
  expect(await arrangerApi(page, "hitTest", end.x, end.y)).toEqual({ clip: e, part: "resize-end" });
  const mid = await clipPoint(page, e, "box");
  expect(await arrangerApi(page, "hitTest", mid.x, mid.y)).toEqual({ clip: e, part: "small" });
  await page.mouse.move(end.x, end.y);
  await expect.poll(() => page.evaluate(([x, y]) => getComputedStyle(document.elementFromPoint(x, y)!).cursor, [end.x, end.y] as const)).toBe("ew-resize");
  await dragClip(page, e, ppb, 0, { part: "resize-end" });
  await expect.poll(() => bounds(e)).toEqual([20, 2, 0]);
  await dragClip(page, f, -ppb, 0, { part: "resize-start" });
  await expect.poll(() => bounds(f)).toEqual([23, 2, -1]);
});

test("clips never disappear while panning and zooming (1k clips)", async ({ page }) => {
  test.skip(test.info().project.name === "canvas2d", "WebGL only");
  test.setTimeout(300_000);
  await launch(page, `Arranger visibility ${Date.now()}`);
  await buildMixedProject(page, { tracks: 40, clips: 1000, importSample: (t) => importBassLoop(page, t) });
  const frame = () => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
  expect((await arrangement(page)).renderer).toBe("webgl");
  /**
   * The clips drawn match the model's clips in view. Mid-animation (wheel motion), against
   * the view the frame was drawn for (WebGL); at rest, against the current view (the last
   * frame isn't stale).
   */
  const check = async (what: string, moving = false) => {
    await frame();
    // Both read in the same task (no frame drawn in between).
    const [drawn, expected] = await page.evaluate((atDrawn) => {
      const api = (window as unknown as { __ether: { arrangement: { visibleClipIds(): string[]; expectedVisibleClipIds(at?: string): string[] } } }).__ether
        .arrangement;
      return [api.visibleClipIds(), api.expectedVisibleClipIds(atDrawn ? "drawn" : undefined)];
    }, moving);
    if (!moving) expect(expected.length, `${what}: clips expected on screen`).toBeGreaterThan(0);
    expect(new Set(drawn), `${what}: drawn clips`).toEqual(new Set(expected));
  };
  // Zoom levels from far out to far in, at several scroll positions.
  for (const ppb of [0.5, 2, 8, 24, 96, 400]) {
    for (const scroll of [0, 7.3, 33, 120]) {
      await setViewport(page, ppb, scroll);
      await check(`ppb ${ppb} scroll ${scroll}`);
    }
  }
  // Wheel: vertical scrolling, ctrl-wheel zooming and horizontal panning, checked between
  // steps while the motion runs, and once it has settled.
  await setViewport(page, 12, 0);
  const box = (await page.locator(".eth-arr__scroll").boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  const settle = async (what: string) => {
    await page.waitForTimeout(800);
    await check(`${what} (settled)`);
  };
  for (let i = 0; i < 12; i++) {
    await page.mouse.wheel(0, i < 6 ? 240 : -200);
    await check(`scroll step ${i}`, true);
  }
  await settle("scroll");
  for (let i = 0; i < 12; i++) {
    await page.keyboard.down("Control");
    await page.mouse.wheel(0, i < 6 ? -120 : 120);
    await page.keyboard.up("Control");
    await check(`zoom step ${i}`, true);
  }
  await settle("zoom");
  for (let i = 0; i < 12; i++) {
    await page.mouse.wheel(i < 6 ? 240 : -200, 0);
    await check(`pan step ${i}`, true);
  }
  await settle("pan");
});

test("Canvas2D matches WebGL: resting, selected and dragged clips in both themes", async ({ page }) => {
  test.setTimeout(120_000);
  test.skip(test.info().project.name === "canvas2d", "compares both backends in one run");
  await launch(page, `Arranger pixels ${Date.now()}`);
  const { a, b } = await smallProject(page);
  // Away from the names and notes: three quarters along the clip.
  const samples = async (id: string) => {
    const title = await clipPoint(page, id, "header", 0.75);
    const body = await clipPoint(page, id, "body", 0.75);
    return { title: await pixelAt(page, title.x, title.y), body: await pixelAt(page, body.x, body.y) };
  };
  const run = async () => {
    await page.mouse.move(5, 5);
    const resting = await samples(a);
    await selectClip(page, b);
    await page.mouse.move(5, 5);
    const selectedClip = await samples(b);
    // Mid-drag (0.85 opacity), then back where it was.
    const p = await clipPoint(page, b);
    await page.mouse.move(p.x, p.y);
    await page.mouse.down();
    await page.mouse.move(p.x + 48, p.y, { steps: 4 });
    const dragged = await samples(b);
    await page.mouse.move(p.x, p.y, { steps: 4 });
    await page.mouse.up();
    return { resting, selectedClip, dragged };
  };
  for (const theme of ["dark", "light"] as const) {
    await page.evaluate((t) => {
      document.documentElement.dataset.theme = t;
    }, theme);
    const got: Record<string, Awaited<ReturnType<typeof run>>> = {};
    for (const renderer of ["webgl", "canvas2d"] as const) {
      await setArrangerRenderer(page, renderer);
      got[renderer] = await run();
    }
    await setArrangerRenderer(page, "webgl");
    for (const state of ["resting", "selectedClip", "dragged"] as const) {
      for (const part of ["title", "body"] as const) {
        const c = got.canvas2d![state][part];
        const w = got.webgl![state][part];
        const diff = Math.max(...c.map((v, i) => Math.abs(v - w[i]!)));
        expect(diff, `${theme} ${state} ${part}: canvas2d ${c} vs webgl ${w}`).toBeLessThanOrEqual(4);
      }
    }
  }
});

test("Canvas2D title text matches WebGL's (no stray strokes), dark on light and light on dark", async ({ page }) => {
  test.setTimeout(120_000);
  test.skip(test.info().project.name === "canvas2d", "compares both backends in one run");
  await launch(page, `Arranger glyphs ${Date.now()}`);
  const { a } = await smallProject(page);
  await send(page, { domain: "Clip", command: { type: "Rename", id: a, name: "Bob Lob 0" } });
  await expect.poll(async () => (await clipOf(page, a)).name).toBe("Bob Lob 0");
  await page.mouse.move(5, 5);
  for (const theme of ["dark", "light"] as const) {
    await page.evaluate((t) => {
      document.documentElement.dataset.theme = t;
    }, theme);
    const shots: Record<string, Awaited<ReturnType<typeof pixelsIn>>> = {};
    for (const renderer of ["webgl", "canvas2d"] as const) {
      await setArrangerRenderer(page, renderer);
      await page.waitForTimeout(300);
      const r = await clipPart(page, a, "header");
      shots[renderer] = await pixelsIn(page, { x: r.x + 2, y: r.y, width: 64, height: r.height });
    }
    await setArrangerRenderer(page, "webgl");
    // Pixels where one backend shows the plain band and the other a solid stroke: a stroke
    // where there is none (or a missing one). Anti-aliasing differences stay below it.
    const c = shots.canvas2d!;
    const w = shots.webgl!;
    const band = [...c.data.slice((c.width * (c.height - 2) + c.width - 2) * 4, (c.width * (c.height - 2) + c.width - 2) * 4 + 3)];
    const near = (d: Uint8Array, i: number) => Math.max(...band.map((v, k) => Math.abs(d[i + k]! - v))) <= 12;
    let strays = 0;
    for (let i = 0; i < c.data.length; i += 4) {
      const diff = Math.max(...[0, 1, 2].map((k) => Math.abs(c.data[i + k]! - w.data[i + k]!)));
      if (diff > 140 && (near(c.data, i) || near(w.data, i))) strays++;
    }
    console.log(`${theme}: ${strays} stray glyph pixels`);
    expect(strays, `${theme}: stray glyph pixels`).toBeLessThanOrEqual(STRAY_GLYPH_PIXELS);
  }
});

/**
 * Pixels where the glyphs disagree outright (an offset stem edge counts; a stray stroke is
 * many): none, both backends draw the same sprites of the browser's own text
 * (webgl-arranger-text.spec.ts compares them with the DOM's).
 */
const STRAY_GLYPH_PIXELS = 0;

test("title text: no stray ink past the name (dark text on a light band), also after an atlas reset", async ({ page }) => {
  test.setTimeout(120_000);
  await launch(page, `Arranger text ${Date.now()}`);
  const { a } = await smallProject(page);
  await send(page, { domain: "Clip", command: { type: "Rename", id: a, name: "Bob" } });
  await expect.poll(async () => (await clipOf(page, a)).name).toBe("Bob");
  // The 4-beat clip is 96 px: "Bob" ends before 30 px; past it the band is one color.
  const scan = async () => {
    const r = await clipPart(page, a, "header");
    const band = await pixelAt(page, r.x + r.width - 10, r.y + r.height / 2);
    let worst = 0;
    for (let x = r.x + 34; x < r.x + r.width - 10; x += 3) {
      for (const dy of [4, 7, 10]) {
        const p = await pixelAt(page, x, r.y + dy);
        worst = Math.max(worst, ...p.map((v, i) => Math.abs(v - band[i]!)));
      }
    }
    return worst;
  };
  await page.mouse.move(5, 5);
  expect(await scan()).toBeLessThanOrEqual(3);
  await page.evaluate(() => (window as unknown as { __ether: { arrangement: { resetText(): void } } }).__ether.arrangement.resetText());
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
  expect(await scan()).toBeLessThanOrEqual(3);
});

test("an audio clip's menu opens the Convert to MIDI dialog (its title status)", async ({ page }) => {
  test.setTimeout(120_000);
  await launch(page, `Arranger a2m ${Date.now()}`);
  await installProbe(page);
  const track = ulid(8, 7);
  await send(page, { domain: "Track", command: { type: "Create", id: track, kind: "Audio", name: "Audio", color: null, parent: null, before: null } });
  await importBassLoop(page, track);
  const clip = Object.values((await doc(page)).clips).find((c) => c.content.type === "Audio")!.id;
  await expect.poll(() => clipBodyInk(page, clip), { timeout: 20_000 }).toBeGreaterThan(50);
  await clipMenu(page, clip);
  await page.getByRole("menuitem", { name: "convert to MIDI…" }).click();
  const dialog = page.getByTestId("audio-to-midi-dialog");
  await expect(dialog).toBeVisible();
  await page.getByRole("button", { name: "cancel", exact: true }).click();
  await expect(dialog).toHaveCount(0);
});

test("WebGL: the context restores after a loss", async ({ page }) => {
  await launch(page, `Arranger context ${Date.now()}`);
  test.skip((await arrangement(page)).renderer !== "webgl", "WebGL only");
  const { a } = await smallProject(page);
  await expect.poll(() => arrangerApi(page, "bodyInk", a)).toBeGreaterThan(0);
  expect(await arrangerApi(page, "loseContext")).toBe(true);
  await expect.poll(() => arrangerApi(page, "bodyInk", a), { timeout: 10_000 }).toBeGreaterThan(0);
  const p = await clipPoint(page, a);
  expect(await arrangerApi(page, "hitTest", p.x, p.y)).toEqual({ clip: a, part: "header" });
});

export type { Project };
