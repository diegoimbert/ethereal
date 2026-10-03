// The UI side of the web share endpoint (docs/SHARING.md §6.2): `RTCPeerConnection` does not
// exist in Workers, so the controller Worker asks this agent, over the share port, to create
// one peer connection per pairing, and the data channel's bytes travel on the same port as
// transferred `ArrayBuffer`s (never through the JSON engine protocol). The Worker side
// (`ether_collab::share::web`) cuts frames into ≤ 16 KiB messages and reassembles them; this
// agent only moves messages. Messages: ./protocol.ts.
import type { IceCandidate, IceServer, StreamSignal } from "@/generated";
import { DC_LABEL, type FromUi, type ToUi } from "./protocol";

/** A pairing whose data channel is not open this long after `open` has failed (as native). */
export const CONNECT_TIMEOUT_MS = 30_000;
/** `bufferedAmountLowThreshold`: below it the Worker hears about the drained buffer. */
export const LOW_WATER_BYTES = 64 * 1024;
/** While the channel still buffers (below the threshold, no event), re-report this often. */
export const FLOW_POLL_MS = 50;

/** The UI end of the share `MessagePort` (a real port, or a test double). */
export interface SharePort {
  postMessage(message: FromUi, transfer?: Transferable[]): void;
  onmessage: ((e: MessageEvent<ToUi>) => void) | null;
}

export interface ShareEndpointOptions {
  /** Defaults to `new RTCPeerConnection(config)`. */
  createPeerConnection?: (config: RTCConfiguration) => RTCPeerConnection;
  connectTimeoutMs?: number;
}

/** `null` when this browser can create peer connections, else why not. */
export function webrtcUnavailableReason(): string | null {
  return typeof globalThis.RTCPeerConnection === "function" ? null : "this browser has no WebRTC";
}

/** The DTLS fingerprint of an SDP as the handshake proofs use it: `sha-256 AB:CD:...`. */
export function sdpFingerprint(sdp: string | undefined | null): string | null {
  const m = /^a=fingerprint:(\S+)\s+(\S+)/m.exec(sdp ?? "");
  return m ? `${m[1]!.toLowerCase()} ${m[2]!.toUpperCase()}` : null;
}

function rtcIceServer(s: IceServer): RTCIceServer {
  const out: RTCIceServer = { urls: s.urls };
  if (s.username != null) out.username = s.username;
  if (s.credential != null) out.credential = s.credential;
  return out;
}

const END_OF_CANDIDATES: IceCandidate = { candidate: "", sdp_mid: null, sdp_m_line_index: null, username_fragment: null };

function iceOf(c: RTCIceCandidate | RTCIceCandidateInit | null): IceCandidate {
  if (!c || !c.candidate) return END_OF_CANDIDATES;
  return {
    candidate: c.candidate,
    sdp_mid: c.sdpMid ?? null,
    sdp_m_line_index: c.sdpMLineIndex ?? null,
    username_fragment: c.usernameFragment ?? null,
  };
}

function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

interface Pairing {
  pc: RTCPeerConnection;
  dc: RTCDataChannel | null;
  opened: boolean;
  remoteSet: boolean;
  pendingIce: RTCIceCandidateInit[];
  /** Bytes handed to the data channel. */
  consumed: number;
  flowQueued: boolean;
  timer: ReturnType<typeof setTimeout> | null;
  /** Re-report `bufferedAmount` until it reaches 0. */
  flowTimer: ReturnType<typeof setTimeout> | null;
}

/**
 * Serve the share port: create and drive peer connections on the Worker's requests. Returns
 * a dispose function (closes every peer connection, detaches from the port).
 */
export function startShareEndpoint(port: SharePort, options: ShareEndpointOptions = {}): () => void {
  const create = options.createPeerConnection ?? ((config: RTCConfiguration) => new RTCPeerConnection(config));
  const timeoutMs = options.connectTimeoutMs ?? CONNECT_TIMEOUT_MS;
  const pairings = new Map<number, Pairing>();

  const post = (m: FromUi, transfer?: Transferable[]) => {
    if (transfer) port.postMessage(m, transfer);
    else port.postMessage(m);
  };

  const teardown = (ch: number): boolean => {
    const p = pairings.get(ch);
    if (!p) return false;
    pairings.delete(ch);
    if (p.timer) clearTimeout(p.timer);
    if (p.flowTimer) clearTimeout(p.flowTimer);
    if (p.dc) {
      p.dc.onopen = p.dc.onmessage = p.dc.onclose = p.dc.onbufferedamountlow = null;
      p.dc.close();
    }
    p.pc.onicecandidate = p.pc.onconnectionstatechange = p.pc.ondatachannel = null;
    p.pc.close();
    return true;
  };

  /** End a pairing on our side and tell the Worker. */
  const end = (ch: number, reason: string) => {
    if (teardown(ch)) post({ type: "closed", ch, reason });
  };

  const flow = (ch: number, p: Pairing) => {
    if (p.flowQueued) return;
    p.flowQueued = true;
    queueMicrotask(() => {
      p.flowQueued = false;
      if (pairings.get(ch) !== p) return;
      const buffered = p.dc?.bufferedAmount ?? 0;
      post({ type: "flow", ch, consumed: p.consumed, buffered });
      if (buffered > 0 && !p.flowTimer) {
        p.flowTimer = setTimeout(() => {
          p.flowTimer = null;
          flow(ch, p);
        }, FLOW_POLL_MS);
      }
    });
  };

  const wire = (ch: number, p: Pairing, dc: RTCDataChannel) => {
    p.dc = dc;
    dc.binaryType = "arraybuffer";
    dc.bufferedAmountLowThreshold = LOW_WATER_BYTES;
    dc.onopen = () => {
      if (pairings.get(ch) !== p || p.opened) return;
      const local = sdpFingerprint(p.pc.localDescription?.sdp);
      const remote = sdpFingerprint(p.pc.remoteDescription?.sdp);
      if (!local || !remote) return end(ch, "no DTLS fingerprint in the session description");
      p.opened = true;
      if (p.timer) clearTimeout(p.timer);
      p.timer = null;
      post({ type: "open", ch, local, remote });
    };
    dc.onmessage = (e: MessageEvent<unknown>) => {
      // Every fragment is a binary message (`share::dc`); anything else is not ours.
      if (e.data instanceof ArrayBuffer) post({ type: "data", ch, data: e.data }, [e.data]);
    };
    dc.onbufferedamountlow = () => flow(ch, p);
    dc.onclose = () => end(ch, p.opened ? "the peer closed the data channel" : "the data channel closed before opening");
  };

  const signal = (ch: number, s: StreamSignal) => post({ type: "signal", ch, signal: s });

  const addIce = (p: Pairing, c: RTCIceCandidateInit) => {
    // Unusable candidates (another address family, mDNS) are expected: ICE uses the others.
    p.pc.addIceCandidate(c).catch(() => undefined);
  };

  const remoteSet = (p: Pairing) => {
    p.remoteSet = true;
    for (const c of p.pendingIce.splice(0)) addIce(p, c);
  };

  const open = (ch: number, offer: boolean, ice: IceServer[], relay: boolean) => {
    teardown(ch);
    const unavailable = options.createPeerConnection ? null : webrtcUnavailableReason();
    if (unavailable) return post({ type: "closed", ch, reason: unavailable });
    let pc: RTCPeerConnection;
    try {
      pc = create({ iceServers: ice.map(rtcIceServer), iceTransportPolicy: relay ? "relay" : "all" });
    } catch (e) {
      return post({ type: "closed", ch, reason: `could not create a peer connection: ${errorText(e)}` });
    }
    const p: Pairing = {
      pc,
      dc: null,
      opened: false,
      remoteSet: false,
      pendingIce: [],
      consumed: 0,
      flowQueued: false,
      timer: null,
      flowTimer: null,
    };
    pairings.set(ch, p);
    p.timer = setTimeout(() => {
      if (!p.opened) end(ch, "could not connect to the peer (ICE/DTLS timed out)");
    }, timeoutMs);
    pc.onicecandidate = (e) => signal(ch, { type: "Ice", candidate: iceOf(e.candidate) });
    pc.onconnectionstatechange = () => {
      if (pc.connectionState === "failed") end(ch, p.opened ? "the connection to the peer was lost" : "could not connect to the peer (ICE failed)");
      else if (pc.connectionState === "closed") end(ch, "connection closed");
    };
    if (offer) {
      wire(ch, p, pc.createDataChannel(DC_LABEL, { ordered: true }));
      void (async () => {
        try {
          await pc.setLocalDescription(await pc.createOffer());
          if (pairings.get(ch) === p) signal(ch, { type: "Offer", sdp: pc.localDescription?.sdp ?? "" });
        } catch (e) {
          end(ch, `could not create the offer: ${errorText(e)}`);
        }
      })();
    } else {
      pc.ondatachannel = (e) => {
        if (e.channel.label === DC_LABEL && !p.dc) wire(ch, p, e.channel);
      };
    }
  };

  const remoteSignal = (ch: number, s: StreamSignal) => {
    const p = pairings.get(ch);
    if (!p) return;
    switch (s.type) {
      case "Offer":
        if (p.remoteSet || p.dc) return; // the joiner offers once; no renegotiation
        void (async () => {
          try {
            await p.pc.setRemoteDescription({ type: "offer", sdp: s.sdp });
            remoteSet(p);
            await p.pc.setLocalDescription(await p.pc.createAnswer());
            if (pairings.get(ch) === p) signal(ch, { type: "Answer", sdp: p.pc.localDescription?.sdp ?? "" });
          } catch (e) {
            end(ch, `bad offer: ${errorText(e)}`);
          }
        })();
        break;
      case "Answer":
        if (p.remoteSet) return;
        p.pc.setRemoteDescription({ type: "answer", sdp: s.sdp }).then(
          () => remoteSet(p),
          (e: unknown) => end(ch, `bad answer: ${errorText(e)}`),
        );
        break;
      case "Ice": {
        const c = s.candidate;
        if (!c.candidate.trim()) return; // end of candidates
        const init: RTCIceCandidateInit = {
          candidate: c.candidate.replace(/^a=/, ""),
          sdpMid: c.sdp_mid,
          sdpMLineIndex: c.sdp_m_line_index ?? (c.sdp_mid == null ? 0 : null),
          usernameFragment: c.username_fragment,
        };
        if (p.remoteSet) addIce(p, init);
        else if (p.pendingIce.length < 64) p.pendingIce.push(init);
        break;
      }
      case "Bye":
        end(ch, s.reason ?? "the peer ended the connection");
        break;
    }
  };

  port.onmessage = (e: MessageEvent<ToUi>) => {
    const m = e.data;
    switch (m.type) {
      case "open":
        open(m.ch, m.offer, m.ice, m.relay);
        break;
      case "signal":
        remoteSignal(m.ch, m.signal);
        break;
      case "data": {
        const p = pairings.get(m.ch);
        if (!p?.dc || p.dc.readyState !== "open") return;
        try {
          p.dc.send(m.data);
        } catch (err) {
          return end(m.ch, `data channel send: ${errorText(err)}`);
        }
        p.consumed += m.data.byteLength;
        flow(m.ch, p);
        break;
      }
      case "close":
        teardown(m.ch);
        break;
    }
  };

  return () => {
    port.onmessage = null;
    for (const ch of [...pairings.keys()]) teardown(ch);
  };
}
