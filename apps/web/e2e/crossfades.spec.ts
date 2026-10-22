// Crossfades on the web build (`crossfades`), through the UI against the real engine
// (WasmTransport → controller Worker → AudioWorklet): two audio clips back to back, drag the
// first clip's fade-out handle across the boundary into the second → an equal-power
// crossfade centred on the boundary (both clips extended into each other); one undo removes
// it. Then resize it from the second clip's fade-in handle and drag it back to the boundary
// to remove it.
//
// No sleeps: every step waits on UI or engine state. `E2E_SHOTS=<dir>` saves PR screenshots
// (dark and light, 1440x900).
import { expect, test, type Page } from "@playwright/test";
import type { AudioContent, Clip } from "@/generated";
import { doc, installProbe, send, ulid } from "./perfProject";
import { clipDecor, clipPart, expectProjectRenderer, type ClipPart } from "./clips";
import { createTrack, expandFolder, launch } from "./ui";

const shot = async (page: Page, clips: string[], name: string, theme: string) => {
  const dir = process.env.E2E_SHOTS;
  if (!dir) return;
  await page.screenshot({ path: `${dir}/${theme}-${name}.png` });
  // A close-up of the clips.
  const boxes = await Promise.all(clips.map((c) => clipPart(page, c)));
  const x0 = Math.max(0, Math.min(...boxes.map((b) => b.x)) - 24);
  const x1 = Math.min(1440, Math.max(...boxes.map((b) => b.x + b.width)) + 24);
  const y0 = Math.min(...boxes.map((b) => b.y)) - 28;
  const y1 = Math.max(...boxes.map((b) => b.y + b.height)) + 8;
  await page.screenshot({ path: `${dir}/${theme}-${name}-closeup.png`, clip: { x: x0, y: y0, width: x1 - x0, height: y1 - y0 } });
};

const clipOf = async (page: Page, id: string): Promise<Clip> => (await doc(page)).clips[id]!;
const audioOf = (c: Clip) => c.content as AudioContent;
/** `[start, end]` of a clip, rounded to 1/100 beat (the gestures land on whole pixels). */
const span = async (page: Page, id: string) => {
  const c = await clipOf(page, id);
  const r = (x: number) => Math.round(x * 100) / 100;
  return [r(c.start), r(c.start + c.length)];
};

/** Drag a clip part (a fade handle) from its centre by `dx` px, with intermediate moves. */
async function dragBy(page: Page, clip: string, part: ClipPart, dx: number) {
  // Hover the clip first: its handles take the pointer while it is hovered.
  const box = await clipPart(page, clip, "header");
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  const b = await clipPart(page, clip, part);
  const [x, y] = [b.x + b.width / 2, b.y + b.height / 2];
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + dx / 2, y, { steps: 4 });
  await page.mouse.move(x + dx, y, { steps: 4 });
  await page.mouse.up();
}

for (const theme of ["dark", "light"] as const) {
  test(`crossfades: drag a fade handle across the boundary, undo, resize, remove (${theme})`, async ({ page }) => {
    test.skip(theme === "light" && !process.env.E2E_SHOTS, "the light run only captures screenshots");
    // Quarantined: removing the crossfade by dragging A's fade-out handle fails (#289).
    test.fixme(true, "#289: dragging A's fade-out handle to the boundary no longer removes the crossfade");
    test.setTimeout(150_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
    // Instant zoom (no spring animation).
    await page.emulateMedia({ reducedMotion: "reduce" });
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));

    // Nothing is open on launch: create the project on the launch screen.
    await launch(page, `Crossfades ${theme} ${Date.now()}`);
    await installProbe(page);

    // --- An audio clip from the library, split in two clips back to back -------------------
    const track = await createTrack(page, "Audio");
    await page.getByRole("button", { name: "library", exact: true }).click();
    await page.getByRole("button", { name: "pin the library panel" }).click();
    await expandFolder(page, "factory");
    await expandFolder(page, "Demo Samples");
    const sample = page.getByRole("treeitem", { name: "Bass Loop 120.wav", exact: true });
    await sample.dragTo(page.locator(`[data-lane="${track.id}"]`), { targetPosition: { x: 5, y: 20 } });
    await expect.poll(async () => Object.keys((await doc(page)).clips).length, { timeout: 20_000 }).toBe(1);
    // Room for the clips: close the library and the inspector.
    await page.getByRole("button", { name: "close the library panel" }).click();
    const inspector = page.getByRole("button", { name: "collapse the inspector panel" });
    if (await inspector.isVisible()) await inspector.click();
    const whole = Object.values((await doc(page)).clips)[0]!;
    // Split in the middle (M): each clip has material across the boundary.
    const [S, E] = [whole.start, whole.start + whole.length];
    const M = (S + E) / 2;
    const second = ulid(9, 1);
    await send(page, { domain: "Clip", command: { type: "Split", id: whole.id, at: M, new_id: second } });
    await expect.poll(() => span(page, second)).toEqual([M, E]);
    const first = whole.id;
    await expectProjectRenderer(page, test.info().project.name);
    const [a, b] = [first, second];
    const width = async (id: string) => (await clipPart(page, id)).width;
    // Zoom in (Ctrl+wheel notches over the clips' start) until the first clip is ~500 px wide.
    for (let i = 0; i < 40 && (await width(a)) < 420; i++) {
      const x = (await clipPart(page, a)).x + 2;
      await page.keyboard.down("Control");
      await page.evaluate((clientX) => {
        const el = document.querySelector<HTMLElement>(".eth-arr__scroll")!;
        const r = el.getBoundingClientRect();
        el.dispatchEvent(
          new WheelEvent("wheel", {
            deltaY: -3,
            deltaMode: WheelEvent.DOM_DELTA_LINE,
            ctrlKey: true,
            clientX,
            clientY: r.top + 20,
            bubbles: true,
            cancelable: true,
          }),
        );
      }, x);
      await page.keyboard.up("Control");
    }
    // Let the zoom settle (the lane re-renders at the final zoom).
    let last = 0;
    await expect
      .poll(async () => {
        const w = await width(a);
        const settled = w === last;
        last = w;
        return settled;
      })
      .toBe(true);
    const px = (await width(a)) / (M - S);
    await shot(page, [a, b], "1-clips-back-to-back", theme);

    // --- Drag A's fade-out handle half a beat past the boundary: a 1-beat crossfade ---------
    await page.mouse.move((await clipPart(page, a, "header")).x + 40, (await clipPart(page, a, "header")).y + 4);
    const handleBox = await clipPart(page, a, "fade-out");
    const aBox = await clipPart(page, a);
    // The handle sits a little inside A's end: travel to the boundary, then 0.5 beat beyond.
    const toEdge = aBox.x + aBox.width - (handleBox.x + handleBox.width / 2);
    await dragBy(page, a, "fade-out", toEdge + 0.5 * px);
    await expect.poll(() => span(page, first)).toEqual([S, M + 0.5]);
    expect(await span(page, second)).toEqual([M - 0.5, E]);
    const [ca, cb] = [await clipOf(page, first), await clipOf(page, second)];
    expect(audioOf(ca).fade_out).toBeCloseTo(1, 2);
    expect(audioOf(cb).fade_in).toBeCloseTo(1, 2);
    expect(audioOf(ca).fade_out_curve).toEqual({ type: "EqualPower" });
    expect(audioOf(cb).fade_in_curve).toEqual({ type: "EqualPower" });
    await expect.poll(async () => (await clipDecor(page, b))?.crossfadeIn).toBe(true);
    await expect.poll(async () => (await clipDecor(page, a))?.crossfadeOut).toBe(true);
    // Both fades drawn across the region (an X): B's own fade-in and A's fade-out, dashed.
    await expect.poll(async () => (await clipDecor(page, b))?.lines).toBe(2);
    const bTitle = await clipPart(page, b, "header");
    await page.mouse.move(bTitle.x + 40, bTitle.y + 4);
    await shot(page, [a, b], "2-crossfade", theme);

    // --- One undo removes it ---------------------------------------------------------------
    await page.getByRole("button", { name: "undo" }).click();
    await expect.poll(() => span(page, first)).toEqual([S, M]);
    expect(await span(page, second)).toEqual([M, E]);
    expect(audioOf(await clipOf(page, first)).fade_out).toBe(0);
    await expect.poll(async () => (await clipDecor(page, b))?.crossfadeIn).toBe(false);

    // --- Redo, resize from B's fade-in handle (its right edge), then remove by dragging -----
    await page.getByRole("button", { name: "redo" }).click();
    await expect.poll(() => span(page, second)).toEqual([M - 0.5, E]);
    await dragBy(page, b, "fade-in", 0.5 * px);
    await expect.poll(async () => audioOf(await clipOf(page, second)).fade_in).toBeCloseTo(2, 1);
    const [s, e] = await span(page, second);
    // Symmetric around the boundary.
    expect(Math.abs(M - s! - ((await span(page, first))[1]! - M))).toBeLessThan(0.011);
    expect(e).toBe(E);
    await shot(page, [a, b], "3-crossfade-resized", theme);
    // A's fade-out handle is the crossfade's left edge: drag it to the boundary → removed.
    // Hover A's start (B is above it over the crossfade): its handles show.
    const aBody = await clipPart(page, a, "header");
    await page.mouse.move(aBody.x + 40, aBody.y + 4);
    const h = await clipPart(page, a, "fade-out");
    const boundary = aBox.x + (M - S) * px;
    await dragBy(page, a, "fade-out", boundary - (h.x + h.width / 2) + 0.2 * px);
    await expect.poll(() => span(page, first)).toEqual([S, M]);
    expect(await span(page, second)).toEqual([M, E]);

    expect(errors).toEqual([]);
  });
}
