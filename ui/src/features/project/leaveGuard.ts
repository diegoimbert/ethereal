/**
 * Guard for actions that switch, create or delete a project. An extension may ask first
 * (e.g. leaving a collaboration session; `@/ext` "project.leaveGuard"); without one the action
 * runs at once. `confirm` is the label of the confirming button, if the guard asks.
 */
import { contributions } from "@/ext";

/** Run `action`, or let a guard ask first. Resolves `true` once it ran, `false` when cancelled. */
export function guardLeave(action: () => void | Promise<unknown>, confirm?: string): Promise<boolean> {
  for (const guard of contributions("project.leaveGuard")) {
    const decided = guard(action, confirm);
    if (decided) return decided;
  }
  void action();
  return Promise.resolve(true);
}
