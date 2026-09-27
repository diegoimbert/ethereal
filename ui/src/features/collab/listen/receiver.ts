// The listener's WebRTC receiver (docs/COLLAB.md §9.1-§9.2): one receive-only
// `RTCPeerConnection` per stream, the host is the offerer. Signals travel through the
// controller (`SendSignal` / `CollabEvent::Signal`); audio plays through an `AudioSink`
// created in the "Listen" click (autoplay policies need the gesture).
import type { IceCandidate, IceServer, SiteId, StreamSignal } from "@/generated";
import { playoutRtp } from "./clock";

/** The receiver gives up (and says `Bye`) after this long without media. */
export const NO_MEDIA_TIMEOUT_MS = 10_000;
/** How often stats are read (media watchdog, fallback delay estimate). */
export const STATS_INTERVAL_MS = 1_000;

/** `null` when this build can receive a stream, else why not (shown on disabled entries). */
export function webrtcUnsupportedReason(): string | null {
  return typeof globalThis.RTCPeerConnection === "function" ? null : "this webview has no WebRTC";
}

/** Run a media call that may throw or reject (autoplay refusals, unimplemented APIs). */
function quiet(f: () => unknown): void {
  try {
    const r = f();
    if (r instanceof Promise) r.catch(() => undefined);
  } catch {
    // nothing to do: the stream still arrives, the watchdog reports a dead one
  }
}

/** Where received audio plays. */
export interface AudioSink {
  play(stream: MediaStream): void;
  /** Seconds between a sample leaving the jitter buffer and the speakers (0 if unknown). */
  outputLatency(): number;
  close(): void;
}

/**
 * An `AudioSink` on an `AudioContext` + `<audio>` element. Call it synchronously in the
 * click handler: creating/resuming the context there is what autoplay policies allow.
 */
export function createAudioSink(): AudioSink {
  let ctx: AudioContext | null = null;
  try {
    ctx = new AudioContext({ latencyHint: "playback" });
    quiet(() => ctx!.resume());
  } catch {
    ctx = null;
  }
  const el = typeof Audio === "function" ? new Audio() : null;
  let source: MediaStreamAudioSourceNode | null = null;
  return {
    play(stream) {
      if (el) {
        el.autoplay = true;
        el.srcObject = stream;
        // Chrome only pulls remote WebRTC audio into WebAudio when an element plays it too.
        if (ctx) el.muted = true;
        quiet(() => el.play());
      }
      if (ctx) {
        source?.disconnect();
        source = ctx.createMediaStreamSource(stream);
        source.connect(ctx.destination);
      }
    },
    outputLatency() {
      return ctx ? ctx.outputLatency || ctx.baseLatency || 0 : 0;
    },
    close() {
      source?.disconnect();
      if (el) {
        quiet(() => el.pause());
        el.srcObject = null;
      }
      if (ctx) quiet(() => ctx!.close());
    },
  };
}

export interface ReceiverOptions {
  host: SiteId;
  stream: number;
  iceServers: IceServer[];
  sink: AudioSink;
  /** Send a signal to the host (`CollabCommand::SendSignal`). */
  send(signal: StreamSignal): void;
  /** The receiver failed (it already sent `Bye`). */
  onFailed(reason: string): void;
  /** Tests inject a fake. */
  createPeerConnection?: (config: RTCConfiguration) => RTCPeerConnection;
  /** `performance.timeOrigin + performance.now()` (the clock of sync source timestamps). */
  now?: () => number;
}

const wallNow = () => performance.timeOrigin + performance.now();

export function toWireCandidate(c: RTCIceCandidate | RTCIceCandidateInit | null): IceCandidate {
  return {
    candidate: c?.candidate ?? "",
    sdp_mid: c?.sdpMid ?? null,
    sdp_m_line_index: c?.sdpMLineIndex ?? null,
    username_fragment: c?.usernameFragment ?? null,
  };
}

export function fromWireCandidate(c: IceCandidate): RTCIceCandidateInit {
  return {
    candidate: c.candidate,
    sdpMid: c.sdp_mid,
    sdpMLineIndex: c.sdp_m_line_index,
    usernameFragment: c.username_fragment,
  };
}

const describe = (e: unknown) => (e instanceof Error ? e.message : String(e));

export class ListenReceiver {
  readonly host: SiteId;
  readonly stream: number;
  private readonly pc: RTCPeerConnection;
  private readonly o: ReceiverOptions;
  private readonly now: () => number;
  private rtp: RTCRtpReceiver | null = null;
  private pendingIce: RTCIceCandidateInit[] = [];
  private closed = false;
  private packets = 0;
  private lastMediaAt: number;
  private delaySec = 0;
  private timer: ReturnType<typeof setInterval>;
  /** Signals are applied in order (each awaits the previous). */
  private queue: Promise<void> = Promise.resolve();

  constructor(o: ReceiverOptions) {
    this.o = o;
    this.host = o.host;
    this.stream = o.stream;
    this.now = o.now ?? wallNow;
    const config: RTCConfiguration = {
      iceServers: o.iceServers.map((s) => ({
        urls: s.urls,
        ...(s.username != null ? { username: s.username } : {}),
        ...(s.credential != null ? { credential: s.credential } : {}),
      })),
    };
    this.pc = (o.createPeerConnection ?? ((c) => new RTCPeerConnection(c)))(config);
    this.pc.onicecandidate = (e) => {
      if (!this.closed) o.send({ type: "Ice", candidate: toWireCandidate(e.candidate) });
    };
    this.pc.ontrack = (e) => {
      this.rtp = e.receiver;
      o.sink.play(e.streams[0] ?? new MediaStream([e.track]));
    };
    this.pc.onconnectionstatechange = () => {
      if (this.pc.connectionState === "failed") this.fail("the connection failed");
    };
    this.lastMediaAt = this.now();
    this.timer = setInterval(() => void this.poll(), STATS_INTERVAL_MS);
  }

  get isClosed(): boolean {
    return this.closed;
  }

  /** A signal from the host for this stream (offer, ICE). */
  onSignal(signal: StreamSignal): Promise<void> {
    this.queue = this.queue.then(() => this.apply(signal));
    return this.queue;
  }

  private async apply(signal: StreamSignal): Promise<void> {
    if (this.closed) return;
    try {
      if (signal.type === "Offer") {
        await this.pc.setRemoteDescription({ type: "offer", sdp: signal.sdp });
        const answer = await this.pc.createAnswer();
        await this.pc.setLocalDescription(answer);
        if (this.closed) return;
        this.o.send({
          type: "Answer",
          sdp: this.pc.localDescription?.sdp ?? answer.sdp ?? "",
        });
        for (const c of this.pendingIce.splice(0)) await this.pc.addIceCandidate(c);
      } else if (signal.type === "Ice") {
        const c = fromWireCandidate(signal.candidate);
        if (this.pc.remoteDescription) await this.pc.addIceCandidate(c);
        else this.pendingIce.push(c);
      }
    } catch (e) {
      this.fail(`could not connect: ${describe(e)}`);
    }
  }

  /** The RTP timestamp being heard now (`null` before media or without `rtpTimestamp`). */
  playoutRtp(): number | null {
    const src = this.rtp?.getSynchronizationSources?.()[0];
    return src ? playoutRtp(src, this.now(), this.o.sink.outputLatency()) : null;
  }

  /** Fallback delay estimate (jitter buffer + half the RTT + output latency), seconds. */
  receiveDelaySec(): number {
    return this.delaySec + this.o.sink.outputLatency();
  }

  private async poll(): Promise<void> {
    if (this.closed) return;
    let jitter = 0;
    let rtt = 0;
    try {
      const stats = await this.pc.getStats();
      stats.forEach((r: Record<string, unknown>) => {
        if (r.type === "inbound-rtp" && r.kind === "audio") {
          const packets = Number(r.packetsReceived ?? 0);
          if (packets > this.packets) {
            this.packets = packets;
            this.lastMediaAt = this.now();
          }
          const emitted = Number(r.jitterBufferEmittedCount ?? 0);
          if (emitted > 0) jitter = Number(r.jitterBufferDelay ?? 0) / emitted;
        } else if (r.type === "candidate-pair" && r.nominated && typeof r.currentRoundTripTime === "number") {
          rtt = r.currentRoundTripTime;
        }
      });
    } catch {
      // stats unavailable: keep the last estimate
    }
    if (this.closed) return;
    this.delaySec = jitter + rtt / 2;
    if (this.now() - this.lastMediaAt > NO_MEDIA_TIMEOUT_MS) this.fail("no audio from the host for 10 s");
  }

  /** Give up: `Bye` to the host (the controller ends the stream), then close. */
  fail(reason: string): void {
    if (this.closed) return;
    this.o.send({ type: "Bye", reason });
    this.close();
    this.o.onFailed(reason);
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    clearInterval(this.timer);
    this.pc.onicecandidate = null;
    this.pc.ontrack = null;
    this.pc.onconnectionstatechange = null;
    this.pc.close();
    this.o.sink.close();
  }
}
