// Join screen texts (docs/SHARING.md §8.3).
import type { JoinFailure } from "@/generated";

/** The host's name isn't known when joining fails. */
export const FAILURE_TEXT: Record<JoinFailure, string> = {
  BadLink: "The invite link is damaged.",
  InvalidInvite: "This invite link was reset or is no longer valid. Ask the host for a new one.",
  Refused: "The host's session is full. Try again later.",
  Version: "Update Ethereal to join this project.",
  Unreachable: "Couldn't reach the host's computer (network). Try again, or ask the host to enable a relay server.",
  HostNotVerified: "The host's identity could not be verified.",
  Network: "Can't reach the sharing service. Check your connection and try again.",
};

/** What the join screen says for a failure: the engine's own text for a damaged link. */
export function failureText(reason: JoinFailure, message: string): string {
  return reason === "BadLink" && message.trim() ? message : FAILURE_TEXT[reason];
}
