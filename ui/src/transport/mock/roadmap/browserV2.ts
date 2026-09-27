/**
 * Mock of `Browser::*` (v0.2, contracts-3). Owned by `browser-v2`. An in-memory index over `library.ts` (search, tags, favourites, paging).
 *
 * Until the node lands every command fails `Unsupported`, like the engine
 * (`crates/ether-controller/tests/roadmap_v3.rs`). Replace with the simulation.
 */

import type { BrowserCommand, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";

/** Runtime command, dispatched from `MockTransport.execute`. */
export function browserCommand(c: BrowserCommand): ReplyValue {
  fail("Unsupported", `${c.type} is not implemented yet (browser-v2)`);
}
