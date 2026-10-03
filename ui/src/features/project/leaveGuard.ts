// "Leave the session?" (base-114): opening, creating or saving as another project while in a
// collaboration session ends this device's part in it (the engine leaves when the open
// project changes). Ask first; on "Leave & open" leave explicitly, then run the action.
import { create } from "zustand";
import { useCollabStore } from "@/features/collab/store";
import { cmd, type EngineTransport } from "@/transport";

interface Pending {
  session: string;
  /** Confirm button label ("Leave & open", "Leave & create", ...). */
  confirm: string;
  action(): void | Promise<unknown>;
  /** Settles the `guardLeave` promise. */
  settle(ran: boolean): void;
}

export const useLeaveGuard = create<{ pending: Pending | null }>()(() => ({
  pending: null,
}));

/** The session this device is in (connecting counts), or `null`. */
export function currentSession(): string | null {
  const s = useCollabStore.getState().status;
  return s.type === "Offline" ? null : s.session;
}

/**
 * Run `action` now when not in a session; otherwise ask first (the dialog's confirm button
 * reads `confirm`). Resolves `true` once it ran, `false` when cancelled.
 */
export function guardLeave(action: () => void | Promise<unknown>, confirm = "Leave & open"): Promise<boolean> {
  const session = currentSession();
  if (session === null) {
    void action();
    return Promise.resolve(true);
  }
  // A newer request replaces an unanswered one (which counts as cancelled).
  useLeaveGuard.getState().pending?.settle(false);
  return new Promise((resolve) =>
    useLeaveGuard.setState({
      pending: { session, confirm, action, settle: resolve },
    }),
  );
}

/** "Cancel" (or Escape): the action does not run. */
export function cancelLeave() {
  const p = useLeaveGuard.getState().pending;
  useLeaveGuard.setState({ pending: null });
  p?.settle(false);
}

/** "Leave & open": leave the session, then run the action. */
export async function confirmLeave(transport: EngineTransport | null) {
  const p = useLeaveGuard.getState().pending;
  useLeaveGuard.setState({ pending: null });
  if (!p) return;
  // Leave explicitly (the engine would also leave once another project opens).
  await transport?.send(cmd("Collab", { type: "Leave" })).catch(() => undefined);
  p.settle(true);
  void p.action();
}
