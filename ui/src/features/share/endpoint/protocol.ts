// Messages on the share port between the controller Worker and the UI thread
// (docs/SHARING.md §6.2). The Rust side is `crates/ether-collab/src/share/web/mod.rs`:
// keep the two in sync. `ch` is a channel number chosen by the Worker, one per pairing.
import type { IceServer, StreamSignal } from "@/generated";

/** Data channel label (`ether_collab::share::dc::DC_LABEL`). */
export const DC_LABEL = "ethereal-collab/1";

/** Worker → UI. */
export type ToUi =
  /** Create an `RTCPeerConnection`. `offer`: create the data channel and the offer (joiner). */
  | { type: "open"; ch: number; offer: boolean; ice: IceServer[]; relay: boolean }
  /** A remote signal for this pairing. */
  | { type: "signal"; ch: number; signal: StreamSignal }
  /** One data channel message (a fragment, `share::dc`), transferred. */
  | { type: "data"; ch: number; data: ArrayBuffer }
  /** Close this pairing's peer connection (no `closed` reply). */
  | { type: "close"; ch: number };

/** UI → Worker. */
export type FromUi =
  /** A local signal, for the signaling service. */
  | { type: "signal"; ch: number; signal: StreamSignal }
  /** The data channel is open; DTLS fingerprints as in SDP (`sha-256 AB:CD:...`). */
  | { type: "open"; ch: number; local: string; remote: string }
  /** One data channel message received, transferred. */
  | { type: "data"; ch: number; data: ArrayBuffer }
  /** Backpressure: bytes handed to the data channel so far, and its `bufferedAmount`. */
  | { type: "flow"; ch: number; consumed: number; buffered: number }
  /** The pairing ended (failed before opening, or closed after). */
  | { type: "closed"; ch: number; reason: string };
