/** Mock of `Collab::*`: unsupported (reserved). Owned by `collab`. */

import type { CollabCommand, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";

export function collabCommand(c: CollabCommand): ReplyValue {
  return fail("Unsupported", `collaboration is not available in the mock engine (${c.type})`);
}
