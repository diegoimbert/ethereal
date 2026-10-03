/**
 * Mock of `Template::*` (v0.3, contracts-4). Owned by `templates`: project and track
 * templates in the user library (`List`, `SaveProject`, `SaveTracks`, `Rename`, `Delete`,
 * `SetDefault`, `NewProject`); `Insert` (tracks from a template) is a document command,
 * one undo step. Until the node lands every command fails `Unsupported`, like the engine
 * (`crates/ether-controller/tests/roadmap_v4.rs`).
 */

import type { ReplyValue, TemplateCommand } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";

export type InsertTemplateCommand = Extract<TemplateCommand, { type: "Insert" }>;

/** `Template::Insert` (document command). */
export function insertTemplate(ctx: ReducerContext, c: InsertTemplateCommand): void {
  void ctx;
  fail("Unsupported", `Template::${c.type} is not implemented yet (templates)`);
}

/** Every other `Template::*` (user library / project store). */
export function templateCommand(c: Exclude<TemplateCommand, InsertTemplateCommand>): ReplyValue {
  return fail("Unsupported", `Template::${c.type} is not implemented yet (templates)`);
}
