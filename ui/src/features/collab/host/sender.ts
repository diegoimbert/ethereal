// The web sender (docs/COLLAB.md §9.1 "Senders → Web", §9.2, §9.4): one RTCPeerConnection
// per `ListenerLink { endpoint: "Ui" }`, carrying the engine's stream tap. The host always
// offers; signals go through the controller (`SendSignal`, `CollabEvent::Signal`), anchors
// through `SendStreamClock` every 100 ms and at each discontinuity, once media flows.
//
// Framework-free and dependency-injected (peer connection factory, encoded-frame observer,
// tap clock, command sink) so the tests drive it with fakes.

import type { CollabCommand, IceCandidate, IceServer, ListenerLink, StreamSignal } from "@/generated";
import { anchor, CountInTracker, RtpOffsetEstimator, STREAM_CLOCK_INTERVAL_MS, type HostTransport } from "./anchors";
import type { RtpObserver } from "./observer";
import { mungeOpus } from "./sdp";
import type { TapClock, TapState } from "./tapClock";

/** Without `connected` by then, the stream fails (`Bye`). */
export const CONNECT_TIMEOUT_MS = 15_000;
/** How often anchors are checked (jumps are sent at the next check). */
export const TICK_MS = 20;

export type LinkState = "connecting" | "connected";

/** What the sender needs to know about the engine's timeline. */
export interface TimelineSource {
  sampleRate: number;
  /** The worklet's tap clock (`null` before the first tapped block). */
  read(): TapClock | null;
  /** Context frame at which an encoded frame observed at `perfMs` starts (`null`: unknown yet). */
  frameStart(perfMs: number): number | null;
  /** The host transport state listeners mirror, and the count-in pre-roll (beats, 0 = none). */
  transport(): HostTransport & { preRoll: number };
}

export interface SenderDeps {
  send(command: CollabCommand): Promise<unknown>;
  createPeer(config: RTCConfiguration): RTCPeerConnection;
  /** The tap stream (one stereo track). */
  stream: MediaStream;
  observer: RtpObserver;
  timeline: TimelineSource;
  /** Link states changed (for the listener list). */
  onChange?(states: ReadonlyMap<string, LinkState>): void;
  /** `performance.now()` (tests override it). */
  now?(): number;
}

interface Peer {
  key: string;
  site: string;
  stream: number;
  pc: RTCPeerConnection;
  state: LinkState;
  estimator: RtpOffsetEstimator;
  /** Remote ICE received before the answer. */
  pendingIce: IceCandidate[];
  answered: boolean;
  /** An anchor was sent (the first one is a discontinuity). */
  started: boolean;
  lastClockAt: number;
  /** A discontinuity not sent yet. */
  jump: TapState | null;
  detach: () => void;
  timeout: ReturnType<typeof setTimeout>;
}

export const linkKey = (site: string, stream: number) => `${site}:${stream}`;

function iceInit(c: IceCandidate): RTCIceCandidateInit {
  return {
    candidate: c.candidate,
    sdpMid: c.sdp_mid,
    sdpMLineIndex: c.sdp_m_line_index,
    usernameFragment: c.username_fragment,
  };
}

function iceServers(servers: IceServer[]): RTCIceServer[] {
  return servers.map((s) => ({
    urls: s.urls,
    ...(s.username !== null ? { username: s.username } : {}),
    ...(s.credential !== null ? { credential: s.credential } : {}),
  }));
}

export class WebSender {
  private readonly peers = new Map<string, Peer>();
  /** Links we ended (Bye sent) that the controller still lists: never reopened. */
  private readonly ended = new Set<string>();
  private servers: IceServer[] = [];
  private readonly countIn = new CountInTracker();
  private events = 0;
  private countInEnd: number | null = null;
  private timer: ReturnType<typeof setInterval> | null = null;
  private disposed = false;
  private readonly now: () => number;

  constructor(private readonly deps: SenderDeps) {
    this.now = deps.now ?? (() => performance.now());
    this.timer = setInterval(() => this.tick(), TICK_MS);
  }

  setIceServers(servers: IceServer[]): void {
    this.servers = servers;
  }

  /** The controller's current listeners (`ListenStatus.listeners`). */
  setListeners(links: ListenerLink[]): void {
    if (this.disposed) return;
    const wanted = new Map(links.filter((l) => l.endpoint === "Ui").map((l) => [linkKey(l.site, l.stream), l]));
    for (const key of [...this.ended]) if (!wanted.has(key)) this.ended.delete(key);
    for (const [key, peer] of [...this.peers]) if (!wanted.has(key)) this.close(peer);
    for (const [key, link] of wanted) if (!this.peers.has(key) && !this.ended.has(key)) this.open(link);
    this.changed();
  }

  /** `CollabEvent::Signal` from a listener. */
  onSignal(from: string, stream: number, signal: StreamSignal): void {
    const peer = this.peers.get(linkKey(from, stream));
    if (!peer) return; // stale or not ours
    switch (signal.type) {
      case "Answer":
        if (peer.answered) return;
        peer.answered = true;
        void peer.pc
          .setRemoteDescription({ type: "answer", sdp: signal.sdp })
          .then(() => {
            for (const c of peer.pendingIce.splice(0)) this.addIce(peer, c);
          })
          .catch((e: unknown) => this.fail(peer, `bad answer: ${String(e)}`));
        break;
      case "Ice":
        if (peer.answered && peer.pc.remoteDescription) this.addIce(peer, signal.candidate);
        else peer.pendingIce.push(signal.candidate);
        break;
      case "Bye":
        this.close(peer);
        this.ended.add(peer.key);
        this.changed();
        break;
      case "Offer":
        break; // the host always offers
    }
  }

  /** Current link states (by `linkKey`). */
  states(): ReadonlyMap<string, LinkState> {
    return new Map([...this.peers].map(([k, p]) => [k, p.state]));
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    if (this.timer !== null) clearInterval(this.timer);
    for (const peer of [...this.peers.values()]) this.close(peer);
    this.deps.observer.dispose();
  }

  // ─── anchors ─────────────────────────────────────────────────────────────────────────

  /** Check the tap clock and send due anchors (also run on every observed frame). */
  tick(): void {
    if (this.disposed) return;
    const clock = this.deps.timeline.read();
    if (!clock) return;
    const transport = this.deps.timeline.transport();
    if (clock.events !== this.events) {
      this.events = clock.events;
      this.countInEnd = this.countIn.update(clock.event, transport.preRoll);
      for (const peer of this.peers.values()) peer.jump = clock.event;
    }
    this.countInEnd = this.countIn.update(clock.latest, transport.preRoll);
    const now = this.now();
    for (const peer of this.peers.values()) this.sendDue(peer, clock, transport, now);
  }

  private sendDue(peer: Peer, clock: TapClock, transport: HostTransport, now: number): void {
    if (peer.state !== "connected") return;
    let state: TapState;
    let discontinuity: boolean;
    if (!peer.started) {
      [state, discontinuity] = [clock.latest, true];
    } else if (peer.jump) {
      [state, discontinuity] = [peer.jump, true];
    } else if (now - peer.lastClockAt >= STREAM_CLOCK_INTERVAL_MS) {
      [state, discontinuity] = [clock.latest, false];
    } else {
      return;
    }
    const rtp = peer.estimator.rtpAt(state.tapFrame);
    if (rtp === null) return; // no encoded frame observed yet
    peer.started = true;
    peer.jump = null;
    peer.lastClockAt = now;
    const countInEnd = state.recording && state.playing ? this.countInEnd : null;
    this.post({
      type: "SendStreamClock",
      to: peer.site,
      stream: peer.stream,
      clock: anchor(state, rtp, transport, discontinuity, countInEnd),
    });
  }

  private observe(peer: Peer, rtp: number, perfMs: number): void {
    const frame = this.deps.timeline.frameStart(perfMs);
    if (frame === null) return;
    peer.estimator.observe(rtp, frame);
    this.tick();
  }

  // ─── peer connections ────────────────────────────────────────────────────────────────

  private open(link: ListenerLink): void {
    const key = linkKey(link.site, link.stream);
    const pc = this.deps.createPeer({
      iceServers: iceServers(this.servers),
      ...(this.deps.observer.peerConfig as RTCConfiguration),
    });
    const peer: Peer = {
      key,
      site: link.site,
      stream: link.stream,
      pc,
      state: "connecting",
      estimator: new RtpOffsetEstimator(this.deps.timeline.sampleRate),
      pendingIce: [],
      answered: false,
      started: false,
      lastClockAt: 0,
      jump: null,
      detach: () => undefined,
      timeout: setTimeout(() => this.fail(peer, "no connection to the listener"), CONNECT_TIMEOUT_MS),
    };
    this.peers.set(key, peer);
    const [track] = this.deps.stream.getAudioTracks();
    if (track) {
      track.contentHint = "music";
      const sender = pc.addTrack(track, this.deps.stream);
      peer.detach = this.deps.observer.attach(sender, (rtp, at) => this.observe(peer, rtp, at));
    }
    pc.onicecandidate = (e) => {
      const c = e.candidate;
      const candidate: IceCandidate = c
        ? { candidate: c.candidate, sdp_mid: c.sdpMid, sdp_m_line_index: c.sdpMLineIndex, username_fragment: c.usernameFragment }
        : { candidate: "", sdp_mid: null, sdp_m_line_index: null, username_fragment: null };
      this.signal(peer, { type: "Ice", candidate });
    };
    pc.onconnectionstatechange = () => {
      if (this.peers.get(key) !== peer) return;
      if (pc.connectionState === "connected" && peer.state !== "connected") {
        clearTimeout(peer.timeout);
        peer.state = "connected";
        this.changed();
        this.tick();
      } else if (pc.connectionState === "failed") {
        this.fail(peer, "connection failed");
      }
    };
    void (async () => {
      const offer = await pc.createOffer();
      const sdp = mungeOpus(offer.sdp ?? "");
      await pc.setLocalDescription({ type: "offer", sdp });
      if (this.peers.get(key) === peer) this.signal(peer, { type: "Offer", sdp });
    })().catch((e: unknown) => this.fail(peer, `could not create the offer: ${String(e)}`));
  }

  private addIce(peer: Peer, c: IceCandidate): void {
    void peer.pc.addIceCandidate(iceInit(c)).catch(() => undefined);
  }

  /** Our side ends the stream: tell the listener (through the controller) and close. */
  private fail(peer: Peer, reason: string): void {
    if (this.peers.get(peer.key) !== peer) return;
    this.signal(peer, { type: "Bye", reason });
    this.close(peer);
    this.ended.add(peer.key);
    this.changed();
  }

  private close(peer: Peer): void {
    if (this.peers.get(peer.key) === peer) this.peers.delete(peer.key);
    clearTimeout(peer.timeout);
    peer.detach();
    peer.pc.onicecandidate = null;
    peer.pc.onconnectionstatechange = null;
    peer.pc.close();
  }

  private signal(peer: Peer, signal: StreamSignal): void {
    this.post({ type: "SendSignal", to: peer.site, stream: peer.stream, signal });
  }

  private post(command: CollabCommand): void {
    this.deps.send(command).catch(() => undefined);
  }

  private changed(): void {
    this.deps.onChange?.(this.states());
  }
}
