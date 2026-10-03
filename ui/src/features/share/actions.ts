// Small helpers shared by the Share popover and the top-bar control.
import type { Command, ShareRole } from "@/generated";
import { isCommandFailed, type EngineTransport } from "@/transport";
import { shareToast } from "./toasts";

export const ROLE_LABEL: Record<ShareRole, string> = { Edit: "Can edit", Listen: "Can listen" };

export function errorText(e: unknown): string {
  if (isCommandFailed(e)) return e.error.message || e.error.code;
  return e instanceof Error ? e.message : String(e);
}

/** Send a command; a failure becomes a toast titled `failure`. Resolves whether it worked. */
export async function attempt(transport: EngineTransport, command: Command, failure: string): Promise<boolean> {
  try {
    await transport.send(command);
    return true;
  } catch (e) {
    shareToast(failure, errorText(e));
    return false;
  }
}
