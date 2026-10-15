import { describe, expect, it } from "vitest";
import { deepInviteLink, parseInvite, webInviteUrl, type Invite } from "./invite";

// Same vectors as crates/ether-collab/src/share/invite.rs.
const ROOM = "AbCdEfGhIjKlMnOpQrStUv";
const SECRET = "0123456789_-abcdefghij";

describe("invite links", () => {
  it("round-trips web and deep links", () => {
    const inv: Invite = { room: ROOM, key: `1${SECRET}`, signalUrl: null };
    const web = webInviteUrl(inv, "https://etherealws.pages.dev/");
    expect(web).toBe(`https://etherealws.pages.dev/join/${ROOM}#1${SECRET}`);
    expect(parseInvite(web)).toEqual(inv);
    const deep = deepInviteLink(inv);
    expect(deep).toBe(`ethereal://join/${ROOM}#1${SECRET}`);
    expect(parseInvite(`  ${deep}\n`)).toEqual(inv);

    const custom = { ...inv, signalUrl: "https://signal.example.org/v?x=1" };
    const url = webInviteUrl(custom, "http://localhost:5173");
    expect(url).toContain("?s=https%3A%2F%2Fsignal.example.org%2Fv%3Fx%3D1#1");
    expect(parseInvite(url)).toEqual(custom);
  });

  it("treats keyless links as room references", () => {
    expect(parseInvite(`https://x.test/join/${ROOM}/`)).toEqual({ room: ROOM, key: null, signalUrl: null });
  });

  it("refuses damaged links", () => {
    expect(parseInvite("https://x.test/song")).toBe("NotAnInvite");
    expect(parseInvite("ftp://x.test/join/a")).toBe("NotAnInvite");
    expect(parseInvite("https://x.test/join/short#1abc")).toBe("BadRoom");
    expect(parseInvite(`https://x.test/join/${ROOM}#1short`)).toBe("BadKey");
    expect(parseInvite(`https://x.test/join/${ROOM}#2${SECRET}`)).toBe("UnknownKeyVersion");
  });
});
