/**
 * Guard for actions that switch, create or delete a project. In this build there are no
 * collaboration sessions, so the action always runs at once; the guard keeps the call sites
 * (and their confirm labels) unchanged.
 */

/** Run `action` now. Resolves `true` once it ran (never `false`: there is no session to leave). */
export function guardLeave(action: () => void | Promise<unknown>, _confirm?: string): Promise<boolean> {
  void action();
  return Promise.resolve(true);
}
