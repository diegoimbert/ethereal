// Read-only sender transform (`RTCRtpScriptTransform`) of the web host: reports the RTP
// timestamp of every encoded audio frame with the instant it went by, and passes the frame
// through untouched. Posts `{ id, rtp, at }` (`at` = `performance.timeOrigin + now()`, so
// the page can convert it to its own `performance.now()` clock).

interface EncodedFrame {
  timestamp?: number;
  getMetadata?(): { rtpTimestamp?: number };
}

interface TransformEvent {
  transformer: {
    options?: { id?: number };
    readable: ReadableStream<EncodedFrame>;
    writable: WritableStream<EncodedFrame>;
  };
}

const scope = self as unknown as {
  onrtctransform: ((e: TransformEvent) => void) | null;
  postMessage(message: unknown): void;
};

scope.onrtctransform = ({ transformer }) => {
  const id = transformer.options?.id ?? 0;
  void transformer.readable
    .pipeThrough(
      new TransformStream<EncodedFrame, EncodedFrame>({
        transform(frame, controller) {
          const rtp = frame.getMetadata?.().rtpTimestamp ?? frame.timestamp;
          if (typeof rtp === "number") scope.postMessage({ id, rtp, at: performance.timeOrigin + performance.now() });
          controller.enqueue(frame);
        },
      }),
    )
    .pipeTo(transformer.writable)
    .catch(() => undefined);
};
