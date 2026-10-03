// synth-2: the Poly Synth against the real wasm engine. Insert it on a MIDI track, check its
// declared layout (two oscillators, filter curve, three envelopes, two LFOs, every param on
// the panel), turn the cutoff knob, then play a clip and hear it on the track meter; closing
// the filter down makes it quieter. `POLY_SYNTH_SHOTS=<dir>` also captures the panel in the
// dark and light themes for the PR.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Command, Project } from "@/generated";
import { addDevice, openDeviceTab, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
}

interface Probe {
  replies: Record<number, { status: string; error?: unknown }>;
  post(json: string): void;
}

const shots = process.env.POLY_SYNTH_SHOTS;

const project = (page: Page): Promise<Project> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project!);

const paramOf = async (page: Page, device: string, param: number): Promise<number | undefined> =>
  (await project(page)).devices[device]?.params[param];

const peakOf = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: Handle }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

/** Deterministic ULIDs: prefix + decimal digits (valid Crockford base32). */
const ulid = (kind: number, n: number) => `01J${kind}${String(n).padStart(22, "0")}`;

async function installProbe(page: Page) {
  await page.evaluate(() => {
    const ep = (
      window as unknown as {
        __etherEngine: { handles(): { controller: Worker } | null; post(json: string): void };
      }
    ).__etherEngine;
    const probe: Probe = { replies: {}, post: (json) => ep.post(json) };
    ep.handles()!.controller.addEventListener("message", (e: MessageEvent<{ type: string; json?: string }>) => {
      if (e.data.type !== "server" || !e.data.json) return;
      for (const m of JSON.parse(e.data.json) as { kind: string; body: Record<string, unknown> }[]) {
        if (m.kind === "Reply") {
          const { id, result } = m.body as { id: number; result: { status: string; error?: unknown } };
          probe.replies[id] = result;
        }
      }
    });
    (window as unknown as { __polyProbe: Probe }).__polyProbe = probe;
  });
}

let nextId = 2_000_000_000;

/** Send a command straight to the controller and wait for its successful reply. */
async function send(page: Page, command: Command) {
  const id = nextId++;
  await page.evaluate(
    ([id, command]) => {
      (window as unknown as { __polyProbe: Probe }).__polyProbe.post(JSON.stringify({ id, gesture: null, command }));
    },
    [id, command] as const,
  );
  await expect
    .poll(() => page.evaluate((id) => (window as unknown as { __polyProbe: Probe }).__polyProbe.replies[id], id), {
      timeout: 15_000,
    })
    .toMatchObject({ status: "Ok" });
}

async function drag(page: Page, target: Locator, dy: number) {
  const box = (await target.boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await page.mouse.move(x, y);
  await page.mouse.down();
  for (let i = 1; i <= 8; i++) await page.mouse.move(x, y + (dy * i) / 8);
  await page.mouse.up();
}

async function open(page: Page, theme: string, height = 900) {
  await page.setViewportSize({ width: 1440, height });
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
}

async function insertPolySynth(page: Page): Promise<{ track: string; device: string; card: Locator }> {
  // A MIDI track created by command (in the headless run, clips of a track created from the
  // UI's "add track" stay silent on the meter; see the synth-2 PR notes).
  await installProbe(page);
  const id = ulid(1, nextId++);
  await send(page, { domain: "Track", command: { type: "Create", id, kind: "Midi", name: "Poly", color: null, parent: null, before: null } });
  const track = { id, name: "Poly" };
  await openDeviceTab(page, track.name);
  const before = new Set(Object.keys((await project(page)).devices));
  await addDevice(page, "PolySynth");
  let device = "";
  await expect
    .poll(async () => {
      const p = await project(page);
      device = Object.keys(p.devices).find((d) => !before.has(d) && p.devices[d]!.track === track.id) ?? "";
      return device !== "";
    })
    .toBe(true);
  const card = page.locator(`section[data-device="${device}"]`);
  await expect(card).toBeVisible();
  await expect(card.getByText("Loading…")).toHaveCount(0);
  return { track: track.id, device, card };
}

test("poly synth: declared layout, knob, and sound on the meter", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await open(page, "dark");
  const { track, device, card } = await insertPolySynth(page);

  // The layout: typed widgets for oscillators, filter, envelopes and LFOs; no folded params.
  await expect(card.locator('[data-widget="Oscillator"]')).toHaveCount(2);
  await expect(card.locator('[data-widget="FilterCurve"]')).toHaveCount(1);
  await expect(card.locator('[data-widget="Envelope"]')).toHaveCount(3);
  await expect(card.locator('[data-widget="Lfo"]')).toHaveCount(2);
  await expect(card.getByRole("button", { name: /More .* controls/ })).toHaveCount(0);
  // Every param control is a MIDI-learn and modulation target.
  const params = card.locator(".eth-param");
  expect(await params.count()).toBeGreaterThanOrEqual(40);
  await expect(params.first()).toHaveAttribute("data-midi-target", /.+/);
  await expect(params.first()).toHaveAttribute("data-mod-target", new RegExp(`^${device}:\\d+$`));

  // Cutoff (param 26): dragging its knob up raises it.
  const cutoff = card.getByRole("slider", { name: "Cutoff" });
  await cutoff.scrollIntoViewIfNeeded();
  const c0 = (await paramOf(page, device, 26)) ?? 8000;
  await drag(page, cutoff, -40);
  await expect.poll(() => paramOf(page, device, 26)).toBeGreaterThan(c0);

  // Hear it: a looped clip of held notes shows on the track meter.
  const clip = ulid(7, 1);
  await send(page, { domain: "Clip", command: { type: "CreateMidi", id: clip, track, start: 0, length: 4, name: null } });
  await send(page, {
    domain: "Note",
    command: {
      type: "Add",
      clip,
      notes: [48, 55, 60, 64].map((pitch, k) => ({ id: ulid(8, k), pitch, velocity: 0.9, start: 0, duration: 4 })),
    },
  });
  await send(page, { domain: "Transport", command: { type: "SetLoopRegion", region: { start: 0, end: 4 } } });
  await send(page, { domain: "Transport", command: { type: "SetLoopEnabled", enabled: true } });
  await playButton(page).click();
  await expect.poll(() => peakOf(page, track), { timeout: 15_000 }).toBeGreaterThan(0.02);

  // Closing the filter (20 Hz) silences the (low-passed) synth.
  await send(page, { domain: "Device", command: { type: "SetParam", device, param: 26, value: 20 } });
  await expect.poll(() => peakOf(page, track), { timeout: 15_000 }).toBeLessThan(0.01);
  await page.getByRole("button", { name: "Stop" }).first().click();

  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});

for (const theme of ["dark", "light"]) {
  test(`poly synth panel screenshot (${theme})`, async ({ page }) => {
    test.skip(!shots, "set POLY_SYNTH_SHOTS=<dir> to capture the panel");
    test.setTimeout(60_000);
    // Tall enough for the whole panel in the inspector.
    await open(page, theme, 3200);
    const { card } = await insertPolySynth(page);
    await card.scrollIntoViewIfNeeded();
    await page.mouse.move(0, 0);
    await page.waitForTimeout(250);
    await card.screenshot({ path: `${shots}/poly-synth-${theme}.png`, timeout: 15_000 });
  });
}


