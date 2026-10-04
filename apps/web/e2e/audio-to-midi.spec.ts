// Audio to MIDI on the web build (audio-to-midi, CONTRACTS.md §13.5; real wasm controller in
// the Worker): import a sung-like melody (sines), right-click its clip → "Convert to MIDI…",
// pick the material, Convert: a MIDI track appears right below with the detected notes
// (pitches exact, onsets within 20 ms) and a Poly Synth; one undo removes it all. Drums
// convert to the drum keys with a Drum Rack.
//
// `A2M_SHOTS=<dir>` also saves PR screenshots (1440×900, dark; one light shot).
// No sleeps: every step waits on UI or engine state.
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { launch, newProject, playButton } from "./ui";

const shots = process.env.A2M_SHOTS;
test.use({ viewport: { width: 1440, height: 900 }, colorScheme: "dark" });

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project!);

const shot = async (page: Page, name: string) => {
  if (shots) await page.screenshot({ path: `${shots}/${name}.png` });
};

const RATE = 44_100;

/** A mono 16-bit WAV of `seconds` with `render(t)` in -1..1. */
function wav(seconds: number, render: (i: number) => number): Buffer {
  const frames = Math.round(RATE * seconds);
  const b = Buffer.alloc(44 + frames * 2);
  b.write("RIFF", 0);
  b.writeUInt32LE(36 + frames * 2, 4);
  b.write("WAVEfmt ", 8);
  b.writeUInt32LE(16, 16);
  b.writeUInt16LE(1, 20);
  b.writeUInt16LE(1, 22);
  b.writeUInt32LE(RATE, 24);
  b.writeUInt32LE(RATE * 2, 28);
  b.writeUInt16LE(2, 32);
  b.writeUInt16LE(16, 34);
  b.write("data", 36);
  b.writeUInt32LE(frames * 2, 40);
  for (let i = 0; i < frames; i++) b.writeInt16LE(Math.round(32767 * Math.max(-1, Math.min(1, render(i)))), 44 + i * 2);
  return b;
}

/** `[seconds, seconds, key]`: C D E G A C', detached. */
const MELODY: ReadonlyArray<readonly [number, number, number]> = [
  [0.25, 0.4, 60],
  [0.75, 0.4, 62],
  [1.25, 0.4, 64],
  [1.75, 0.4, 67],
  [2.25, 0.4, 69],
  [2.75, 0.6, 72],
];

const hz = (key: number) => 440 * 2 ** ((key - 69) / 12);

function melodyWav(): Buffer {
  return wav(3.6, (i) => {
    const t = i / RATE;
    let v = 0;
    for (const [s, d, k] of MELODY) {
      if (t < s || t >= s + d) continue;
      const ramp = Math.min(1, (t - s) / 0.003, (s + d - t) / 0.003);
      v += 0.3 * ramp * Math.sin(2 * Math.PI * hz(k) * (t - s));
    }
    return v;
  });
}

/** Kick (decaying 60 Hz) on the beats, a noise hat in between. */
const HITS: ReadonlyArray<readonly [number, number]> = [0, 1, 2, 3, 4, 5, 6, 7].map((i) => [0.1 + i * 0.25, i % 2 ? 42 : 36] as const);

function drumsWav(): Buffer {
  let seed = 1;
  const noise = () => {
    seed = (seed * 1_664_525 + 1_013_904_223) >>> 0;
    return seed / 2 ** 31 - 1;
  };
  let prev = 0;
  return wav(2.3, (i) => {
    const t = i / RATE;
    let v = 0;
    const n = noise();
    const hp = n - prev;
    prev = n;
    for (const [s, k] of HITS) {
      const dt = t - s;
      if (dt < 0 || dt > 0.25) continue;
      v += k === 36 ? 0.6 * Math.sin(2 * Math.PI * 60 * dt) * Math.exp(-dt / 0.08) : 0.3 * hp * Math.exp(-dt / 0.015);
    }
    return v;
  });
}

/** Import `files` with ⌘I; resolves with the new clips' ids in order. */
async function importFiles(page: Page, files: { name: string; buffer: Buffer }[]): Promise<string[]> {
  const before = new Set(Object.keys((await project(page)).clips));
  const chooser = page.waitForEvent("filechooser");
  await page.keyboard.press("ControlOrMeta+i");
  await (await chooser).setFiles(files.map((f) => ({ ...f, mimeType: "audio/wav" })));
  await expect
    .poll(async () => Object.keys((await project(page)).clips).filter((id) => !before.has(id)).length, { timeout: 20_000 })
    .toBe(files.length);
  const p = await project(page);
  return files.map((f) => {
    const media = Object.values(p.media).find((m) => m.name === f.name)!;
    return Object.values(p.clips).find((c) => c.content.type === "Audio" && c.content.media === media.id)!.id;
  });
}

async function convert(page: Page, clip: string, mode: "Melody" | "Harmony" | "Drums", shotName?: string) {
  await page.locator(`[data-clip-id="${clip}"] .eth-clip__title`).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Convert to MIDI…" }).click();
  const dialog = page.getByTestId("audio-to-midi-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByRole("radio", { name: mode }).click();
  await expect(dialog.getByRole("radio", { name: mode })).toHaveAttribute("aria-checked", "true");
  if (shotName) await shot(page, shotName);
  await page.getByRole("button", { name: "Convert", exact: true }).click();
  // The dialog closes once the new track is in.
  await expect(dialog).toHaveCount(0, { timeout: 30_000 });
}

test("convert a melody and a drum loop to MIDI tracks below their clips", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await newProject(page, `A2M ${Date.now()}`);
  const [vox, drums] = await importFiles(page, [
    { name: "Vox.wav", buffer: melodyWav() },
    { name: "Beat.wav", buffer: drumsWav() },
  ]);
  const before = await project(page);
  const src = before.clips[vox!]!;
  // The project tempo (a new project has one tempo point).
  const bpm = Object.values(before.tempo_points).sort((a, b) => a.time - b.time)[0]?.bpm ?? 120;

  await convert(page, vox!, "Melody", "01-dialog");
  let p = await project(page);
  const midiTrack = Object.values(p.tracks).find((t) => t.kind === "Midi" && t.name === `${src.name} MIDI`)!;
  expect(midiTrack).toBeDefined();
  // Right below the source track.
  const siblings = Object.values(p.tracks)
    .filter((t) => t.parent === midiTrack.parent)
    .sort((a, b) => (a.order < b.order ? -1 : 1));
  expect(siblings[siblings.findIndex((t) => t.id === src.track) + 1]!.id).toBe(midiTrack.id);
  const clip = Object.values(p.clips).find((c) => c.track === midiTrack.id)!;
  expect([clip.start, clip.length]).toEqual([src.start, src.length]);
  const notes = Object.values(p.notes)
    .filter((n) => n.clip === clip.id)
    .sort((a, b) => a.start - b.start);
  expect(notes.map((n) => n.pitch)).toEqual(MELODY.map(([, , k]) => k));
  // Onsets within 20 ms (beats → seconds at the project tempo).
  const secPerBeat = 60 / bpm;
  notes.forEach((n, i) => expect(Math.abs(n.start * secPerBeat - MELODY[i]![0])).toBeLessThan(0.02));
  expect(Object.values(p.devices).some((d) => d.track === midiTrack.id && d.kind.type === "Builtin" && d.kind.device.type === "PolySynth")).toBe(true);
  // The new clip is selected and draws its notes.
  await expect(page.locator(`[data-clip-id="${clip.id}"]`)).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator(`[data-clip-id="${clip.id}"] [data-testid="clip-notes"]`)).toHaveAttribute("data-notes", "6");
  await shot(page, "02-melody-converted");

  // Drums: kick/hat keys and a Drum Rack.
  await convert(page, drums!, "Drums");
  p = await project(page);
  const beatSrc = p.clips[drums!]!;
  const drumTrack = Object.values(p.tracks).find((t) => t.kind === "Midi" && t.name === `${beatSrc.name} MIDI`)!;
  const drumClip = Object.values(p.clips).find((c) => c.track === drumTrack.id)!;
  const keys = Object.values(p.notes)
    .filter((n) => n.clip === drumClip.id)
    .sort((a, b) => a.start - b.start)
    .map((n) => n.pitch);
  expect(keys).toEqual(HITS.map(([, k]) => k));
  expect(Object.values(p.devices).some((d) => d.track === drumTrack.id && d.kind.type === "Builtin" && d.kind.device.type === "DrumRack")).toBe(true);
  await shot(page, "03-drums-converted");
  if (shots) {
    await page.emulateMedia({ colorScheme: "light" });
    await page.evaluate(() => document.documentElement.setAttribute("data-theme", "light"));
    await page.locator(`[data-clip-id="${vox}"] .eth-clip__title`).click({ button: "right" });
    await page.getByRole("menuitem", { name: "Convert to MIDI…" }).click();
    await page.getByTestId("audio-to-midi-dialog").getByRole("radio", { name: "Harmony" }).click();
    await shot(page, "04-dialog-light");
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
    await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
    // A long file, to catch the progress in the dialog and on the clip (best effort: the
    // release wasm is fast).
    const melody = melodyWav().subarray(44);
    const long = Buffer.concat([melodyWav().subarray(0, 44), ...Array.from({ length: 60 }, () => melody)]);
    long.writeUInt32LE(long.length - 8, 4);
    long.writeUInt32LE(long.length - 44, 40);
    const [longClip] = await importFiles(page, [{ name: "Take.wav", buffer: long }]);
    await page.locator(`[data-clip-id="${longClip}"] .eth-clip__title`).click({ button: "right" });
    await page.getByRole("menuitem", { name: "Convert to MIDI…" }).click();
    await page.getByTestId("audio-to-midi-dialog").getByRole("radio", { name: "Melody" }).click();
    await page.getByRole("button", { name: "Convert", exact: true }).click();
    if (await page.getByRole("button", { name: "Hide", exact: true }).isVisible()) {
      await shot(page, "05-converting-dialog");
      await page.getByRole("button", { name: "Hide", exact: true }).click().catch(() => {});
      await page.getByTestId("audio-to-midi-dialog").waitFor({ state: "detached", timeout: 2_000 }).catch(() => {});
      await shot(page, "06-converting-clip");
    }
    await expect(page.getByRole("progressbar")).toHaveCount(0, { timeout: 60_000 });
    await page.getByTestId("arrangement-content").click({ position: { x: 5, y: 5 } });
    await page.keyboard.press("ControlOrMeta+z");
    await page.keyboard.press("ControlOrMeta+z");
    await expect.poll(async () => (await project(page)).clips[longClip!]).toBeUndefined();
  }

  // One undo step per conversion.
  await page.getByTestId("arrangement-content").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await project(page)).tracks[drumTrack.id]).toBeUndefined();
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => (await project(page)).tracks[midiTrack.id]).toBeUndefined();
  p = await project(page);
  expect(Object.keys(p.tracks).sort()).toEqual(Object.keys(before.tracks).sort());
  expect(Object.keys(p.notes).length).toBe(Object.keys(before.notes).length);
  expect(errors).toEqual([]);
});
