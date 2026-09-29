/** MockTransport: `Preset` (v0.2, presets). */
import { describe, expect, it } from "vitest";
import type { Device, PresetInfo, ReplyValue } from "@/generated";
import { cmd } from "../../cmd";
import { deviceKey, fileStem } from "./presets";
import { project, undo, useMock } from "./testUtils";

const presetsOf = (r: ReplyValue): PresetInfo[] => (r.type === "Presets" ? r.presets : []);
const presetOf = (r: ReplyValue): PresetInfo => {
  if (r.type !== "Preset") throw new Error(r.type);
  return r.preset;
};

describe("MockTransport Preset (presets)", () => {
  const f = useMock();
  const synth = (): Device => Object.values(project(f).devices).find((d) => d.kind.type === "Builtin" && d.kind.device.type === "Synth")!;

  it("lists the embedded factory presets, sorted and filtered", async () => {
    const all = presetsOf(
      await f.mock.send(
        cmd("Preset", {
          type: "List",
          device: { type: "Builtin", device: "Synth" },
          text: null,
        }),
      ),
    );
    expect(all.map((p) => p.name)).toEqual(["Glass Keys", "Pluck", "Soft Pad", "Square Lead", "Sub Bass"]);
    expect(all.every((p) => p.preset.source === "Factory" && p.preset.id.startsWith("synth/"))).toBe(true);
    const bass = presetsOf(await f.mock.send(cmd("Preset", { type: "List", device: null, text: "BASS" })));
    expect(bass.map((p) => p.preset.id)).toContain("synth/sub-bass");
  });

  it("loads a preset as one undo step", async () => {
    const d = synth();
    await f.mock.send(
      cmd("Preset", {
        type: "Load",
        device: d.id,
        preset: { source: "Factory", id: "synth/soft-pad" },
      }),
    );
    expect(project(f).devices[d.id]!.params[2]).toBe(800);
    expect(project(f).devices[d.id]!.params[6]).toBe(2500);
    await undo(f);
    expect(project(f).devices[d.id]!.params).toEqual(d.params);
    await expect(
      f.mock.send(
        cmd("Preset", {
          type: "Load",
          device: d.id,
          preset: { source: "Factory", id: "compressor/gentle-glue" },
        }),
      ),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
  });

  it("saves, renames, edits and deletes user presets", async () => {
    const d = synth();
    await f.mock.send(cmd("Device", { type: "SetParam", device: d.id, param: 6, value: 1234 }));
    const saved = presetOf(
      await f.mock.send(
        cmd("Preset", {
          type: "Save",
          device: d.id,
          name: " My Pad ",
          meta: { tags: ["Pad", "pad"], author: null, description: null },
          overwrite: false,
        }),
      ),
    );
    expect(saved).toMatchObject({
      name: "My Pad",
      preset: { source: "User", id: "synth/My Pad.etherpreset" },
      meta: { tags: ["pad"] },
    });
    expect(f.events.some((e) => e.type === "Preset")).toBe(true);
    await expect(
      f.mock.send(
        cmd("Preset", {
          type: "Save",
          device: d.id,
          name: "my pad",
          meta: { tags: [], author: null, description: null },
          overwrite: false,
        }),
      ),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
    const renamed = presetOf(
      await f.mock.send(
        cmd("Preset", {
          type: "Rename",
          preset: saved.preset,
          name: "Dark Pad",
        }),
      ),
    );
    expect(renamed.preset.id).toBe("synth/Dark Pad.etherpreset");
    await f.mock.send(cmd("Device", { type: "SetParam", device: d.id, param: 6, value: 500 }));
    await f.mock.send(cmd("Preset", { type: "Load", device: d.id, preset: renamed.preset }));
    expect(project(f).devices[d.id]!.params[6]).toBe(1234);
    const list = presetsOf(
      await f.mock.send(
        cmd("Preset", {
          type: "List",
          device: { type: "Builtin", device: "Synth" },
          text: null,
        }),
      ),
    );
    expect(list.at(-1)?.name).toBe("Dark Pad");
    await f.mock.send(cmd("Preset", { type: "Delete", preset: renamed.preset }));
    await expect(f.mock.send(cmd("Preset", { type: "Delete", preset: renamed.preset }))).rejects.toMatchObject({ code: "NotFound" });
    await expect(
      f.mock.send(
        cmd("Preset", {
          type: "Delete",
          preset: { source: "Factory", id: "synth/pluck" },
        }),
      ),
    ).rejects.toMatchObject({ code: "InvalidArgument" });
  });

  it("names folders and files like the engine", () => {
    expect(deviceKey("PolySynth")).toBe("poly-synth");
    expect(deviceKey("Eq")).toBe("eq");
    expect(fileStem("a/b:c?")).toBe("a-b-c-");
    expect(fileStem("  ..hidden. ")).toBe("hidden");
    expect(fileStem("")).toBe("Preset");
  });
});
