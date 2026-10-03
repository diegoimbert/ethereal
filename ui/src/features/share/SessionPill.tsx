import type { ButtonHTMLAttributes } from "react";
import { AvatarStack } from "./PeerAvatar";
import type { SessionStatus } from "./status";

export interface SessionPillProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  status: SessionStatus;
  people: ReadonlyArray<{ name: string; color: number }>;
}

/** The one session element of the top bar: avatars, a status dot and a label. */
export function SessionPill({ status, people, ...rest }: SessionPillProps) {
  return (
    <button
      type="button"
      className="eth-share-pill"
      data-testid="session-pill"
      data-tone={status.tone}
      title={status.detail}
      aria-label={`Session: ${status.label}. ${status.detail}`}
      {...rest}
    >
      <AvatarStack people={people} />
      <span className="eth-share-pill__dot" aria-hidden />
      <span className="eth-share-pill__label">{status.label}</span>
    </button>
  );
}
