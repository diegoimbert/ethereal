import type { Command, Device, DeviceId } from "@/generated";
import { cmd, type EngineTransport } from "@/transport";
import { asOneStep } from "@/features/arrangement/editMath";
import { fetchDescriptor } from "./descriptors";

/**
 * The command adding an instrument to a chain (Ableton behaviour): it replaces the chain's
 * top-level instrument (built-in or plugin, by descriptor category), in place, as one undo
 * step; with none it goes first (its output feeds the effects). `insert(before)` builds the
 * new instrument's `Device::Insert`. A trailing instrument gets no notes (the engine only
 * forwards MIDI device to device) and would clear the new one's audio.
 */
export async function addInstrumentCommand(
  transport: EngineTransport,
  devices: ReadonlyArray<Device>,
  insert: (before: DeviceId | null) => Command,
): Promise<Command> {
  for (const d of devices) {
    if (d.chain) continue;
    const descriptor = await fetchDescriptor(transport, d).catch(() => null);
    if (descriptor?.category === "Instrument")
      return asOneStep("Replace Instrument", [insert(d.id), cmd("Device", { type: "Remove", id: d.id })])!;
  }
  return insert(devices[0]?.id ?? null);
}
