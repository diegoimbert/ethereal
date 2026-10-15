// Device on/off automation (`AutomationTarget::DeviceEnabled`) in the mock: lanes on a
// device's switch are accepted, follow a duplicated track, and die with the device.
import { describe, expect, it } from "vitest";
import { cmd } from "../cmd";
import { MockTransport } from "./MockTransport";

describe("device switch lanes (mock)", () => {
  it("accepts them, copies them with the track and deletes them with the device", async () => {
    const mock = new MockTransport({ timers: "manual", seed: 3 });
    await mock.connect();
    const device = Object.values(mock.snapshot().devices)[0]!;
    const track = device.track;
    await mock.send(
      cmd("Automation", {
        type: "CreateLane",
        id: "LANE",
        owner: { type: "Track", track },
        target: { type: "DeviceEnabled", device: device.id },
      }),
    );
    await mock.send(
      cmd("Automation", {
        type: "AddPoints",
        lane: "LANE",
        points: [
          { id: "P1", time: 0, value: 1, curve: { type: "Step" } },
          { id: "P2", time: 4, value: 0, curve: { type: "Step" } },
        ],
      }),
    );
    expect(mock.snapshot().automation_lanes.LANE?.target).toEqual({ type: "DeviceEnabled", device: device.id });

    // Unknown device: refused.
    await expect(
      mock.send(
        cmd("Automation", {
          type: "CreateLane",
          id: "BAD",
          owner: { type: "Track", track },
          target: { type: "DeviceEnabled", device: "NOPE" },
        }),
      ),
    ).rejects.toThrow();

    // A duplicated track gets a lane on its copy of the device.
    await mock.send(cmd("Track", { type: "Duplicate", id: track, new_id: "COPY" }));
    const p = mock.snapshot();
    const copyDevice = Object.values(p.devices).find((d) => d.track === "COPY" && d.name === device.name)!;
    expect(Object.values(p.automation_lanes).some((l) => l.target.type === "DeviceEnabled" && l.target.device === copyDevice.id)).toBe(true);

    await mock.send(cmd("Device", { type: "Remove", id: device.id }));
    expect(mock.snapshot().automation_lanes.LANE).toBeUndefined();
  });
});
