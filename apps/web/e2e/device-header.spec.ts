// #790: a device card's header hides its presets menu and "+" (modulator) until hovered, and
// then they take no room: the name uses the whole width. Hovering shows them right after
// the name, so only the name shrinks: the power button and the controls on the right stay put.
// Checked on an instrument, an audio effect, a rack and a MIDI effect.
import { expect, test, type Locator, type Page } from "@playwright/test";
import { addDevice, createTrack, launch, openDeviceTab } from "./ui";

const card = (page: Page, name: string): Locator => page.getByTestId("inspector").getByRole("region", { name, exact: true });

async function width(el: Locator): Promise<number> {
  return (await el.boundingBox())!.width;
}
async function left(el: Locator): Promise<number> {
  return (await el.boundingBox())!.x;
}

test("the device name takes the hover-only controls' room while they're hidden", async ({ page }) => {
  test.setTimeout(120_000);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await launch(page);
  const track = await createTrack(page, "Midi");
  await openDeviceTab(page, track.name);
  await addDevice(page, "Compressor");
  await addDevice(page, "AudioEffectRack");
  await addDevice(page, "Arpeggiator");

  const names = ["synth", "compressor", "audio effect rack", "arpeggiator"];
  for (const name of names) {
    const c = card(page, name);
    const header = c.locator(":scope > header");
    const label = header.locator(".eth-device__name");
    const slot = header.getByTestId("device-on-hover");
    const power = header.locator(".eth-device__power");
    const remove = header.getByRole("button", { name: `Remove ${name}` });
    await header.scrollIntoViewIfNeeded();

    // Not hovered: the hover-only controls take no width.
    await page.mouse.move(0, 0);
    await expect.poll(() => width(slot)).toBe(0);
    const idle = { name: await width(label), power: await left(power), remove: await left(remove) };

    // Hovered: presets and "+" show, the name gives them its room, nothing else moves.
    await header.hover({ position: { x: (await width(header)) / 2, y: 4 } });
    await expect(slot.getByRole("button", { name: `Presets for ${name}` })).toBeVisible();
    await expect.poll(() => width(slot)).toBeGreaterThan(0);
    const hovered = await width(label);
    expect(idle.name, `${name}: wider without hover`).toBeGreaterThan(hovered);
    expect(await left(power)).toBe(idle.power);
    expect(await left(remove)).toBe(idle.remove);
  }

  // Keyboard: focusing the presets menu shows the controls without hover.
  await page.mouse.move(0, 0);
  const slot = card(page, "compressor").locator(":scope > header").getByTestId("device-on-hover");
  await slot.getByRole("button", { name: "presets for compressor" }).focus();
  await expect.poll(() => width(slot)).toBeGreaterThan(0);
});

// Owner: hovering a device shows presets and "+" with no animation, and never moves or
// squeezes the open-window and on/off buttons, however narrow the card. With too little
// room, the hover-only controls are clipped whole instead.
test("hovering a device header never moves the window and power buttons, at any width", async ({ page }) => {
  test.setTimeout(120_000);
  await launch(page);
  const track = await createTrack(page, "Midi");
  await openDeviceTab(page, track.name);
  await addDevice(page, "Compressor");

  const c = card(page, "compressor");
  const header = c.locator(":scope > header");
  const slot = header.getByTestId("device-on-hover");
  const fixed = {
    window: header.getByRole("button", { name: "open compressor window" }),
    power: header.locator(".eth-device__power"),
    remove: header.getByRole("button", { name: "remove compressor" }),
  };
  await header.scrollIntoViewIfNeeded();
  // No animation: nothing on the slot transitions.
  expect(await slot.evaluate((el) => getComputedStyle(el).transitionDuration)).toBe("0s");

  const boxes = async () => Object.fromEntries(await Promise.all(Object.entries(fixed).map(async ([k, el]) => [k, (await el.boundingBox())!] as const)));
  await page.mouse.move(0, 0);
  const full = (await header.boundingBox())!.width;
  const nameRoom = await width(header.locator(".eth-device__name"));
  const presets = slot.getByRole("button", { name: "presets for compressor" });
  await presets.focus();
  const presetsWidth = await width(presets);
  await presets.blur();
  // Wide: the inspector's width. Room for presets but not +. Narrow: the name keeps a few
  // pixels, far less than presets and + need. Then no room at all for the name.
  for (const [label, cardWidth] of [
    ["wide", full],
    ["presets only", full - nameRoom + presetsWidth + 4],
    ["narrow", full - nameRoom + 8],
    ["no room for the name", full - nameRoom],
  ] as const) {
    await c.evaluate((el, w) => {
      (el as HTMLElement).style.width = `${w}px`;
      (el as HTMLElement).style.flex = "none";
    }, cardWidth);
    await page.mouse.move(0, 0);
    await expect.poll(() => width(slot)).toBe(0);
    const idle = await boxes();

    const h = (await header.boundingBox())!;
    await page.mouse.move(h.x + h.width / 2, h.y + h.height / 2);
    await expect.poll(() => slot.evaluate((el) => getComputedStyle(el).opacity)).toBe("1");
    expect(await boxes(), `${label}: hovering moved or resized a fixed button`).toEqual(idle);
    // Whatever shows of the hover-only controls stays clear of the buttons on the right.
    const slotBox = (await slot.boundingBox())!;
    expect(slotBox.x + slotBox.width, label).toBeLessThanOrEqual(idle.remove!.x);
    // Each hover-only button shows whole or not at all, never cut in half.
    for (const button of await slot.getByRole("button").all()) {
      const b = (await button.boundingBox())!;
      const inside = b.x >= slotBox.x && b.x + b.width <= slotBox.x + slotBox.width && b.y >= slotBox.y && b.y + b.height <= slotBox.y + slotBox.height;
      const outside = b.x >= slotBox.x + slotBox.width || b.y >= slotBox.y + slotBox.height;
      expect(inside || outside, `${label}: ${await button.getAttribute("aria-label")} is cut`).toBe(true);
    }
    if (label !== "narrow" && label !== "no room for the name") await expect(presets).toBeInViewport({ ratio: 1 });
  }
});
