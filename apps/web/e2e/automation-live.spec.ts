// automation-live: automated values show live in the UI, against the real engine.
//
// A synth switched off in the document with "Device On" automation (on until beat 2, then
// off), like an imported Ableton device whose switch is automated (AWIP 115's Auto Filter),
// and a volume lane: the device's power button and the track's header fader follow the
// automation at the engine's playhead, stopped (locate) and while playing.
//
// No sleeps: every step waits on UI or engine state. The UI mirror is `window.__ether`.
import { expect, test } from "@playwright/test";
import { doc, installProbe, send, sendBatched, ulid } from "./perfProject";
import { deviceCard, launch, newProject, openDeviceTab } from "./ui";

test.use({ viewport: { width: 1440, height: 900 } });

test("device on/off and volume follow their automation at the playhead", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await launch(page);
  await newProject(page, `Live automation ${Date.now()}`);
  await installProbe(page);

  const track = ulid(1, 0);
  const synth = ulid(8, 0);
  const onLane = ulid(0, 0);
  const volLane = ulid(0, 1);
  await sendBatched(page, [
    { domain: "Track", command: { type: "Create", id: track, kind: "Midi", name: "Live", color: null, parent: null, before: null } },
    { domain: "Device", command: { type: "Insert", id: synth, track, device: { type: "Builtin", device: { type: "Synth" } }, before: null } },
    { domain: "Device", command: { type: "SetEnabled", id: synth, enabled: false } },
    { domain: "Automation", command: { type: "CreateLane", id: onLane, owner: { type: "Track", track }, target: { type: "DeviceEnabled", device: synth } } },
    {
      domain: "Automation",
      command: {
        type: "AddPoints",
        lane: onLane,
        points: [
          { id: ulid(2, 0), time: 0, value: 1, curve: { type: "Step" } },
          { id: ulid(2, 1), time: 2, value: 0, curve: { type: "Step" } },
        ],
      },
    },
    { domain: "Automation", command: { type: "CreateLane", id: volLane, owner: { type: "Track", track }, target: { type: "TrackVolume", track } } },
    {
      domain: "Automation",
      command: {
        type: "AddPoints",
        lane: volLane,
        points: [
          { id: ulid(2, 2), time: 0, value: 0.2, curve: { type: "Step" } },
          { id: ulid(2, 3), time: 2, value: 0.9, curve: { type: "Step" } },
        ],
      },
    },
  ]);
  expect((await doc(page)).devices[synth]?.enabled).toBe(false);

  await openDeviceTab(page, "Live");
  const power = deviceCard(page, synth).locator(".eth-device__power");
  // The track header's fader and the inspector's volume knob.
  const fader = page.getByTestId("arrangement-content").getByRole("slider", { name: "Live volume" });
  const knob = page.getByTestId("inspector").getByRole("slider", { name: "Live volume" });
  const faderValue = () => fader.getAttribute("aria-valuenow").then(Number);
  const knobValue = () => knob.getAttribute("aria-valuenow").then(Number);

  // Stopped at beat 1: automation says on (the document says off).
  await send(page, { domain: "Transport", command: { type: "Locate", position: 1 } });
  await expect(power).toHaveAttribute("aria-pressed", "true");
  await expect(power).toHaveAttribute("data-automated", "true");
  const low = await faderValue();
  const lowKnob = await knobValue();
  // Beat 3: off, and the volume stepped up.
  await send(page, { domain: "Transport", command: { type: "Locate", position: 3 } });
  await expect(power).toHaveAttribute("aria-pressed", "false");
  await expect.poll(faderValue).toBeGreaterThan(low);
  await expect.poll(knobValue).toBeGreaterThan(lowKnob);

  // Playing from the start: on, then off once the playhead passes beat 2 (live, no edit).
  await send(page, { domain: "Transport", command: { type: "Locate", position: 0 } });
  await expect(power).toHaveAttribute("aria-pressed", "true");
  await send(page, { domain: "Transport", command: { type: "Play" } });
  await expect(power).toHaveAttribute("aria-pressed", "false", { timeout: 15_000 });
  await send(page, { domain: "Transport", command: { type: "Stop" } });

  // The document's switch is untouched.
  expect((await doc(page)).devices[synth]?.enabled).toBe(false);
  expect(errors).toEqual([]);
});
