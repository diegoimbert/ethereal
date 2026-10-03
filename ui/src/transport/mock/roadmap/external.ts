/**
 * Mock of `External::*` (v0.3, contracts-4). Owned by `external-instrument`: `SetRouting`
 * is a document command (the device's `ExternalRouting`, one undo step); `ListPorts` and
 * `MeasureLatency` are runtime (hardware) commands, and the mock has no hardware (the node
 * may reply empty ports and keep `MeasureLatency` `Unsupported`). The devices insert as
 * placeholders (descriptors in `../devices/external.json`). Until the node lands every
 * command fails `Unsupported`, like the engine (`crates/ether-controller/tests/roadmap_v4.rs`).
 */

import type { ExternalCommand, ReplyValue } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";

export type ExternalRoutingCommand = Extract<ExternalCommand, { type: "SetRouting" }>;

/** `External::SetRouting` (document command). */
export function setExternalRouting(ctx: ReducerContext, c: ExternalRoutingCommand): void {
  void ctx;
  fail("Unsupported", `External::${c.type} is not implemented yet (external-instrument)`);
}

/** `External::{ListPorts, MeasureLatency}` (runtime). */
export function externalCommand(c: Exclude<ExternalCommand, ExternalRoutingCommand>): ReplyValue {
  return fail("Unsupported", `External::${c.type} is not implemented yet (external-instrument)`);
}
