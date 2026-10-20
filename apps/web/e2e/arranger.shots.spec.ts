// Arranger clip rendering screenshots: MIDI previews, looped,
// muted, overlapping and selected clips, audio waveforms with gain and fades, and a
// copy-drag in progress, and clips extended left (leading gap, wrapped loop), in the dark
// and light themes. Used to check that drawing clip bodies on one canvas per lane looks the
// same as one canvas per clip.
//
// `ARRANGER_SHOTS=<dir>` to capture (skipped otherwise).
import { expect, test, type Page } from "@playwright/test";
import { clipPart, selectClip } from "./clips";
import { expandFolder, launch, openLibrary } from "./ui";
import { doc, installProbe, send, ulid } from "./perfProject";

const shots = process.env.ARRANGER_SHOTS;

test.use({ viewport: { width: 1900, height: 1000 } });

async function buildScene(page: Page): Promise<void> {
  await installProbe(page);
  const keys = ulid(1, 0);
  await send(page, { domain: "Track", command: { type: "Create", id: keys, kind: "Midi", name: "Keys", color: null, parent: null, before: null } });
  const clip = async (n: number, start: number, length: number) => {
    const id = ulid(3, n);
    await send(page, { domain: "Clip", command: { type: "CreateMidi", id, track: keys, start, length, name: `clip ${n}` } });
    await send(page, {
      domain: "Note",
      command: {
        type: "Add",
        clip: id,
        notes: Array.from({ length: 12 }, (_, k) => ({
          id: ulid(4, n * 100 + k),
          pitch: 60 + ((k * 5) % 14),
          velocity: 60 + k * 5,
          start: k / 3,
          duration: 0.3,
        })),
      },
    });
    return id;
  };
  await clip(0, 0, 4);
  const looped = await clip(1, 4, 8);
  await send(page, { domain: "Clip", command: { type: "SetLoop", id: looped, looping: { enabled: true, start: 0, end: 2 } } });
  const muted = await clip(2, 12, 4);
  await send(page, { domain: "Clip", command: { type: "SetMuted", ids: [muted], muted: true } });
  // Overlapping pair.
  await clip(3, 16, 4);
  await clip(4, 18, 4);
  await clip(5, 23, 3);
  // A second MIDI track with plain clips only.
  const pads = ulid(1, 1);
  await send(page, { domain: "Track", command: { type: "Create", id: pads, kind: "Midi", name: "Pads", color: null, parent: null, before: null } });
  for (let c = 0; c < 4; c++) {
    const id = ulid(3, 10 + c);
    await send(page, { domain: "Clip", command: { type: "CreateMidi", id, track: pads, start: c * 6, length: 5, name: null } });
    await send(page, {
      domain: "Note",
      command: {
        type: "Add",
        clip: id,
        notes: [0, 1, 2, 3].map((k) => ({ id: ulid(4, 2000 + c * 10 + k), pitch: 48 + k * 4, velocity: 100, start: k, duration: 2 })),
      },
    });
  }
  // Clips extended left past their content start: an unlooped one with a
  // leading gap (negative offset), and a looped one whose offset wraps into the loop.
  for (const [c, start, offset, loop] of [
    [4, 24, -2, false],
    [5, 30, 2, true],
  ] as const) {
    const id = ulid(3, 10 + c);
    await send(page, { domain: "Clip", command: { type: "CreateMidi", id, track: pads, start, length: 5, name: null } });
    await send(page, {
      domain: "Note",
      command: {
        type: "Add",
        clip: id,
        notes: [0, 1, 2, 3].map((k) => ({ id: ulid(4, 2000 + c * 10 + k), pitch: 48 + k * 4, velocity: 100, start: k, duration: 1 })),
      },
    });
    if (loop) await send(page, { domain: "Clip", command: { type: "SetLoop", id, looping: { enabled: true, start: 0, end: 4 } } });
    await send(page, { domain: "Clip", command: { type: "SetBounds", id, start, length: 5, offset } });
  }
  // Audio: a library loop dropped twice, one with fades and one with +9 dB gain.
  await page.getByRole("button", { name: "add track", exact: true }).click();
  await page.getByRole("menuitem", { name: "create audio track" }).click();
  await expect.poll(async () => Object.values((await doc(page)).tracks).some((t) => t.kind === "Audio")).toBe(true);
  const audio = Object.values((await doc(page)).tracks).find((t) => t.kind === "Audio")!;
  await openLibrary(page);
  await expandFolder(page, "Demo Samples");
  const sample = page.getByRole("treeitem", { name: "Bass Loop 120.wav", exact: true });
  const lane = page.locator(`[data-lane="${audio.id}"]`);
  await sample.dragTo(lane, { targetPosition: { x: 5, y: 20 } });
  await expect.poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === audio.id).length, { timeout: 20_000 }).toBe(1);
  await sample.dragTo(lane, { targetPosition: { x: 400, y: 20 } });
  await expect.poll(async () => Object.values((await doc(page)).clips).filter((c) => c.track === audio.id).length, { timeout: 20_000 }).toBe(2);
  const [a, b] = Object.values((await doc(page)).clips)
    .filter((c) => c.track === audio.id)
    .sort((x, y) => x.start - y.start);
  await send(page, { domain: "Clip", command: { type: "SetFades", id: a!.id, fade_in: 1, fade_out: 2 } });
  await send(page, { domain: "Clip", command: { type: "SetGain", id: b!.id, gain: 9 } });
  // Close the library so the arrangement has the room.
  await page.keyboard.press("Escape");
}

for (const theme of ["dark", "light"] as const) {
  test(`arranger clips (${theme})`, async ({ page }) => {
    test.skip(!shots, "set ARRANGER_SHOTS=<dir> to capture the screenshots");
    test.setTimeout(120_000);
    await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
    await launch(page, `Arranger shots ${theme}`);
    await buildScene(page);
    const arr = page.locator(".eth-arr__scroll");
    // Peaks arrive asynchronously.
    await page.waitForTimeout(1500);
    await selectClip(page, ulid(3, 10));
    await page.mouse.move(0, 0);
    await page.waitForTimeout(300);
    await arr.screenshot({ path: `${shots}/arranger-${theme}.png` });
    // Copy-drag in progress (ghost) of the first clip, one bar right and one track down.
    const box = await clipPart(page, ulid(3, 0), "header");
    await page.keyboard.down("Control");
    await page.mouse.move(box.x + 10, box.y + 4);
    await page.mouse.down();
    await page.mouse.move(box.x + 60, box.y + 30, { steps: 5 });
    await page.mouse.move(box.x + 90, box.y + 70, { steps: 5 });
    await page.waitForTimeout(200);
    await arr.screenshot({ path: `${shots}/arranger-${theme}-drag.png` });
    await page.mouse.up();
    await page.keyboard.up("Control");
  });
}
