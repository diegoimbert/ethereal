// midi-fx: the six MIDI effects against the real wasm engine. MIDI effects added from the
// picker go before the instrument (a MIDI effect behind it is refused), their panels render,
// a knob turns, and the notes they let through are heard on the track meter: Velocity in
// Gate mode with its input range above the played velocity silences the synth, and opening
// the range plays it again. `MIDI_FX_SHOTS=<dir>` also captures every panel in the dark and
// light themes for the PR.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { BuiltinDevice, BuiltinDeviceType, Command, Project } from "@/generated";
import { addDevice, launch, openDeviceTab, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
}

interface Probe {
  replies: Record<number, { status: string; error?: { code?: string } }>;
  post(json: string): void;
}

const shots = process.env.MIDI_FX_SHOTS;

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
          const { id, result } = m.body as { id: number; result: Probe["replies"][number] };
          probe.replies[id] = result;
        }
      }
    });
    (window as unknown as { __midiFxProbe: Probe }).__midiFxProbe = probe;
  });
}

let nextId = 2_100_000_000;

/** Send a command straight to the controller; resolves with its reply. */
async function sendRaw(page: Page, command: Command): Promise<Probe["replies"][number]> {
  const id = nextId++;
  await page.evaluate(
    ([id, command]) => {
      (window as unknown as { __midiFxProbe: Probe }).__midiFxProbe.post(JSON.stringify({ id, gesture: null, command }));
    },
    [id, command] as const,
  );
  let reply: Probe["replies"][number] | undefined;
  await expect
    .poll(
      async () => {
        reply = await page.evaluate((id) => (window as unknown as { __midiFxProbe: Probe }).__midiFxProbe.replies[id], id);
        return reply !== undefined;
      },
      { timeout: 15_000 },
    )
    .toBe(true);
  return reply!;
}

async function send(page: Page, command: Command) {
  expect(await sendRaw(page, command)).toMatchObject({ status: "Ok" });
}

async function drag(page: Page, target: Locator, dy: number) {
  await target.scrollIntoViewIfNeeded();
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
  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await installProbe(page);
}

/** A MIDI track created by command (no default instrument; see the synth-2 PR notes). */
async function midiTrack(page: Page, name: string): Promise<string> {
  const id = ulid(1, nextId++);
  await send(page, { domain: "Track", command: { type: "Create", id, kind: "Midi", name, color: null, parent: null, before: null } });
  await openDeviceTab(page, name);
  return id;
}

/** Add a device from the chain's picker; resolves with its id and panel. */
async function pick(page: Page, track: string, type: BuiltinDeviceType): Promise<{ device: string; card: Locator }> {
  const before = new Set(Object.keys((await project(page)).devices));
  await addDevice(page, type);
  let device = "";
  await expect
    .poll(async () => {
      const p = await project(page);
      device = Object.keys(p.devices).find((d) => !before.has(d) && p.devices[d]!.track === track) ?? "";
      return device !== "";
    })
    .toBe(true);
  const card = page.locator(`section[data-device="${device}"]`);
  await expect(card).toBeVisible();
  await expect(card.getByText("Loading…")).toHaveCount(0);
  return { device, card };
}

const insertCmd = (track: string, id: string, type: BuiltinDeviceType): Command => ({
  domain: "Device",
  command: { type: "Insert", id, track, device: { type: "Builtin", device: { type } as BuiltinDevice }, before: null },
});

test("MIDI effects: before the instrument, panels, a knob, and heard on the meter", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await open(page, "dark");
  const track = await midiTrack(page, "Keys");

  // An instrument, then MIDI effects from the picker: they go before it.
  const synth = ulid(2, 1);
  await send(page, insertCmd(track, synth, "Synth"));
  const arp = await pick(page, track, "Arpeggiator");
  const vel = await pick(page, track, "Velocity");
  // A MIDI effect behind the instrument is refused by the controller.
  const refused = await sendRaw(page, insertCmd(track, ulid(2, 2), "Chord"));
  expect(refused.status).toBe("Err");
  const chain = Object.values((await project(page)).devices)
    .filter((d) => d.track === track)
    .sort((a, b) => (a.order < b.order ? -1 : 1))
    .map((d) => d.id);
  expect(chain).toEqual([arp.device, vel.device, synth]);

  // Declared panels: pickers and knobs, every control learnable.
  await expect(arp.card.getByRole("combobox", { name: "Style", exact: true })).toBeVisible();
  for (const name of ["Rate", "Gate", "Octaves", "Swing"]) {
    await expect(arp.card.getByRole("slider", { name, exact: true })).toBeVisible();
  }
  await expect(arp.card.locator(".eth-param").first()).toHaveAttribute("data-midi-target", /.+/);
  for (const name of ["In Low", "In High", "Out Low", "Out High", "Drive"]) {
    await expect(vel.card.getByRole("slider", { name, exact: true })).toBeVisible();
  }

  // Turn a knob: the arpeggiator's Gate (param 2) goes up.
  const g0 = (await paramOf(page, arp.device, 2)) ?? 75;
  await drag(page, arp.card.getByRole("slider", { name: "Gate", exact: true }), -40);
  await expect.poll(() => paramOf(page, arp.device, 2)).toBeGreaterThan(g0);

  // Hear it: a looped held chord, arpeggiated into the synth.
  const clip = ulid(7, 1);
  await send(page, { domain: "Clip", command: { type: "CreateMidi", id: clip, track, start: 0, length: 4, name: null } });
  await send(page, {
    domain: "Note",
    command: {
      type: "Add",
      clip,
      notes: [60, 64, 67].map((pitch, k) => ({ id: ulid(8, k), pitch, velocity: 0.6, start: 0, duration: 4 })),
    },
  });
  await send(page, { domain: "Transport", command: { type: "SetLoopRegion", region: { start: 0, end: 4 } } });
  await send(page, { domain: "Transport", command: { type: "SetLoopEnabled", enabled: true } });
  await playButton(page).click();
  await expect.poll(() => peakOf(page, track), { timeout: 15_000 }).toBeGreaterThan(0.02);

  // Velocity in Gate mode (param 0 = 1) with In Low (param 5) at 127: velocity 0.6 (76) is
  // dropped, so is everything the arpeggiator plays. Silence.
  await send(page, { domain: "Device", command: { type: "SetParam", device: vel.device, param: 0, value: 1 } });
  await send(page, { domain: "Device", command: { type: "SetParam", device: vel.device, param: 5, value: 127 } });
  await expect.poll(() => peakOf(page, track), { timeout: 15_000 }).toBeLessThan(0.001);
  // Open the range again: it plays.
  await send(page, { domain: "Device", command: { type: "SetParam", device: vel.device, param: 5, value: 1 } });
  await expect.poll(() => peakOf(page, track), { timeout: 15_000 }).toBeGreaterThan(0.02);
  await page.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Stop" }).first().click();

  expect(errors.filter((e) => /Ethereal engine|panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});

const TYPES: BuiltinDeviceType[] = ["Arpeggiator", "Chord", "ScaleQuantize", "NoteLength", "Velocity", "Randomizer"];

for (const theme of ["dark", "light"]) {
  for (const type of TYPES) {
    test(`${type} panel screenshot (${theme})`, async ({ page }) => {
      test.skip(!shots, "set MIDI_FX_SHOTS=<dir> to capture the panels");
      test.setTimeout(60_000);
      await open(page, theme, 1400);
      const track = await midiTrack(page, "Keys");
      const { card } = await pick(page, track, type);
      await card.scrollIntoViewIfNeeded();
      await page.mouse.move(0, 0);
      await page.waitForTimeout(250);
      await card.screenshot({ path: `${shots}/midi-fx-${type}-${theme}.png`, timeout: 15_000 });
    });
  }
}
