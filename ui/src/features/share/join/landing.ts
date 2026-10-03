// The web `/join/` route (docs/SHARING.md §5): decided before the engine boots.
import { deepLinkForTail, joinTail, linkProblem } from "./links";

/** "Always continue in browser" (a per-browser preference). */
export const PREFER_BROWSER_KEY = "eth.join.preferBrowser";
/** Where "Download it" points. */
export const DOWNLOAD_URL = "https://github.com/diegoimbert/ethereal/releases/latest";
/** "Open in the app": after this long with the page still visible, offer the download. */
export const APP_OPEN_TIMEOUT_MS = 1500;

export interface JoinRoute {
  /** The invite as opened (`https://…/join/<room>[?s=…]#<key>`), sent with `OpenInvite`. */
  link: string;
  /** The same invite as a desktop deep link. */
  deepLink: string;
  /** Why the link can't be joined, or `null`. */
  problem: string | null;
}

export function joinRoute(loc: Pick<Location, "pathname" | "search" | "hash" | "href">): JoinRoute | null {
  const tail = joinTail(loc);
  if (tail === null) return null;
  return { link: loc.href, deepLink: deepLinkForTail(tail), problem: linkProblem(loc.href) };
}

/** Phones and tablets go straight to the browser (there is no mobile app). */
export function isMobile(nav: Pick<Navigator, "userAgent"> & { maxTouchPoints?: number } = navigator): boolean {
  return /Android|iPhone|iPad|iPod|Mobile/i.test(nav.userAgent) || (/Macintosh/.test(nav.userAgent) && (nav.maxTouchPoints ?? 0) > 1);
}

export function prefersBrowser(): boolean {
  try {
    return localStorage.getItem(PREFER_BROWSER_KEY) === "1";
  } catch {
    return false;
  }
}

export function setPrefersBrowser(on: boolean): void {
  try {
    if (on) localStorage.setItem(PREFER_BROWSER_KEY, "1");
    else localStorage.removeItem(PREFER_BROWSER_KEY);
  } catch {
    // not remembered
  }
}
