// Read-only encoded transforms on the web sender (docs/COLLAB.md §9.4 "Host math (web)"):
// observe the RTP timestamp of each encoded audio frame at `performance.now()`.
//
// Two APIs, feature-detected: `RTCRtpScriptTransform` (standard; Safari, Firefox, recent
// Chromium), else Chromium's `RTCRtpSender.createEncodedStreams()` (needs the peer
// connection created with `encodedInsertableStreams: true`).

/** `(rtp, perfMs)`: an encoded frame with RTP timestamp `rtp` went by at `perfMs`. */
export type FrameListener = (rtp: number, perfMs: number) => void;

export type ObserverKind = "script" | "streams";

export interface RtpObserver {
  kind: ObserverKind;
  /** Extra `RTCConfiguration` fields the peer connection needs. */
  peerConfig: Record<string, unknown>;
  /** Observe `sender`'s encoded frames; returns a detach function. */
  attach(sender: RTCRtpSender, onFrame: FrameListener): () => void;
  dispose(): void;
}

interface Globals {
  RTCPeerConnection?: unknown;
  RTCRtpScriptTransform?: new (worker: Worker, options?: unknown) => unknown;
  RTCRtpSender?: { prototype: object };
}

/** Which encoded transform API this browser has (`null`: none, the UI cannot host). */
export function observerKind(g: Globals = globalThis as Globals): ObserverKind | null {
  if (typeof g.RTCPeerConnection === "undefined") return null;
  if (typeof g.RTCRtpScriptTransform === "function") return "script";
  if (g.RTCRtpSender && "createEncodedStreams" in g.RTCRtpSender.prototype) return "streams";
  return null;
}

interface EncodedFrame {
  timestamp?: number;
  getMetadata?(): { rtpTimestamp?: number };
}

function rtpOf(frame: EncodedFrame): number | undefined {
  return frame.getMetadata?.().rtpTimestamp ?? frame.timestamp;
}

/** Chromium: the transform runs on the main thread. */
function streamsObserver(): RtpObserver {
  return {
    kind: "streams",
    peerConfig: { encodedInsertableStreams: true },
    attach(sender, onFrame) {
      const s = sender as unknown as {
        createEncodedStreams(): { readable: ReadableStream<EncodedFrame>; writable: WritableStream<EncodedFrame> };
      };
      const { readable, writable } = s.createEncodedStreams();
      let live = true;
      void readable
        .pipeThrough(
          new TransformStream<EncodedFrame, EncodedFrame>({
            transform(frame, controller) {
              const rtp = rtpOf(frame);
              if (live && typeof rtp === "number") onFrame(rtp, performance.now());
              controller.enqueue(frame);
            },
          }),
        )
        .pipeTo(writable)
        .catch(() => undefined);
      return () => {
        live = false;
      };
    },
    dispose() {},
  };
}

/** Standard: one shared worker (./rtpObserver.worker.ts) for every sender. */
function scriptObserver(ctor: NonNullable<Globals["RTCRtpScriptTransform"]>): RtpObserver {
  let worker: Worker | null = null;
  const listeners = new Map<number, FrameListener>();
  let nextId = 1;
  const getWorker = () => {
    if (!worker) {
      worker = new Worker(new URL("./rtpObserver.worker.ts", import.meta.url), { type: "module", name: "ether-rtp-observer" });
      worker.onmessage = (e: MessageEvent<{ id: number; rtp: number; at: number }>) => {
        listeners.get(e.data.id)?.(e.data.rtp, e.data.at - performance.timeOrigin);
      };
    }
    return worker;
  };
  return {
    kind: "script",
    peerConfig: {},
    attach(sender, onFrame) {
      const id = nextId++;
      listeners.set(id, onFrame);
      (sender as unknown as { transform: unknown }).transform = new ctor(getWorker(), { side: "send", id });
      return () => {
        listeners.delete(id);
      };
    },
    dispose() {
      worker?.terminate();
      worker = null;
      listeners.clear();
    },
  };
}

/** The observer for this browser, or `null` when it has no encoded transforms. */
export function createRtpObserver(g: Globals = globalThis as Globals): RtpObserver | null {
  const kind = observerKind(g);
  if (kind === "script") return scriptObserver(g.RTCRtpScriptTransform!);
  if (kind === "streams") return streamsObserver();
  return null;
}
