/**
 * Invite links (docs/SHARING.md §4.1): the TypeScript mirror of
 * `crates/ether-collab/src/share/invite.rs` (same grammar, same test vectors).
 *
 *   https://app.ethereal.ws/join/<room>[?s=<signal url>]#<key>
 *   ethereal://join/<room>[?s=...]#<key>
 *
 * `room`: 22 base64url chars. `key`: a version char (`1`) + 22 base64url chars, in the
 * fragment so it never reaches a server. A link without a key is a bare room reference
 * (reserved for account-based joins; refused today).
 *
 * The UI only needs this to route (`/join/...` on web, the deep link on desktop) and to
 * reject a damaged link early; the engine parses the link again (`Share::OpenInvite`).
 */

export const ID_CHARS = 22;
export const KEY_V1 = "1";
export const DEEP_LINK_PREFIX = "ethereal://join/";

export interface Invite {
  room: string;
  /** Version char + secret, or `null` (no key). */
  key: string | null;
  /** Non-default signaling service (`?s=`). */
  signalUrl: string | null;
}

export type InviteError = "NotAnInvite" | "BadRoom" | "BadKey" | "UnknownKeyVersion";

export const INVITE_ERROR_TEXT: Record<InviteError, string> = {
  NotAnInvite: "This is not an Ethereal invite link.",
  BadRoom: "The invite link is damaged.",
  BadKey: "The invite link is damaged.",
  UnknownKeyVersion: "This invite needs a newer version of Ethereal.",
};

const B64URL = /^[A-Za-z0-9_-]*$/;

export const validId = (s: string): boolean => s.length === ID_CHARS && B64URL.test(s);

/** Parse a pasted or clicked link; an `InviteError` string when it is not a valid invite. */
export function parseInvite(link: string): Invite | InviteError {
  const trimmed = link.trim();
  let rest: string;
  if (trimmed.startsWith(DEEP_LINK_PREFIX)) {
    rest = trimmed.slice(DEEP_LINK_PREFIX.length);
  } else {
    const m = /^https?:\/\/[^/]+\/join\/(.*)$/s.exec(trimmed);
    if (!m) return "NotAnInvite";
    rest = m[1] ?? "";
  }
  const hash = rest.indexOf("#");
  const fragment = hash >= 0 ? rest.slice(hash + 1) : "";
  if (hash >= 0) rest = rest.slice(0, hash);
  const q = rest.indexOf("?");
  const query = q >= 0 ? rest.slice(q + 1) : "";
  const room = (q >= 0 ? rest.slice(0, q) : rest).replace(/\/+$/, "");
  if (!validId(room)) return "BadRoom";
  let signalUrl: string | null = null;
  for (const kv of query ? query.split("&") : []) {
    if (!kv.startsWith("s=")) continue;
    try {
      signalUrl = decodeURIComponent(kv.slice(2));
    } catch {
      return "NotAnInvite";
    }
    if (!/^https?:\/\//.test(signalUrl)) signalUrl = null;
    break;
  }
  let key: string | null = null;
  if (fragment) {
    if (fragment[0] !== KEY_V1) return "UnknownKeyVersion";
    if (!validId(fragment.slice(1))) return "BadKey";
    key = fragment;
  }
  return { room, key, signalUrl };
}

function tail(i: Invite): string {
  let s = i.room;
  if (i.signalUrl) s += `?s=${encodeURIComponent(i.signalUrl)}`;
  if (i.key) s += `#${i.key}`;
  return s;
}

/** The web link (`origin` like `https://etherealws.pages.dev`). */
export const webInviteUrl = (i: Invite, origin: string): string => `${origin.replace(/\/+$/, "")}/join/${tail(i)}`;

/** The desktop deep link. */
export const deepInviteLink = (i: Invite): string => `${DEEP_LINK_PREFIX}${tail(i)}`;
