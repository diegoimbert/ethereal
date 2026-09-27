// drum-rack on the web build, through the UI, against the real engine (WasmTransport →
// controller Worker → AudioWorklet).
//
// MIDI track → Drum Rack tab → "+ Drum Rack" → drop two demo samples from the browser on
// pads C1 and D1 (a Sampler on each pad) → select D1, choke group 1, mute/unmute →
// click C1 (select + audition) → undo the D1 drop in one step, redo.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Device, DrumPad, Project } from "@/generated";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const pads = async (page: Page): Promise<DrumPad[]> =>
  Object.values((await doc(page)).drum_pads).sort((a, b) => a.note - b.note);

const padChain = async (page: Page, pad: string): Promise<Device[]> =>
  Object.values((await doc(page)).devices).filter((d) => d.pad === pad);

const sampleOf = (d: Device): string | null =>
  d.kind.type === "Builtin" && d.kind.device.type === "Sampler" ? d.kind.device.sample : null;

test("drum rack: build a 2-pad kit from the browser", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play" })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  await page.getByRole("button", { name: "Projects" }).click();
  await page.getByLabel("New project name").fill(`Drum Rack ${Date.now()}`);
  await page.getByRole("dialog", { name: "Projects" }).getByRole("button", { name: "New" }).click();
  await expect.poll(async () => Object.keys((await doc(page)).drum_pads).length).toBe(0);

  // --- MIDI track, selected, with a drum rack --------------------------------------------
  await page.getByRole("button", { name: "+ MIDI track" }).click();
  await expect.poll(async () => Object.values((await doc(page)).tracks).some((t) => t.kind === "Midi")).toBe(true);
  const midi = Object.values((await doc(page)).tracks).find((t) => t.kind === "Midi")!;
  await page.getByRole("group", { name: `${midi.name} track` }).click();
  await page.getByRole("tab", { name: "Drum Rack" }).click();
  const view = page.getByTestId("drum-rack-view");
  await expect(view).toContainText(midi.name);
  await view.getByRole("button", { name: "+ Drum Rack" }).click();
  await expect
    .poll(async () =>
      Object.values((await doc(page)).devices).some(
        (d) => d.track === midi.id && d.kind.type === "Builtin" && d.kind.device.type === "DrumRack",
      ),
    )
    .toBe(true);
  const grid = view.getByRole("grid", { name: "Drum pads" });
  await expect(grid.getByRole("gridcell")).toHaveCount(16);

  // --- Two samples dropped on C1 and D1 --------------------------------------------------
  await page.getByRole("tablist", { name: "Locations" }).getByRole("tab", { name: "Browser library" }).click();
  await page.getByRole("list", { name: "Files" }).getByRole("button", { name: "Demo Samples" }).click();
  const files = page.getByRole("list", { name: "Files" });
  await files.getByRole("button", { name: "Kick.wav", exact: true }).dragTo(grid.getByRole("gridcell", { name: /^C1 / }));
  await expect.poll(async () => (await pads(page)).map((p) => p.note), { timeout: 20_000 }).toEqual([36]);
  await files.getByRole("button", { name: "Snare.wav", exact: true }).dragTo(grid.getByRole("gridcell", { name: /^D1 / }));
  await expect.poll(async () => (await pads(page)).map((p) => p.note), { timeout: 20_000 }).toEqual([36, 38]);
  const [kick, snare] = await pads(page);
  expect(kick!.name).toBe("Kick");
  expect(snare!.name).toBe("Snare");
  for (const pad of [kick!, snare!]) {
    const chain = await padChain(page, pad.id);
    expect(chain).toHaveLength(1);
    expect(sampleOf(chain[0]!)).not.toBeNull();
  }
  // Pad devices are not on the track chain (the default synth + the rack, first).
  const trackChain = Object.values((await doc(page)).devices)
    .filter((d) => d.track === midi.id && d.pad === null)
    .sort((a, b) => (a.order < b.order ? -1 : 1));
  expect(trackChain.map((d) => d.name)).toEqual(["Drum Rack", "Synth"]);

  // --- Pad settings: the dropped pad is selected ------------------------------------------
  const settings = page.getByTestId("pad-settings");
  await expect(settings).toHaveAttribute("aria-label", "Pad Snare");
  await expect(settings.getByRole("region", { name: "Sampler" })).toBeVisible();
  await settings.getByLabel("Choke group").selectOption("1");
  await expect.poll(async () => (await doc(page)).drum_pads[snare!.id]!.choke_group).toBe(1);
  await settings.getByRole("button", { name: "Mute pad" }).click();
  await expect.poll(async () => (await doc(page)).drum_pads[snare!.id]!.mute).toBe(true);
  await settings.getByRole("button", { name: "Mute pad" }).click();
  await expect.poll(async () => (await doc(page)).drum_pads[snare!.id]!.mute).toBe(false);

  // --- Click C1: selects it (and auditions its sample) -------------------------------------
  await grid.getByRole("gridcell", { name: /^C1 / }).click();
  await expect(settings).toHaveAttribute("aria-label", "Pad Kick");

  // --- Undo: unmute, mute and choke are single steps; the D1 drop is one step -------------
  const undo = page.getByRole("button", { name: "Undo" });
  const snarePad = async () => (await doc(page)).drum_pads[snare!.id];
  await undo.click();
  await expect.poll(async () => (await snarePad())?.mute).toBe(true);
  await undo.click();
  await expect.poll(async () => (await snarePad())?.mute).toBe(false);
  await undo.click();
  await expect.poll(async () => (await snarePad())?.choke_group).toBe(null);
  await undo.click();
  await expect.poll(async () => (await pads(page)).map((p) => p.note)).toEqual([36]);
  await page.getByRole("button", { name: "Redo" }).click();
  await expect.poll(async () => (await pads(page)).map((p) => p.note)).toEqual([36, 38]);

  expect(errors).toEqual([]);
});
