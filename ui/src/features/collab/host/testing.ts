// Test doubles of the web sender: a scripted RTCPeerConnection, an encoded-frame observer
// the test drives, and a timeline (1 ms of performance time = 48 context frames).
import type { FrameListener, RtpObserver } from "./observer";
import type { TimelineSource } from "./sender";
import type { TapClock, TapState } from "./tapClock";

export const FAKE_OFFER = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\na=fmtp:111 minptime=10;useinbandfec=1\r\n";

export class FakePeerConnection {
  static created: FakePeerConnection[] = [];
  onicecandidate: ((e: { candidate: RTCIceCandidate | null }) => void) | null = null;
  onconnectionstatechange: (() => void) | null = null;
  connectionState: RTCPeerConnectionState = "new";
  localDescription: RTCSessionDescriptionInit | null = null;
  remoteDescription: RTCSessionDescriptionInit | null = null;
  readonly tracks: MediaStreamTrack[] = [];
  readonly candidates: RTCIceCandidateInit[] = [];
  closed = false;
  rejectAnswer = false;

  constructor(readonly config: RTCConfiguration) {
    FakePeerConnection.created.push(this);
  }

  addTrack(track: MediaStreamTrack): RTCRtpSender {
    this.tracks.push(track);
    return { track } as unknown as RTCRtpSender;
  }
  createOffer(): Promise<RTCSessionDescriptionInit> {
    return Promise.resolve({ type: "offer", sdp: FAKE_OFFER });
  }
  setLocalDescription(d: RTCSessionDescriptionInit): Promise<void> {
    this.localDescription = d;
    return Promise.resolve();
  }
  setRemoteDescription(d: RTCSessionDescriptionInit): Promise<void> {
    if (this.rejectAnswer) return Promise.reject(new Error("bad sdp"));
    this.remoteDescription = d;
    return Promise.resolve();
  }
  addIceCandidate(c: RTCIceCandidateInit): Promise<void> {
    this.candidates.push(c);
    return Promise.resolve();
  }
  close(): void {
    this.closed = true;
    this.connectionState = "closed";
  }

  // ─── test drivers ───
  setState(state: RTCPeerConnectionState): void {
    this.connectionState = state;
    this.onconnectionstatechange?.();
  }
  gather(candidate: string | null): void {
    this.onicecandidate?.({
      candidate: candidate === null ? null : ({ candidate, sdpMid: "0", sdpMLineIndex: 0, usernameFragment: "uf" } as RTCIceCandidate),
    });
  }
}

export function createFakePeer(config: RTCConfiguration): RTCPeerConnection {
  return new FakePeerConnection(config) as unknown as RTCPeerConnection;
}

export function fakeStream(): MediaStream {
  const track = { kind: "audio", contentHint: "" } as MediaStreamTrack;
  return { getAudioTracks: () => [track] } as unknown as MediaStream;
}

/** An observer whose frames the test emits (`emit(i, rtp, perfMs)` for the i-th sender). */
export class FakeObserver implements RtpObserver {
  kind = "streams" as const;
  peerConfig = { encodedInsertableStreams: true };
  readonly listeners: (FrameListener | null)[] = [];
  disposed = false;
  attach(_sender: RTCRtpSender, onFrame: FrameListener): () => void {
    const i = this.listeners.push(onFrame) - 1;
    return () => {
      this.listeners[i] = null;
    };
  }
  emit(i: number, rtp: number, perfMs: number): void {
    this.listeners[i]?.(rtp, perfMs);
  }
  dispose(): void {
    this.disposed = true;
  }
}

/** A timeline whose tap clock the test sets; context frame = `perfMs * 48`. */
export class FakeTimeline implements TimelineSource {
  sampleRate = 48_000;
  clock: TapClock | null = null;
  preRoll = 0;
  read = () => this.clock;
  frameStart = (perfMs: number) => perfMs * 48;
  transport = () => ({ loop_enabled: false, loop_region: { start: 0, end: 16 }, metronome: true, preRoll: this.preRoll });

  set(latest: Partial<TapState>, event?: Partial<TapState>): void {
    const base: TapState = { tapFrame: 0, position: 0, playing: true, recording: false, bpm: 120, latency: 0 };
    const prev = this.clock;
    this.clock = {
      sampleRate: 48_000,
      latest: { ...base, ...latest },
      events: (prev?.events ?? 0) + (event ? 1 : 0),
      event: event ? { ...base, ...event } : (prev?.event ?? base),
    };
  }
}
