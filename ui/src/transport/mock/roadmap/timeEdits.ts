/**
 * Mock of `TimeEdit::*` (v0.2, contracts-3). Owned by `time-edits`. Split across tracks, cut/copy/paste/delete time, insert silence; `Copy` fills a mock time clipboard.
 *
 * Until the node lands every command fails `Unsupported`, like the engine
 * (`crates/ether-controller/tests/roadmap_v3.rs`). Replace with the simulation.
 */

import type { TimeEditCommand, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";

/** Runtime command, dispatched from `MockTransport.execute`. */
export function timeEditCommand(c: TimeEditCommand): ReplyValue {
  fail("Unsupported", `${c.type} is not implemented yet (time-edits)`);
}
