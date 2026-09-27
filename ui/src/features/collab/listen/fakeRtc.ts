// Test double of `RTCPeerConnection` for the listener's receiver (unit tests only).

export interface FakeSyncSource {
  rtpTimestamp?: number;
  timestamp: number;
  source: number;
}

export class FakeRtpReceiver {
  sources: FakeSyncSource[] = [];
  getSynchronizationSources(): FakeSyncSource[] {
    return this.sources;
  }
}

export class FakePeerConnection {
  static last: FakePeerConnection | null = null;
  config: RTCConfiguration;
  remoteDescription: RTCSessionDescriptionInit | null = null;
  localDescription: RTCSessionDescriptionInit | null = null;
  connectionState: RTCPeerConnectionState = "new";
  candidates: RTCIceCandidateInit[] = [];
  closed = false;
  stats: Record<string, unknown>[] = [];
  onicecandidate: ((e: { candidate: RTCIceCandidateInit | null }) => void) | null = null;
  ontrack: ((e: { receiver: FakeRtpReceiver; streams: unknown[]; track: unknown }) => void) | null = null;
  onconnectionstatechange: (() => void) | null = null;
  readonly receiver = new FakeRtpReceiver();

  constructor(config: RTCConfiguration = {}) {
    this.config = config;
    FakePeerConnection.last = this;
  }

  async setRemoteDescription(d: RTCSessionDescriptionInit) {
    if (d.sdp === "bad") throw new Error("invalid SDP");
    this.remoteDescription = d;
  }
  async createAnswer(): Promise<RTCSessionDescriptionInit> {
    return { type: "answer", sdp: `answer to ${this.remoteDescription?.sdp}` };
  }
  async setLocalDescription(d: RTCSessionDescriptionInit) {
    this.localDescription = d;
  }
  async addIceCandidate(c: RTCIceCandidateInit) {
    this.candidates.push(c);
  }
  async getStats() {
    return new Map(this.stats.map((s, i) => [String(i), s]));
  }
  close() {
    this.closed = true;
  }

  // ── Test drivers ──
  emitCandidate(c: RTCIceCandidateInit | null) {
    this.onicecandidate?.({ candidate: c });
  }
  emitTrack(stream: unknown = { id: "remote" }) {
    this.ontrack?.({ receiver: this.receiver, streams: [stream], track: {} });
  }
  setState(s: RTCPeerConnectionState) {
    this.connectionState = s;
    this.onconnectionstatechange?.();
  }
}

/** Use as `createPeerConnection` (typed as the DOM one). */
export const fakePeerConnection = (config: RTCConfiguration) => new FakePeerConnection(config) as unknown as RTCPeerConnection;

export function fakeSink() {
  const sink = {
    played: [] as unknown[],
    closed: false,
    latency: 0,
    play(stream: unknown) {
      sink.played.push(stream);
    },
    outputLatency: () => sink.latency,
    close() {
      sink.closed = true;
    },
  };
  return sink;
}
