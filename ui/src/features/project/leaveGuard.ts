/**
 * Guard for actions that switch, create or delete a project. Leaving an untitled project asks
 * to keep or discard it first (`untitledLeave.ts`); an extension may ask too (e.g. leaving a
 * collaboration session; `@/ext` "project.leaveGuard"); without either the action runs at
 * once. `confirm` is the label of the extension's confirming button, if it asks.
 */
import { contributions } from "@/ext";
import { guardUntitled, untitledOpen } from "./untitledLeave";

export interface GuardLeaveOptions {
  /** Don't ask about an untitled project (the action deletes the open project anyway). */
  untitled?: boolean;
}

/** Run `action`, or let a guard ask first. Resolves `true` once it ran, `false` when cancelled. */
export function guardLeave(action: () => void | Promise<unknown>, confirm?: string, { untitled = true }: GuardLeaveOptions = {}): Promise<boolean> {
  // Not asking: synchronous, as before (the action starts before this returns).
  if (!untitled || !untitledOpen()) return guardOthers(action, confirm);
  return guardUntitled(() => guardOthers(action, confirm));
}

/**
 * Only the untitled project's question, for a way out that has its own guards (an extension
 * joining a session). Resolves `true` once `action` ran, `false` when cancelled.
 */
export function guardUntitledLeave(action: () => void | Promise<unknown>): Promise<boolean> {
  return guardUntitled(() => {
    void action();
    return Promise.resolve(true);
  });
}

function guardOthers(action: () => void | Promise<unknown>, confirm?: string): Promise<boolean> {
  for (const guard of contributions("project.leaveGuard")) {
    const decided = guard(action, confirm);
    if (decided) return decided;
  }
  void action();
  return Promise.resolve(true);
}
