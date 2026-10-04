// base-106 on the web build (real wasm controller): dragging a MIDI clip and an audio clip
// below the last track (the empty canvas above the pinned master) shows a ghost "new track"
// lane, and the drop creates a track of the clip's kind with the clip on it. Each drop is
// ONE undo step. A sample dragged from the browser below the last track makes an audio track.
//
// `CLIP_DRAG_SHOTS=<dir>` also saves the PR screenshots (1440×900, dark).
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createTrack, openLibrary, launch } from "./ui";

const shots = process.env.CLIP_DRAG_SHOTS;
test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

interface Handle {
  state(): { project: Project | null };
}

async function doc(page: Page): Promise<Project> {
  const p = await page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);
  if (!p) throw new Error("no project open");
  return p;
}

const count = (r: Record<string, unknown>) => Object.keys(r).length;

/** Drags a clip by its title bar into the empty canvas below the last track; `shot` mid-drag. */
async function dragBelowLastTrack(page: Page, clipId: string, kind: "Midi" | "Audio", shot?: string): Promise<void> {
  const title = (await page.locator(`[data-clip-id="${clipId}"] .eth-clip__title`).boundingBox())!;
  const area = (await page.locator(".eth-arr__drop-area").boundingBox())!;
  // Grab the middle of the title bar: its corners belong to the fade handles (base-112).
  const x = title.x + title.width / 2;
  const y = title.y + title.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  await page.mouse.move(x + 40, y + 20, { steps: 4 });
  await page.mouse.move(x + 96, area.y + 24, { steps: 8 });
  const ghost = page.getByTestId("new-track-ghost");
  await expect(ghost).toHaveCount(1);
  await expect(ghost).toHaveAttribute("data-kind", kind);
  await expect(ghost.locator(".eth-clip")).toHaveCount(1);
  if (shots && shot) await page.screenshot({ path: `${shots}/${shot}.png` });
  await page.mouse.up();
  await expect(ghost).toHaveCount(0);
}

test("clips dragged below the last track land on new tracks, one undo step each", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page, `Clip drag ${Date.now()}`);

  // A MIDI clip (double-click on the MIDI lane) and an audio clip (sample from the library).
  const midiTrack = await createTrack(page, "Midi");
  await page.locator(`[data-lane="${midiTrack.id}"]`).dblclick({ position: { x: 10, y: 20 } });
  await expect.poll(async () => count((await doc(page)).clips)).toBe(1);
  const midiClip = Object.values((await doc(page)).clips)[0]!;

  const audioTrack = await createTrack(page, "Audio");
  await openLibrary(page);
  await page.getByRole("list", { name: "Files" }).getByRole("button", { name: "Demo Samples" }).click();
  const sample = page.getByRole("button", { name: "Bass Loop 120.wav", exact: true });
  await sample.dragTo(page.locator(`[data-lane="${audioTrack.id}"]`), { targetPosition: { x: 200, y: 20 } });
  await expect.poll(async () => count((await doc(page)).clips), { timeout: 20_000 }).toBe(2);
  const audioClip = Object.values((await doc(page)).clips).find((c) => c.track === audioTrack.id)!;
  if (shots) await page.screenshot({ path: `${shots}/0-before.png` });

  const original = await doc(page);
  const tracks0 = count(original.tracks);

  // --- MIDI clip → new MIDI track (with the built-in synth, like "New track") -------------
  await dragBelowLastTrack(page, midiClip.id, "Midi", "1-midi-mid-drag");
  await expect.poll(async () => count((await doc(page)).tracks)).toBe(tracks0 + 1);
  let p = await doc(page);
  const newMidi = Object.values(p.tracks).find((t) => !original.tracks[t.id])!;
  expect(newMidi.kind).toBe("Midi");
  expect(p.clips[midiClip.id]!.track).toBe(newMidi.id);
  expect(p.clips[midiClip.id]!.start).toBeGreaterThan(midiClip.start);
  expect(Object.values(p.devices).some((d) => d.track === newMidi.id)).toBe(true);
  if (shots) await page.screenshot({ path: `${shots}/2-midi-after-drop.png` });

  // --- Audio clip → new audio track -------------------------------------------------------
  const tracks1 = count(p.tracks);
  await dragBelowLastTrack(page, audioClip.id, "Audio", "3-audio-mid-drag");
  await expect.poll(async () => count((await doc(page)).tracks)).toBe(tracks1 + 1);
  p = await doc(page);
  const newAudio = Object.values(p.tracks).find((t) => !original.tracks[t.id] && t.id !== newMidi.id)!;
  expect(newAudio.kind).toBe("Audio");
  expect(p.clips[audioClip.id]!.track).toBe(newAudio.id);
  if (shots) await page.screenshot({ path: `${shots}/4-audio-after-drop.png` });

  // --- One undo step per drop: track and clip move go back together ----------------------
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => count((await doc(page)).tracks)).toBe(tracks1);
  p = await doc(page);
  expect(p.tracks[newAudio.id]).toBeUndefined();
  expect(p.clips[audioClip.id]!.track).toBe(audioTrack.id);
  expect(p.clips[audioClip.id]!.start).toBeCloseTo(audioClip.start, 6);
  expect(p.clips[midiClip.id]!.track).toBe(newMidi.id);

  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => count((await doc(page)).tracks)).toBe(tracks0);
  p = await doc(page);
  expect(p.tracks[newMidi.id]).toBeUndefined();
  expect(p.clips[midiClip.id]!.track).toBe(midiTrack.id);
  expect(p.clips[midiClip.id]!.start).toBeCloseTo(midiClip.start, 6);

  // --- A sample from the browser dropped below the last track: a new audio track ----------
  await sample.dragTo(page.locator(".eth-arr__drop-area"), { targetPosition: { x: 400, y: 30 } });
  await expect.poll(async () => count((await doc(page)).tracks), { timeout: 20_000 }).toBe(tracks0 + 1);
  p = await doc(page);
  const dropped = Object.values(p.tracks).find((t) => !original.tracks[t.id])!;
  expect(dropped.kind).toBe("Audio");
  expect(Object.values(p.clips).some((c) => c.track === dropped.id)).toBe(true);

  expect(errors).toEqual([]);
});
