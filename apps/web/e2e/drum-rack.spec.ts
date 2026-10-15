// drum-rack on the web build, through the UI, against the real engine (WasmTransport →
// controller Worker → AudioWorklet).
//
// MIDI track → a Drum Rack from the instrument picker (its window is the pad editor) → drop two demo samples from the browser on
// pads C1 and D1 (a Sampler on each pad) → select D1, choke group 1, mute/unmute (on the pad) →
// click C1 (select + audition) → undo the D1 drop in one step, redo.
//
// No sleeps: every step waits on UI or engine state. The mirror is read through
// `window.__ether` (apps/web/src/main.tsx).
import { expect, test, type Page } from "@playwright/test";
import type { Device, DrumPad, Project } from "@/generated";
import { addDrumRack, clickHeaderControl, padButton, createTrack, launch, newProject, openLibrary, pickOption, playButton, expandFolder } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

async function doc(page: Page): Promise<Project> {
  const p = await project(page);
  if (!p) throw new Error("no project open");
  return p;
}

const pads = async (page: Page): Promise<DrumPad[]> => Object.values((await doc(page)).drum_pads).sort((a, b) => a.note - b.note);

const padChain = async (page: Page, pad: string): Promise<Device[]> => Object.values((await doc(page)).devices).filter((d) => d.pad === pad);

const sampleOf = (d: Device): string | null => (d.kind.type === "Builtin" && d.kind.device.type === "Sampler" ? d.kind.device.sample : null);

test("drum rack: build a 2-pad kit from the browser", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);

  await newProject(page, `Drum Rack ${Date.now()}`);
  await expect.poll(async () => Object.keys((await doc(page)).drum_pads).length).toBe(0);

  // --- MIDI track, selected, with a drum rack --------------------------------------------
  const midi = await createTrack(page, "Midi");
  // The library first: the rack's window (its pad editor) opens over the top of the page.
  await openLibrary(page);
  await expandFolder(page, "Demo Samples");
  const { win: view } = await addDrumRack(page, midi.name);
  await expect
    .poll(async () =>
      Object.values((await doc(page)).devices).some((d) => d.track === midi.id && d.kind.type === "Builtin" && d.kind.device.type === "DrumRack"),
    )
    .toBe(true);
  const grid = view.getByRole("grid", { name: "drum pads" });
  await expect(grid.getByRole("gridcell")).toHaveCount(16);

  // --- Two samples dropped on C1 and D1 --------------------------------------------------
  const files = page.getByRole("tree", { name: "folders" });
  await files.getByRole("treeitem", { name: "Kick.wav", exact: true }).dragTo(grid.getByRole("gridcell", { name: /^C1 / }));
  await expect.poll(async () => (await pads(page)).map((p) => p.note), { timeout: 20_000 }).toEqual([36]);
  await files.getByRole("treeitem", { name: "Snare.wav", exact: true }).dragTo(grid.getByRole("gridcell", { name: /^D1 / }));
  await expect.poll(async () => (await pads(page)).map((p) => p.note), { timeout: 20_000 }).toEqual([36, 38]);
  const [kick, snare] = await pads(page);
  expect(kick!.name).toBe("Kick");
  expect(snare!.name).toBe("Snare");
  for (const pad of [kick!, snare!]) {
    const chain = await padChain(page, pad.id);
    expect(chain).toHaveLength(1);
    expect(sampleOf(chain[0]!)).not.toBeNull();
  }
  // Pad devices are not on the track chain (the rack, which replaced the default synth).
  const trackChain = Object.values((await doc(page)).devices)
    .filter((d) => d.track === midi.id && d.pad === null)
    .sort((a, b) => (a.order < b.order ? -1 : 1));
  expect(trackChain.map((d) => d.name)).toEqual(["drum rack"]);

  // --- Pad settings: the dropped pad is selected ------------------------------------------
  const settings = page.getByTestId("pad-settings");
  await expect(settings).toHaveAttribute("aria-label", "pad Snare");
  await expect(settings.getByRole("region", { name: "Sampler" })).toBeVisible();
  await pickOption(settings, "choke group", { value: "1" });
  await expect.poll(async () => (await doc(page)).drum_pads[snare!.id]!.choke_group).toBe(1);
  await (await padButton(grid, "mute Snare")).click();
  await expect.poll(async () => (await doc(page)).drum_pads[snare!.id]!.mute).toBe(true);
  await (await padButton(grid, "mute Snare")).click();
  await expect.poll(async () => (await doc(page)).drum_pads[snare!.id]!.mute).toBe(false);

  // --- Click C1: selects it (and auditions it through its chain)----------------------------------
  await grid.getByRole("gridcell", { name: /^C1 / }).click();
  await expect(settings).toHaveAttribute("aria-label", "pad Kick");

  // --- Undo: unmute, mute and choke are single steps; the D1 drop is one step -------------
  const undo = page.getByRole("button", { name: "undo" });
  const snarePad = async () => (await doc(page)).drum_pads[snare!.id];
  await undo.click();
  await expect.poll(async () => (await snarePad())?.mute).toBe(true);
  await undo.click();
  await expect.poll(async () => (await snarePad())?.mute).toBe(false);
  await undo.click();
  await expect.poll(async () => (await snarePad())?.choke_group).toBe(null);
  await undo.click();
  await expect.poll(async () => (await pads(page)).map((p) => p.note)).toEqual([36]);
  await page.getByRole("button", { name: "redo" }).click();
  await expect.poll(async () => (await pads(page)).map((p) => p.note)).toEqual([36, 38]);

  expect(errors).toEqual([]);
});

// #761: a drum rack preset keeps the pads (notes, names, choke group) and their samplers with
// their samples; loading it onto another drum rack rebuilds them, and undo removes them in
// one step.
test("drum rack: a preset keeps the pads and loads them onto another drum rack", async ({ page }) => {
  test.setTimeout(150_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });

  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await newProject(page, `Drum Rack Presets ${Date.now()}`);

  // --- A kit: Kick on C1, Snare on D1 in choke group 1 ------------------------------------
  const midi = await createTrack(page, "Midi");
  await openLibrary(page);
  await expandFolder(page, "Demo Samples");
  const { id: rack, win } = await addDrumRack(page, midi.name);
  const grid = win.getByRole("grid", { name: "drum pads" });
  const files = page.getByRole("tree", { name: "folders" });
  const padsOf = async (r: string) => (await pads(page)).filter((p) => p.rack === r);
  await files.getByRole("treeitem", { name: "Kick.wav", exact: true }).dragTo(grid.getByRole("gridcell", { name: /^C1 / }));
  await expect.poll(async () => (await padsOf(rack)).map((p) => p.note), { timeout: 20_000 }).toEqual([36]);
  await files.getByRole("treeitem", { name: "Snare.wav", exact: true }).dragTo(grid.getByRole("gridcell", { name: /^D1 / }));
  await expect.poll(async () => (await padsOf(rack)).map((p) => p.note), { timeout: 20_000 }).toEqual([36, 38]);
  const settings = page.getByTestId("pad-settings");
  await expect(settings).toHaveAttribute("aria-label", "pad Snare");
  await pickOption(settings, "choke group", { value: "1" });
  await expect.poll(async () => (await padsOf(rack)).map((p) => p.choke_group)).toEqual([null, 1]);

  // --- Save it from the rack's preset menu -------------------------------------------------
  const name = (await doc(page)).devices[rack]!.name;
  await page.getByRole("button", { name: `close ${name} window` }).click();
  const presetsOf = (id: string) =>
    page
      .locator(`section[data-device="${id}"]`)
      .getByRole("button", { name: `presets for ${name}` })
      .first();
  const panel = page.getByRole("dialog", { name: `presets for ${name}` });
  await clickHeaderControl(presetsOf(rack));
  await panel.getByRole("button", { name: "save preset…" }).click();
  const save = page.getByRole("dialog", { name: `save preset for ${name}` });
  await save.getByRole("textbox", { name: "preset name" }).fill("E2E Kit");
  await save.getByRole("button", { name: "save" }).click();
  await expect(save).toBeHidden();

  // --- Load it onto a new drum rack: the same pads, with their samples ---------------------
  const midi2 = await createTrack(page, "Midi");
  const { id: rack2 } = await addDrumRack(page, midi2.name);
  await page.getByRole("button", { name: `close ${name} window` }).click();
  await clickHeaderControl(presetsOf(rack2));
  await panel
    .getByRole("region", { name: "user presets" })
    .getByRole("button", { name: /^E2E Kit/ })
    .click();
  const kit = async (r: string) => {
    const p = await doc(page);
    return Promise.all(
      (await padsOf(r)).map(async (pad) => ({
        note: pad.note,
        name: pad.name,
        choke: pad.choke_group,
        sampled: (await padChain(page, pad.id)).map((d) => {
          const sample = sampleOf(d);
          return sample !== null && sample in p.media;
        }),
      })),
    );
  };
  const expected = [
    { note: 36, name: "Kick", choke: null, sampled: [true] },
    { note: 38, name: "Snare", choke: 1, sampled: [true] },
  ];
  await expect.poll(() => kit(rack2), { timeout: 20_000 }).toEqual(expected);
  // New ids: the first rack keeps its own pads.
  expect(await kit(rack)).toEqual(expected);

  // --- One undo step back to the empty rack, and redo --------------------------------------
  await page.getByRole("button", { name: "undo" }).click();
  await expect.poll(async () => (await padsOf(rack2)).length).toBe(0);
  await page.getByRole("button", { name: "redo" }).click();
  await expect.poll(() => kit(rack2)).toEqual(expected);

  // Clean up the user preset (the OPFS library outlives the test).
  await clickHeaderControl(presetsOf(rack2));
  await panel.getByRole("region", { name: "user presets" }).getByRole("button", { name: "more actions for E2E Kit" }).click();
  await page.getByRole("menuitem", { name: "delete preset…" }).click();
  await page.getByRole("dialog", { name: "delete “E2E Kit”?" }).getByRole("button", { name: "delete" }).click();
  expect(errors).toEqual([]);
});
