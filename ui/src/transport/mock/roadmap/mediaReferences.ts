/**
 * Mock of `MediaRef::*` (v0.2, contracts-3). Owned by `media-references`. Missing media, relink and collect-all over the mock library.
 *
 * Until the node lands every command fails `Unsupported`, like the engine
 * (`crates/ether-controller/tests/roadmap_v3.rs`). Replace with the simulation.
 */

import type { MediaRefCommand, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";

/** Runtime command, dispatched from `MockTransport.execute`. */
export function mediaRefCommand(c: MediaRefCommand): ReplyValue {
  fail("Unsupported", `${c.type} is not implemented yet (media-references)`);
}
