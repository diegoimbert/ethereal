// multisampler: the Multisampler against the real wasm engine. Insert it on a MIDI track,
// drop two Browser samples onto the zone map (the first covers the keyboard, the second
// becomes a one-key zone at its drop key), select a zone and edit it in the inspector
// (one undo step), then play a clip and hear the zones on the track meter.
// `MULTISAMPLER_SHOTS=<dir>` also captures the zone map + panel in the dark and light themes.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Command, Project, SampleZone } from "@/generated";
import { addDevice, launch, openDeviceTab, openLibrary, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
}

interface Probe {
  replies: Record<number, { status: string; error?: unknown }>;
  post(json: string): void;
}

const shots = process.env.MULTISAMPLER_SHOTS;

const project = (page: Page): Promise<Project> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project!);

const zonesOf = async (page: Page, device: string): Promise<SampleZone[]> => {
  const k = (await project(page)).devices[device]?.kind;
  return k?.type === "Builtin" && k.device.type === "MultiSampler" ? k.device.zones : [];
};

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
    (window as unknown as { __msProbe: Probe }).__msProbe = probe;
  });
}

let nextId = 2_000_000_000;

/** Send a command straight to the controller and wait for its successful reply. */
async function send(page: Page, command: Command) {
  const id = nextId++;
  await page.evaluate(
    ([id, command]) => {
      (window as unknown as { __msProbe: Probe }).__msProbe.post(JSON.stringify({ id, gesture: null, command }));
    },
    [id, command] as const,
  );
  await expect
    .poll(() => page.evaluate((id) => (window as unknown as { __msProbe: Probe }).__msProbe.replies[id], id), {
      timeout: 15_000,
    })
    .toMatchObject({ status: "Ok" });
}

async function open(page: Page, theme: string, height = 1100) {
  await page.setViewportSize({ width: 1600, height });
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
}

async function insertMultisampler(page: Page): Promise<{ track: string; device: string; card: Locator }> {
  await installProbe(page);
  const id = ulid(1, nextId++);
  await send(page, { domain: "Track", command: { type: "Create", id, kind: "Midi", name: "Multi", color: null, parent: null, before: null } });
  await openDeviceTab(page, "Multi");
  const before = new Set(Object.keys((await project(page)).devices));
  await addDevice(page, "MultiSampler");
  let device = "";
  await expect
    .poll(async () => {
      const p = await project(page);
      device = Object.keys(p.devices).find((d) => !before.has(d) && p.devices[d]!.track === id) ?? "";
      return device !== "";
    })
    .toBe(true);
  const card = page.locator(`section[data-device="${device}"]`);
  await expect(card).toBeVisible();
  await expect(card.getByText("Loading…")).toHaveCount(0);
  return { track: id, device, card };
}

/** Drop Browser file `name` onto the zone map at `fraction` of its width. */
async function dropSample(page: Page, map: Locator, name: string, fraction: number) {
  const box = (await map.boundingBox())!;
  const files = page.getByRole("list", { name: "Files" });
  await files.getByRole("button", { name, exact: true }).dragTo(map, {
    targetPosition: { x: box.width * fraction, y: box.height * 0.3 },
  });
}

async function withSamples(page: Page, device: string, card: Locator) {
  await openLibrary(page);
  await page.getByRole("list", { name: "Files" }).getByRole("button", { name: "Demo Samples" }).click();
  const map = card.getByTestId("zone-map");
  await map.scrollIntoViewIfNeeded();
  await dropSample(page, map, "Kick.wav", 0.4);
  await expect.poll(() => zonesOf(page, device).then((z) => z.length), { timeout: 20_000 }).toBe(1);
  await dropSample(page, map, "Snare.wav", 0.75);
  await expect.poll(() => zonesOf(page, device).then((z) => z.length), { timeout: 20_000 }).toBe(2);
  return map;
}

test("multisampler: drop samples on the zone map, edit a zone, hear it", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await open(page, "dark");
  const { track, device, card } = await insertMultisampler(page);
  await expect(card.locator('[data-widget="ZoneMap"]')).toHaveCount(1);
  await expect(card.locator('[data-widget="Envelope"]')).toHaveCount(2);
  await expect(card.getByText("Drop samples here from the Browser or your computer")).toBeVisible();

  const map = await withSamples(page, device, card);
  const [kick, snare] = await zonesOf(page, device);
  // First sample on an empty map: the whole keyboard; second: one key at its drop point.
  expect(kick!.keys).toEqual({ lo: 0, hi: 127 });
  expect(snare!.keys.lo).toBe(snare!.keys.hi);
  expect(snare!.keys.lo).toBeGreaterThan(80);

  // Select the kick zone (left part of the map) and set its root in the inspector.
  const box = (await map.getByTestId("widget-zone-map").boundingBox())!;
  await page.mouse.click(box.x + box.width * 0.1, box.y + box.height * 0.5);
  const inspector = card.getByTestId("zone-inspector");
  await expect(inspector).toContainText("Kick.wav");
  const root = inspector.getByRole("spinbutton", { name: "Root" });
  await root.click();
  await root.fill("48");
  await root.press("Enter");
  await expect.poll(() => zonesOf(page, device).then((z) => z[0]!.root_key)).toBe(48);
  await send(page, { domain: "Edit", command: { type: "Undo" } });
  await expect.poll(() => zonesOf(page, device).then((z) => z[0]!.root_key)).toBe(kick!.root_key);

  // Hear it: a looped clip of notes on the kick zone shows on the track meter.
  const clip = ulid(7, 1);
  await send(page, { domain: "Clip", command: { type: "CreateMidi", id: clip, track, start: 0, length: 4, name: null } });
  await send(page, {
    domain: "Note",
    command: {
      type: "Add",
      clip,
      notes: [0, 1, 2, 3].map((k) => ({ id: ulid(8, k), pitch: 48 + k, velocity: 1, start: k, duration: 0.5 })),
    },
  });
  await send(page, { domain: "Transport", command: { type: "SetLoopRegion", region: { start: 0, end: 4 } } });
  await send(page, { domain: "Transport", command: { type: "SetLoopEnabled", enabled: true } });
  await playButton(page).click();
  await expect.poll(() => peakOf(page, track), { timeout: 15_000 }).toBeGreaterThan(0.02);
  await page.getByRole("button", { name: "Stop" }).first().click();

  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});

for (const theme of ["dark", "light"]) {
  test(`multisampler zone map + panel screenshot (${theme})`, async ({ page }) => {
    test.skip(!shots, "set MULTISAMPLER_SHOTS=<dir> to capture the panel");
    test.setTimeout(90_000);
    await open(page, theme, 2400);
    const { device, card } = await insertMultisampler(page);
    const map = await withSamples(page, device, card);
    const box = (await map.getByTestId("widget-zone-map").boundingBox())!;
    await page.mouse.click(box.x + box.width * 0.1, box.y + box.height * 0.5);
    await expect(card.getByTestId("zone-inspector")).toBeVisible();
    await card.scrollIntoViewIfNeeded();
    await page.mouse.move(0, 0);
    await page.waitForTimeout(250);
    await card.screenshot({ path: `${shots}/multisampler-${theme}.png`, timeout: 15_000 });
  });
}
