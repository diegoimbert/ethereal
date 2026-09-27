/** MockTransport: drum racks and slices (drum-rack). */
import { describe, expect, it } from "vitest";
import { cmd } from "../../cmd";
import { newId } from "../../ids";
import { devicesOfPad, devicesOfTrack } from "@/state/selectors";
import { project, trackNamed, undo, useMock } from "./testUtils";

describe("MockTransport drumRack", () => {
  const f = useMock();

  it("drum racks: pads, note swap, pad chains, cascade on rack removal", async () => {
    const t = trackNamed(f, "Keys");
    const rack = newId();
    await f.mock.send(cmd("Device", { type: "Insert", id: rack, track: t.id, device: { type: "Builtin", device: { type: "DrumRack" } }, before: null }));
    const [a, b] = [newId(), newId()];
    await f.mock.send(cmd("DrumRack", { type: "AddPad", id: a, rack, note: 36, name: null }));
    await f.mock.send(cmd("DrumRack", { type: "AddPad", id: b, rack, note: 38, name: "Snare" }));
    expect(project(f).drum_pads[a]).toMatchObject({ note: 36, name: "C1", choke_group: null });
    await f.mock.send(cmd("DrumRack", { type: "SetPadNote", id: a, note: 38 }));
    expect([project(f).drum_pads[a]!.note, project(f).drum_pads[b]!.note]).toEqual([38, 36]);
    const sampler = newId();
    await f.mock.send(
      cmd("DrumRack", {
        type: "InsertDevice",
        id: sampler,
        pad: a,
        device: { type: "Builtin", device: { type: "Sampler", sample: null, slices: { enabled: false, base_note: 36, markers: [] } } },
        before: null,
      }),
    );
    expect(project(f).devices[sampler]).toMatchObject({ pad: a, track: t.id });
    await f.mock.send(cmd("Device", { type: "Remove", id: rack }));
    expect(project(f).devices[sampler]).toBeUndefined();
    expect(Object.keys(project(f).drum_pads)).toHaveLength(0);
    await undo(f);
    expect(project(f).devices[sampler]).toBeDefined();
    expect(Object.keys(project(f).drum_pads)).toHaveLength(2);
  });
  it("structure rules: pad devices and racks with pads don't Device::Move; duplicates copy pads", async () => {
    const t = trackNamed(f, "Keys");
    const other = trackNamed(f, "Bass");
    const rack = newId();
    await f.mock.send(cmd("Device", { type: "Insert", id: rack, track: t.id, device: { type: "Builtin", device: { type: "DrumRack" } }, before: null }));
    const pad = newId();
    await f.mock.send(cmd("DrumRack", { type: "AddPad", id: pad, rack, note: 36, name: null }));
    const [s1, s2] = [newId(), newId()];
    for (const id of [s1, s2]) {
      await f.mock.send(cmd("DrumRack", { type: "InsertDevice", id, pad, device: { type: "Builtin", device: { type: "Delay" } }, before: null }));
    }
    // Pad devices are not in the track chain.
    const chain = devicesOfTrack(project(f), t.id).map((d) => d.id);
    expect(chain).toContain(rack);
    expect(chain).not.toContain(s1);
    expect(devicesOfPad(project(f), pad).map((d) => d.id)).toEqual([s1, s2]);
    await expect(f.mock.send(cmd("Device", { type: "Move", id: s1, track: t.id, before: null }))).rejects.toMatchObject({
      code: "InvalidArgument",
    });
    await expect(f.mock.send(cmd("Device", { type: "Move", id: rack, track: other.id, before: null }))).rejects.toMatchObject({
      code: "InvalidArgument",
    });

    // Duplicating a pad device keeps it in the pad chain, right after the original.
    const dup = newId();
    await f.mock.send(cmd("Device", { type: "Duplicate", id: s1, new_id: dup }));
    expect(project(f).devices[dup]!.pad).toBe(pad);
    expect(devicesOfPad(project(f), pad).map((d) => d.id)).toEqual([s1, dup, s2]);
    await undo(f);

    // Duplicating the rack copies its pads and chains with new ids.
    const rack2 = newId();
    await f.mock.send(cmd("Device", { type: "Duplicate", id: rack, new_id: rack2 }));
    const pads2 = Object.values(project(f).drum_pads).filter((x) => x.rack === rack2);
    expect(pads2.map((x) => x.note)).toEqual([36]);
    expect(pads2[0]!.id).not.toBe(pad);
    expect(devicesOfPad(project(f), pads2[0]!.id)).toHaveLength(2);
    await undo(f);

    // Duplicating the track copies the rack with its pads and chains.
    const copy = newId();
    await f.mock.send(cmd("Track", { type: "Duplicate", id: t.id, new_id: copy }));
    const rackCopy = devicesOfTrack(project(f), copy).find((d) => d.kind.type === "Builtin" && d.kind.device.type === "DrumRack")!;
    const padCopy = Object.values(project(f).drum_pads).find((x) => x.rack === rackCopy.id)!;
    const chainCopy = devicesOfPad(project(f), padCopy.id);
    expect(chainCopy).toHaveLength(2);
    expect(chainCopy.every((d) => d.track === copy)).toBe(true);
  });
});
