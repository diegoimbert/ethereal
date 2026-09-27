/**
 * Mock of `Freeze::*` (v0.2, contracts-3). Owned by `freeze-bounce`. Render jobs reply `RenderStarted` and report `Event::Freeze` (simulate progress per playhead step, like `export.ts`).
 *
 * Until the node lands every command fails `Unsupported`, like the engine
 * (`crates/ether-controller/tests/roadmap_v3.rs`). Replace with the simulation.
 */

import type { FreezeCommand, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";

/** Runtime command, dispatched from `MockTransport.execute`. */
export function freezeCommand(c: FreezeCommand): ReplyValue {
  fail("Unsupported", `${c.type} is not implemented yet (freeze-bounce)`);
}
