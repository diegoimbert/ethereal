/**
 * Mock of `Chat::*` and `PinnedNote::*` (docs/COLLAB.md §12). Owned by `collab-social`.
 *
 * Contract stub (base-62): both reply `Unsupported`, like the engine, until the node lands.
 * The node then simulates them here: `PinnedNote::*` as ordinary undoable document edits
 * (the `Marker::*` pattern in `clipEditing.ts`); `Chat::Send` outside the undo history, with
 * `seq` = next in order and the oldest pruned past `CHAT_MAX_MESSAGES`, and `MockCollab`
 * simulating a peer's message (patch + `CollabEvent::ChatReceived`).
 */

import type { ChatCommand, PinnedNoteCommand, ReplyValue } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";

/** `Chat::*` (not a document command). */
export function chatCommand(c: ChatCommand): ReplyValue {
  return fail("Unsupported", `chat (${c.type}) is not implemented yet`);
}

/** `PinnedNote::*` (a document command). */
export function pinnedNoteCommand(ctx: ReducerContext, c: PinnedNoteCommand): void {
  void ctx;
  fail("Unsupported", `pinned notes (${c.type}) are not implemented yet`);
}
