import { afterEach, describe, expect, it } from "vitest";
import { parseInvite } from "@/domain/invite";
import { isMobile, joinRoute, prefersBrowser, PREFER_BROWSER_KEY, setPrefersBrowser } from "./landing";
import { deepLinkForTail, inviteLinkFrom, joinTail } from "./links";

const ROOM = "AbCdEfGhIjKlMnOpQrStUv";
const KEY = "10123456789_-abcdefghij";

function loc(href: string) {
  const u = new URL(href);
  return { href, pathname: u.pathname, search: u.search, hash: u.hash };
}

describe("deep-link routing", () => {
  it("routes ethereal://join links (any scheme case) and web invites", () => {
    expect(inviteLinkFrom(`ethereal://join/${ROOM}#${KEY}`)).toBe(`ethereal://join/${ROOM}#${KEY}`);
    expect(inviteLinkFrom(`  Ethereal://JOIN/${ROOM}#${KEY}\n`)).toBe(`ethereal://join/${ROOM}#${KEY}`);
    expect(inviteLinkFrom(`https://etherealws.pages.dev/join/${ROOM}#${KEY}`)).toBe(`https://etherealws.pages.dev/join/${ROOM}#${KEY}`);
    // Damaged invites still reach the join screen (which says what is wrong).
    expect(inviteLinkFrom("ethereal://join/short")).toBe("ethereal://join/short");
  });

  it("ignores links that are not invites", () => {
    expect(inviteLinkFrom("ethereal://open/project")).toBeNull();
    expect(inviteLinkFrom("ethereal:join")).toBeNull();
    expect(inviteLinkFrom("https://example.org/song")).toBeNull();
    expect(inviteLinkFrom("file:///join/x")).toBeNull();
    expect(inviteLinkFrom("")).toBeNull();
  });

  it("the routed deep link parses to the same invite as the web link", () => {
    const deep = inviteLinkFrom(`ETHEREAL://join/${ROOM}?s=https%3A%2F%2Fsig.example#${KEY}`)!;
    expect(parseInvite(deep)).toEqual(parseInvite(`https://x.test/join/${ROOM}?s=https%3A%2F%2Fsig.example#${KEY}`));
  });
});

describe("web /join route", () => {
  afterEach(() => localStorage.clear());

  it("takes the tail after /join/ and builds the deep link from it", () => {
    const l = loc(`https://etherealws.pages.dev/join/${ROOM}?s=https%3A%2F%2Fsig.example#${KEY}`);
    expect(joinTail(l)).toBe(`${ROOM}?s=https%3A%2F%2Fsig.example#${KEY}`);
    expect(deepLinkForTail(joinTail(l)!)).toBe(`ethereal://join/${ROOM}?s=https%3A%2F%2Fsig.example#${KEY}`);
    expect(joinTail(loc("https://etherealws.pages.dev/"))).toBeNull();
    expect(joinTail(loc("https://etherealws.pages.dev/joinx/a"))).toBeNull();
  });

  it("describes valid and damaged invites", () => {
    expect(joinRoute(loc(`https://etherealws.pages.dev/join/${ROOM}#${KEY}`))).toEqual({
      link: `https://etherealws.pages.dev/join/${ROOM}#${KEY}`,
      deepLink: `ethereal://join/${ROOM}#${KEY}`,
      problem: null,
    });
    expect(joinRoute(loc(`https://h.test/join/${ROOM}#1short`))?.problem).toBe("The invite link is damaged.");
    expect(joinRoute(loc(`https://h.test/join/${ROOM}#2${KEY.slice(1)}`))?.problem).toBe("This invite needs a newer version of Ethereal.");
    expect(joinRoute(loc(`https://h.test/join/${ROOM}`))?.problem).toMatch(/no invite key/);
    expect(joinRoute(loc("https://h.test/project"))).toBeNull();
  });

  it("remembers 'Always continue in browser'", () => {
    expect(prefersBrowser()).toBe(false);
    setPrefersBrowser(true);
    expect(localStorage.getItem(PREFER_BROWSER_KEY)).toBe("1");
    expect(prefersBrowser()).toBe(true);
    setPrefersBrowser(false);
    expect(prefersBrowser()).toBe(false);
  });

  it("sends phones straight to the browser", () => {
    expect(isMobile({ userAgent: "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) Mobile/15E148" })).toBe(true);
    expect(isMobile({ userAgent: "Mozilla/5.0 (Linux; Android 15; Pixel 9) Chrome/140 Mobile Safari/537.36" })).toBe(true);
    expect(isMobile({ userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Safari/605", maxTouchPoints: 5 })).toBe(true);
    expect(isMobile({ userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Safari/605", maxTouchPoints: 0 })).toBe(false);
    expect(isMobile({ userAgent: "Mozilla/5.0 (X11; Linux x86_64) Chrome/140 Safari/537.36" })).toBe(false);
  });
});
