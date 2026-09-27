/** Opus SDP munging of the web sender's offer (docs/COLLAB.md §9.1). */
import { describe, expect, it } from "vitest";
import { mungeOpus } from "./sdp";

const OFFER = [
  "v=0",
  "o=- 1 2 IN IP4 127.0.0.1",
  "s=-",
  "t=0 0",
  "m=audio 9 UDP/TLS/RTP/SAVPF 111 63 9",
  "a=rtpmap:111 opus/48000/2",
  "a=rtcp-fb:111 transport-cc",
  "a=fmtp:111 minptime=10;useinbandfec=0;usedtx=1",
  "a=rtpmap:63 red/48000/2",
  "a=fmtp:63 111/111",
  "a=rtpmap:9 G722/8000",
  "",
].join("\r\n");

describe("mungeOpus", () => {
  it("sets stereo, bitrate, FEC and DTX off on the Opus fmtp, keeping other params and lines", () => {
    const out = mungeOpus(OFFER);
    expect(out).toContain("a=fmtp:111 minptime=10;stereo=1;sprop-stereo=1;maxaveragebitrate=128000;useinbandfec=1;usedtx=0\r\n");
    // RED and the rest are untouched; CRLF line endings preserved.
    expect(out).toContain("a=fmtp:63 111/111\r\n");
    expect(out.split("\r\n")).toHaveLength(OFFER.split("\r\n").length);
    expect(mungeOpus(out)).toBe(out);
  });

  it("adds an fmtp line when the offer has none", () => {
    const sdp = "m=audio 9 RTP/AVP 109\na=rtpmap:109 OPUS/48000/2\na=sendrecv\n";
    const out = mungeOpus(sdp).split("\n");
    expect(out[2]).toBe("a=fmtp:109 stereo=1;sprop-stereo=1;maxaveragebitrate=128000;useinbandfec=1;usedtx=0");
    expect(out[3]).toBe("a=sendrecv");
  });

  it("leaves an SDP without Opus alone", () => {
    const sdp = "m=audio 9 RTP/AVP 0\na=rtpmap:0 PCMU/8000\n";
    expect(mungeOpus(sdp)).toBe(sdp);
  });
});
