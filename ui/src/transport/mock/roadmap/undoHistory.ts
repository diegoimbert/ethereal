/**
 * Mock of `History::*` (v0.3, contracts-4). Owned by `undo-history`: the undo history as a
 * list (`List` → `History`), jumping to a step (`JumpTo`, an undo/redo run) and named
 * checkpoints (`SetCheckpoint`), with `HistoryEvent::Changed` pushed after every recorded
 * step. Collab-aware: only this site's own steps are listed. Until the node lands every
 * command fails `Unsupported`, like the engine (`crates/ether-controller/tests/roadmap_v4.rs`).
 */

import type { HistoryCommand, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";

export function historyCommand(c: HistoryCommand): ReplyValue {
  return fail("Unsupported", `History::${c.type} is not implemented yet (undo-history)`);
}
