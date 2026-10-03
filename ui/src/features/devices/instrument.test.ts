import { describe, expect, it } from "vitest";
import type { Command, Device, DeviceDescriptor, DeviceId, ReplyValue } from "@/generated";
import { cmd, type EngineTransport } from "@/transport";
import { addInstrumentCommand } from "./instrument";

const device = (id: string, kind: "builtin" | "plugin", chain: string | null = null) =>
  ({
    id,
    track: "t",
    chain,
    kind:
      kind === "builtin"
        ? { type: "Builtin", device: { type: "Synth" } }
        : { type: "Plugin", plugin: { plugin_id: `p.${id}`, format: "Clap" } },
  }) as unknown as Device;

/** Answers `Device::GetDescriptor` with a category per device id (missing = an error). */
const describing = (categories: Record<string, DeviceDescriptor["category"]>): EngineTransport =>
  ({
    send: (c: Command) => {
      const id = c.domain === "Device" && c.command.type === "GetDescriptor" ? c.command.device : null;
      const category = id ? categories[id] : undefined;
      if (!category) return Promise.reject(new Error("no descriptor"));
      return Promise.resolve({ type: "Descriptor", descriptor: { category } } as ReplyValue);
    },
  }) as unknown as EngineTransport;

const insert = (before: DeviceId | null) => cmd("Device", { type: "Remove", id: `insert-before:${before}` });

describe("addInstrumentCommand", () => {
  it("replaces the top-level instrument (a plugin counts) in place, as one step", async () => {
    const chain = [device("fx", "plugin"), device("inst", "plugin")];
    const c = await addInstrumentCommand(describing({ fx: "AudioEffect", inst: "Instrument" }), chain, insert);
    expect(c).toEqual(
      cmd("Edit", { type: "Batch", label: "Replace Instrument", commands: [insert("inst"), cmd("Device", { type: "Remove", id: "inst" })] }),
    );
  });

  it("goes first when the chain has no instrument (rack-chain instruments and unknown devices don't count)", async () => {
    const chain = [device("fx", "plugin"), device("nested", "plugin", "rack-chain"), device("loading", "plugin")];
    const t = describing({ fx: "AudioEffect", nested: "Instrument" });
    expect(await addInstrumentCommand(t, chain, insert)).toEqual(insert("fx"));
    expect(await addInstrumentCommand(t, [], insert)).toEqual(insert(null));
  });

  it("with no instrument, goes after the MIDI effects (they feed it)", async () => {
    const t = describing({ arp: "NoteEffect", scale: "NoteEffect", fx: "AudioEffect" });
    const chain = [device("arp", "plugin"), device("scale", "plugin"), device("fx", "plugin")];
    expect(await addInstrumentCommand(t, chain, insert)).toEqual(insert("fx"));
    expect(await addInstrumentCommand(t, chain.slice(0, 2), insert)).toEqual(insert(null));
  });
});
