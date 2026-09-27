/**
 * Mock of `Preset::*` (v0.2, contracts-3). Owned by `presets`. Factory presets per device + an in-memory user preset store; `Load` applies params as one undo step.
 *
 * Until the node lands every command fails `Unsupported`, like the engine
 * (`crates/ether-controller/tests/roadmap_v3.rs`). Replace with the simulation.
 */

import type { PresetCommand, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";

/** Runtime command, dispatched from `MockTransport.execute`. */
export function presetCommand(c: PresetCommand): ReplyValue {
  fail("Unsupported", `${c.type} is not implemented yet (presets)`);
}
