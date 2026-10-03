/**
 * Which links open the join screen (docs/SHARING.md §5). Damaged invites are routed too, so
 * the join screen can say what is wrong with them; anything else is ignored.
 *
 * - `ethereal://join/...` (a desktop deep link; the scheme is case-insensitive);
 * - `http(s)://<any host>/join/...` (a web invite pasted into the app).
 *
 * The engine parses the link again (`Share::OpenInvite`).
 */
import { DEEP_LINK_PREFIX, INVITE_ERROR_TEXT, parseInvite } from "@/domain/invite";

const DEEP = /^ethereal:\/\/join\//i;
const WEB = /^https?:\/\/[^/\s]+\/join\//i;

/** The link to send with `OpenInvite`, or `null` when `url` is not an invite link. */
export function inviteLinkFrom(url: string): string | null {
  const s = url.trim();
  if (DEEP.test(s)) return DEEP_LINK_PREFIX + s.replace(DEEP, "");
  if (WEB.test(s)) return s;
  return null;
}

/** The web `/join/` route: the part after `/join/` (path, query, fragment), or `null`. */
export function joinTail(loc: { pathname: string; search: string; hash: string }): string | null {
  if (!loc.pathname.startsWith("/join/")) return null;
  return loc.pathname.slice("/join/".length) + loc.search + loc.hash;
}

/** The desktop deep link for a web `/join/` tail ("Open in the app"). */
export const deepLinkForTail = (tail: string): string => DEEP_LINK_PREFIX + tail;

/** Why `link` can't be joined, or `null` when it looks like a valid invite. */
export function linkProblem(link: string): string | null {
  const inv = parseInvite(link);
  if (typeof inv === "string") return INVITE_ERROR_TEXT[inv];
  if (!inv.key) return "This link has no invite key. Ask the host for the full link.";
  return null;
}
