/**
 * Mock of `External::*` (v0.3). Owned by `external-instrument`; mirrors
 * `crates/ether-controller/src/external/mod.rs`:
 *
 * - `SetRouting` is a document command (the device's `ExternalRouting`, one undo step):
 *   external devices only; MIDI channel 1..=16; mono or stereo channel pairs; the instrument
 *   has no audio send and the effect no MIDI out (`InvalidArgument` otherwise).
 * - `ListPorts` replies a fixed demo interface ([`MOCK_PORTS`]) so the routing panel can be
 *   used in UI dev and tests (the web engine replies `Unsupported`).
 * - `MeasureLatency` stays `Unsupported`: the mock has no audio clock or hardware loop.
 */

import type { ExternalCommand, ExternalRouting, HardwarePorts, ReplyValue } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";

export type ExternalRoutingCommand = Extract<ExternalCommand, { type: "SetRouting" }>;

/** The mock's hardware: one MIDI out, a 4-in / 4-out audio interface. */
export const MOCK_PORTS: HardwarePorts = {
  midi_outputs: [{ id: "Mock Synth", name: "Mock Synth" }],
  audio_inputs: [0, 1, 2, 3].map((index) => ({ index, name: `In ${index + 1}` })),
  audio_outputs: [0, 1, 2, 3].map((index) => ({ index, name: `Out ${index + 1}` })),
};

/** Mirrors `ether_model::check_routing`. */
export function checkRouting(r: ExternalRouting): string | null {
  if (!Number.isInteger(r.midi_channel) || r.midi_channel < 1 || r.midi_channel > 16) return "external MIDI channel must be 1..=16";
  for (const ch of [r.audio_send, r.audio_return]) {
    if (!ch) continue;
    if (ch.count < 1 || ch.count > 2) return "hardware channels are mono (1) or stereo (2)";
    if (ch.first < 0 || ch.first + ch.count > 0xffff) return "hardware channel range overflows";
  }
  if (r.midi_out !== null && (r.midi_out.length === 0 || r.midi_out.length > 256)) return "MIDI output port id must be 1..=256 bytes";
  return null;
}

/** `External::SetRouting` (document command). */
export function setExternalRouting(ctx: ReducerContext, c: ExternalRoutingCommand): void {
  const d = ctx.tx.get("Device", c.device) ?? fail("NotFound", `device ${c.device}`);
  const kind = d.kind.type === "Builtin" ? d.kind.device.type : null;
  if (kind !== "ExternalInstrument" && kind !== "ExternalAudioEffect") fail("InvalidArgument", `device ${d.id} is not an external device`);
  const bad = checkRouting(c.routing);
  if (bad) fail("InvalidArgument", bad);
  if (kind === "ExternalInstrument" && c.routing.audio_send !== null) fail("InvalidArgument", "an External Instrument has no audio send");
  if (kind === "ExternalAudioEffect" && c.routing.midi_out !== null) fail("InvalidArgument", "an External Audio Effect has no MIDI output");
  ctx.tx.upsert("Device", { ...d, kind: { type: "Builtin", device: { type: kind, routing: { ...c.routing } } } });
}

/** `External::{ListPorts, MeasureLatency}` (runtime). */
export function externalCommand(c: Exclude<ExternalCommand, ExternalRoutingCommand>): ReplyValue {
  if (c.type === "ListPorts") return { type: "HardwarePorts", ports: structuredClone(MOCK_PORTS) };
  return fail("Unsupported", "latency measurement needs audio hardware (desktop app)");
}
