/**
 * Mock of `Rack::*` and `Modulation::*` (v0.2, contracts-3). Owned by `racks-modulation`.
 * Rack chains, macros (rack params 0..8) and Bitwig-style modulators with mappings
 * (`ether_model::{rack, modulation}`). `ListModulatorKinds` already answers from the
 * generated `devices/modulators.json`.
 *
 * Until the node lands the document commands fail `Unsupported`, like the engine
 * (`crates/ether-controller/tests/roadmap_v3.rs`). Replace with the simulation.
 */

import type { ModulationCommand, RackCommand, ReplyValue } from "@/generated";
import { MODULATOR_DESCRIPTORS } from "../devices";
import { fail, type ReducerContext } from "../documentReducer";

/** Document command, dispatched from `roadmap/index.ts`. */
export function rackCommand(ctx: ReducerContext, c: RackCommand): void {
  void ctx;
  fail("Unsupported", `${c.type} is not implemented yet (racks-modulation)`);
}

/** Document command (all but `ListModulatorKinds`), dispatched from `roadmap/index.ts`. */
export function modulationCommand(ctx: ReducerContext, c: ModulationCommand): void {
  void ctx;
  fail("Unsupported", `${c.type} is not implemented yet (racks-modulation)`);
}

/** `Modulation::ListModulatorKinds` (runtime), dispatched from `MockTransport.execute`. */
export function listModulatorKinds(): ReplyValue {
  return { type: "ModulatorKinds", kinds: [...MODULATOR_DESCRIPTORS] };
}
