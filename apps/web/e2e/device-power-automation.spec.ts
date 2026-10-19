// device-power-automation: right-click a device's on/off button → "show automation lane"
// opens its "device on" lane in the arrangement; points drawn there (on, then off) switch
// the device during playback, and the button follows live. Against the real engine.
//
// No sleeps: every step waits on UI or engine state. The UI mirror is `window.__ether`.
import { expect, test } from "@playwright/test";
import { doc, installProbe, send } from "./perfProject";
import { createTrack, deviceCard, launch, newProject, openDeviceTab } from "./ui";

test.use({ viewport: { width: 1440, height: 900 } });

test("right-click the power button to automate the device's on/off", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await launch(page);
  await newProject(page, `Device power ${Date.now()}`);
  await installProbe(page);
  const midi = await createTrack(page, "Midi");
  const synth = Object.values((await doc(page)).devices).find((d) => d.track === midi.id)!;

  await openDeviceTab(page, midi.name);
  const power = deviceCard(page, synth.id).locator(".eth-device__power");
  await expect(power).toHaveAttribute("aria-pressed", "true");
  await power.click({ button: "right" });
  // The param menu (like a knob's), not the device header's.
  await expect(page.getByRole("menuitem", { name: "delete" })).toHaveCount(0);
  await page.getByRole("menuitem", { name: "show automation lane" }).click();
  const lane = page.getByRole("group", { name: /device on automation/i });
  await expect(lane).toBeVisible();

  // Two points: a new point starts at the lane's value there (not at the pointer's y), so both
  // are on; the second (selected once made) steps down to off. A toggle lane holds them, stepped.
  const svg = lane.getByTestId("automation-lane-svg");
  const box = (await svg.boundingBox())!;
  await svg.dblclick({ position: { x: 4, y: box.height * 0.2 } });
  await svg.dblclick({ position: { x: 120, y: box.height * 0.8 } });
  await expect.poll(async () => Object.keys((await doc(page)).automation_points).length).toBe(2);
  await page.keyboard.press("ArrowDown");
  await expect
    .poll(async () =>
      Object.values((await doc(page)).automation_points)
        .sort((a, b) => a.time - b.time)
        .map((p) => p.value),
    )
    .toEqual([1, 0]);
  await expect.poll(async () => Object.keys((await doc(page)).automation_points).length).toBe(2);
  const project = await doc(page);
  const switchLane = Object.values(project.automation_lanes).find((l) => l.target.type === "DeviceEnabled")!;
  expect(switchLane.target).toEqual({ type: "DeviceEnabled", device: synth.id });
  const pts = Object.values(project.automation_points).sort((a, b) => a.time - b.time);
  expect(pts.map((p) => p.value)).toEqual([1, 0]);
  expect(pts.every((p) => p.curve.type === "Step")).toBe(true);

  // Playing from the start: on, then off once the playhead passes the second point.
  await openDeviceTab(page, midi.name);
  await send(page, { domain: "Transport", command: { type: "Locate", position: 0 } });
  await expect(power).toHaveAttribute("aria-pressed", "true");
  await expect(power).toHaveAttribute("data-automated", "true");
  await send(page, { domain: "Transport", command: { type: "Play" } });
  await expect(power).toHaveAttribute("aria-pressed", "false", { timeout: 20_000 });
  await expect(deviceCard(page, synth.id)).toHaveClass(/eth-device--bypassed/);
  await send(page, { domain: "Transport", command: { type: "Stop" } });

  // The document's switch is untouched: automation drove it.
  expect((await doc(page)).devices[synth.id]?.enabled).toBe(true);
  expect(errors).toEqual([]);
});
