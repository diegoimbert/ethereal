/**
 * Mock of `Take::*` (v0.2, contracts-3). Owned by `comping`. Take lanes, swipe comping (`SetComp`), flatten; data model in `ether_model::take`.
 *
 * Until the node lands every command fails `Unsupported`, like the engine
 * (`crates/ether-controller/tests/roadmap_v3.rs`). Replace with the simulation.
 */

import type { TakeCommand } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";

/** Document command (one undo step), dispatched from `roadmap/index.ts`. */
export function takeCommand(ctx: ReducerContext, c: TakeCommand): void {
  void ctx;
  fail("Unsupported", `${c.type} is not implemented yet (comping)`);
}
