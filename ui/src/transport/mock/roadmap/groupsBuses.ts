/**
 * Mock of `Track::*` (v0.2, contracts-3). Owned by `groups-buses`. `Track::{GroupSelected, Ungroup, SetVca}` (called from the core track reducer's hook).
 *
 * Until the node lands every command fails `Unsupported`, like the engine
 * (`crates/ether-controller/tests/roadmap_v3.rs`). Replace with the simulation.
 */

import type { TrackCommand } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";

/** Document command (one undo step), dispatched from `roadmap/index.ts`. */
export function groupsTrackCommand(ctx: ReducerContext, c: TrackCommand): void {
  void ctx;
  fail("Unsupported", `${c.type} is not implemented yet (groups-buses)`);
}
