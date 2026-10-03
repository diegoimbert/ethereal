// Test doubles of `RTCPeerConnection` / `RTCDataChannel` for the share endpoint agent
// (unit tests only).
import type { FromUi, ToUi } from "./protocol";
import type { SharePort } from "./agent";

export const SDP_FP = (hex: string) => `v=0\r\na=fingerprint:SHA-256 ${hex}\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=mid:0\r\n`;

export class FakeDataChannel {
  label: string;
  readyState: RTCDataChannelState = "connecting";
  binaryType = "blob";
  bufferedAmount = 0;
  bufferedAmountLowThreshold = 0;
  sent: ArrayBuffer[] = [];
  closed = false;
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  onbufferedamountlow: (() => void) | null = null;

  constructor(label: string) {
    this.label = label;
  }
  send(data: ArrayBuffer) {
    if (this.readyState !== "open") throw new Error("not open");
    this.sent.push(data);
    this.bufferedAmount += data.byteLength;
  }
  close() {
    this.closed = true;
    this.readyState = "closed";
  }

  // ── Test drivers ──
  open() {
    this.readyState = "open";
    this.onopen?.();
  }
  receive(data: unknown) {
    this.onmessage?.({ data });
  }
  drain() {
    this.bufferedAmount = 0;
    this.onbufferedamountlow?.();
  }
  remoteClose() {
    this.readyState = "closed";
    this.onclose?.();
  }
}

export class FakePeerConnection {
  static all: FakePeerConnection[] = [];
  config: RTCConfiguration;
  localDescription: RTCSessionDescriptionInit | null = null;
  remoteDescription: RTCSessionDescriptionInit | null = null;
  connectionState: RTCPeerConnectionState = "new";
  candidates: RTCIceCandidateInit[] = [];
  channels: FakeDataChannel[] = [];
  closed = false;
  onicecandidate: ((e: { candidate: RTCIceCandidateInit | null }) => void) | null = null;
  onconnectionstatechange: (() => void) | null = null;
  ondatachannel: ((e: { channel: FakeDataChannel }) => void) | null = null;

  constructor(config: RTCConfiguration = {}) {
    this.config = config;
    FakePeerConnection.all.push(this);
  }

  createDataChannel(label: string, _init?: RTCDataChannelInit) {
    const dc = new FakeDataChannel(label);
    this.channels.push(dc);
    return dc;
  }
  async createOffer(): Promise<RTCSessionDescriptionInit> {
    return { type: "offer", sdp: SDP_FP("aa:bb") };
  }
  async createAnswer(): Promise<RTCSessionDescriptionInit> {
    return { type: "answer", sdp: SDP_FP("cc:dd") };
  }
  async setLocalDescription(d: RTCSessionDescriptionInit) {
    this.localDescription = d;
  }
  async setRemoteDescription(d: RTCSessionDescriptionInit) {
    if (d.sdp === "bad") throw new Error("invalid SDP");
    this.remoteDescription = d;
  }
  async addIceCandidate(c: RTCIceCandidateInit) {
    this.candidates.push(c);
  }
  close() {
    this.closed = true;
  }

  // ── Test drivers ──
  emitCandidate(c: RTCIceCandidateInit | null) {
    this.onicecandidate?.({ candidate: c });
  }
  emitChannel(label: string) {
    const dc = new FakeDataChannel(label);
    this.channels.push(dc);
    this.ondatachannel?.({ channel: dc });
    return dc;
  }
  setState(s: RTCPeerConnectionState) {
    this.connectionState = s;
    this.onconnectionstatechange?.();
  }
}

export const fakePeerConnection = (config: RTCConfiguration) => new FakePeerConnection(config) as unknown as RTCPeerConnection;

/** A port double: what the agent posts is recorded, `deliver` plays the Worker. */
export class FakePort implements SharePort {
  posted: { message: FromUi; transfer?: Transferable[] }[] = [];
  onmessage: ((e: MessageEvent<ToUi>) => void) | null = null;

  postMessage(message: FromUi, transfer?: Transferable[]) {
    this.posted.push({ message, transfer });
  }
  deliver(message: ToUi) {
    this.onmessage?.({ data: message } as MessageEvent<ToUi>);
  }
  messages(): FromUi[] {
    return this.posted.map((p) => p.message);
  }
}
