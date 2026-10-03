/**
 * Mock of `Expression::SetTrackMpe` (v0.3, contracts-4). Owned by `mpe`: a MIDI track's MPE
 * settings (`Track::mpe`, one undo step; `null` turns MPE off). Until the node lands it
 * fails `Unsupported`, like the engine (`crates/ether-controller/tests/roadmap_v4.rs`).
 */

import type { MpeSettings, TrackId } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";

export function setTrackMpe(ctx: ReducerContext, track: TrackId, mpe: MpeSettings | null): void {
  void ctx;
  void track;
  void mpe;
  fail("Unsupported", "MPE is not implemented yet (mpe)");
}
