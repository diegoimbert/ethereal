// SDP munging of the web sender's offer (docs/COLLAB.md §9.1, Opus settings): stereo,
// 128 kbit/s target, in-band FEC, DTX off.

/** Opus `fmtp` parameters set on the offer (others are kept). */
export const OPUS_FMTP: Readonly<Record<string, string>> = {
  stereo: "1",
  "sprop-stereo": "1",
  maxaveragebitrate: "128000",
  useinbandfec: "1",
  usedtx: "0",
};

/** Set the Opus `fmtp` parameters of every Opus payload type in `sdp`. */
export function mungeOpus(sdp: string): string {
  const eol = sdp.includes("\r\n") ? "\r\n" : "\n";
  const lines = sdp.split(eol);
  const opus = new Set<string>();
  for (const line of lines) {
    const m = /^a=rtpmap:(\d+) opus\/48000(\/\d+)?$/i.exec(line);
    if (m) opus.add(m[1]!);
  }
  if (opus.size === 0) return sdp;
  const out: string[] = [];
  for (const line of lines) {
    const fmtp = /^a=fmtp:(\d+) (.*)$/.exec(line);
    if (fmtp && opus.has(fmtp[1]!)) {
      out.push(`a=fmtp:${fmtp[1]} ${mergeParams(fmtp[2]!)}`);
      continue;
    }
    out.push(line);
    const rtpmap = /^a=rtpmap:(\d+) /.exec(line);
    // An Opus payload without an fmtp line gets one right after its rtpmap.
    if (rtpmap && opus.has(rtpmap[1]!) && !lines.some((l) => l.startsWith(`a=fmtp:${rtpmap[1]} `))) {
      out.push(`a=fmtp:${rtpmap[1]} ${mergeParams("")}`);
    }
  }
  return out.join(eol);
}

function mergeParams(params: string): string {
  const kept: [string, string][] = [];
  for (const part of params.split(";")) {
    const p = part.trim();
    if (!p) continue;
    const eq = p.indexOf("=");
    const key = eq < 0 ? p : p.slice(0, eq);
    if (key in OPUS_FMTP) continue;
    kept.push([key, eq < 0 ? "" : p.slice(eq + 1)]);
  }
  const all = [...kept, ...Object.entries(OPUS_FMTP)];
  return all.map(([k, v]) => (v === "" ? k : `${k}=${v}`)).join(";");
}
