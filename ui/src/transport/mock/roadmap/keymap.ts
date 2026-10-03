/**
 * Mock of `Keymap::*` (v0.3, contracts-4). Owned by `keymap`: the user keymap stored in the
 * user library (`Get` → `Keymap`, `Set`, `Reset`; `KeymapEvent::Changed` after a change).
 * Until the node lands every command fails `Unsupported`, like the engine
 * (`crates/ether-controller/tests/roadmap_v4.rs`).
 */

import type { KeymapCommand, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";

export function keymapCommand(c: KeymapCommand): ReplyValue {
  return fail("Unsupported", `Keymap::${c.type} is not implemented yet (keymap)`);
}
