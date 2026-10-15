// fm-synth: the FM Synth against the real wasm engine. Insert it on a MIDI track, check its
// declared layout (algorithm picker, four operators with their envelopes, filter, LFO),
// pick an algorithm, then play a clip and hear it on the track meter; switching the only
// carrier off silences it. `FM_SYNTH_SHOTS=<dir>` also captures the window in the dark and
// light themes for the PR.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Command, Project } from "@/generated";
import { addDevice, launch, openDeviceTab, openDeviceWindow, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
}

interface Probe {
  replies: Record<number, { status: string; error?: unknown }>;
  post(json: string): void;
}

const shots = process.env.FM_SYNTH_SHOTS;

/** Param ids (`ether_devices::fm_synth::fm_synth`). */
const ALGORITHM = 52;
const OP_A_ON = 0;
const OP_B_LEVEL = 13 + 6;

const project = (page: Page): Promise<Project> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project!);

const paramOf = async (page: Page, device: string, param: number): Promise<number | undefined> => (await project(page)).devices[device]?.params[param];

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
    (window as unknown as { __fmProbe: Probe }).__fmProbe = probe;
  });
}

let nextId = 2_100_000_000;

/** Send a command straight to the controller and wait for its successful reply. */
async function send(page: Page, command: Command) {
  const id = nextId++;
  await page.evaluate(
    ([id, command]) => {
      (window as unknown as { __fmProbe: Probe }).__fmProbe.post(JSON.stringify({ id, gesture: null, command }));
    },
    [id, command] as const,
  );
  await expect
    .poll(() => page.evaluate((id) => (window as unknown as { __fmProbe: Probe }).__fmProbe.replies[id], id), {
      timeout: 15_000,
    })
    .toMatchObject({ status: "Ok" });
}

async function open(page: Page, theme: string, height = 900) {
  await page.setViewportSize({ width: 1600, height });
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
}

async function insertFmSynth(page: Page): Promise<{ track: string; device: string; card: Locator }> {
  // A MIDI track created by command (as in poly-synth.spec.ts).
  await installProbe(page);
  const id = ulid(1, nextId++);
  await send(page, { domain: "Track", command: { type: "Create", id, kind: "Midi", name: "FM", color: null, parent: null, before: null } });
  await openDeviceTab(page, "FM");
  const before = new Set(Object.keys((await project(page)).devices));
  await addDevice(page, "FmSynth");
  let device = "";
  await expect
    .poll(async () => {
      const p = await project(page);
      device = Object.keys(p.devices).find((d) => !before.has(d) && p.devices[d]!.track === id) ?? "";
      return device !== "";
    })
    .toBe(true);
  const card = await openDeviceWindow(page, device);
  return { track: id, device, card };
}

test("fm synth: declared layout, algorithm picker, and sound on the meter", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await open(page, "dark");
  const { track, device, card } = await insertFmSynth(page);

  // The layout: the algorithm picker, an envelope per operator plus the filter's, the
  // filter curve and the LFO; no folded params.
  await expect(card.locator('[data-widget="FmAlgorithm"]')).toHaveCount(1);
  await expect(card.locator('[data-widget="Envelope"]')).toHaveCount(5);
  await expect(card.locator('[data-widget="FilterCurve"]')).toHaveCount(1);
  await expect(card.locator('[data-widget="Lfo"]')).toHaveCount(1);
  await expect(card.getByRole("button", { name: /more .* controls/ })).toHaveCount(0);
  const params = card.locator(".eth-param");
  expect(await params.count()).toBeGreaterThanOrEqual(50);
  await expect(params.first()).toHaveAttribute("data-mod-target", new RegExp(`^${device}:\\d+$`));

  // Pick algorithm 7 (two pairs).
  const cells = card.getByTestId("widget-fm-algorithm").getByRole("radio");
  await expect(cells).toHaveCount(11);
  await cells.nth(6).click();
  await expect.poll(() => paramOf(page, device, ALGORITHM)).toBe(6);
  await expect(cells.nth(6)).toHaveAttribute("aria-checked", "true");

  // Some FM: operator B into A.
  await send(page, { domain: "Device", command: { type: "SetParam", device, param: OP_B_LEVEL, value: -12 } });

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

  // Operator A is the only carrier: switching it off silences the synth.
  await send(page, { domain: "Device", command: { type: "SetParam", device, param: OP_A_ON, value: 0 } });
  await expect.poll(() => peakOf(page, track), { timeout: 15_000 }).toBeLessThan(0.005);
  await page.getByRole("button", { name: "stop" }).first().click();

  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});

for (const theme of ["dark", "light"]) {
  test(`fm synth window screenshot (${theme})`, async ({ page }) => {
    test.skip(!shots, "set FM_SYNTH_SHOTS=<dir> to capture the window");
    test.setTimeout(60_000);
    await open(page, theme, 1400);
    const { device, card } = await insertFmSynth(page);
    // A patch with some FM, so the panel shows a real setting.
    await send(page, { domain: "Device", command: { type: "SetParam", device, param: ALGORITHM, value: 6 } });
    await send(page, { domain: "Device", command: { type: "SetParam", device, param: OP_B_LEVEL, value: -12 } });
    await card.scrollIntoViewIfNeeded();
    await page.mouse.move(0, 0);
    await page.waitForTimeout(250);
    await card.screenshot({ path: `${shots}/fm-synth-${theme}.png`, timeout: 15_000 });
  });
}
