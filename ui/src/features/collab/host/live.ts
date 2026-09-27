// Wiring of the web sender to the real engine: capability detection (`SetHosting {
// ui_sender }`), the transport's stream tap, and the live timeline source.

import type { EngineTransport } from "@/transport";
import { useProjectStore } from "@/state/projectStore";
import { barsToBeats, encodedFrameStart, type HostTransport } from "./anchors";
import { observerKind } from "./observer";
import type { TimelineSource } from "./sender";
import { readTapClock } from "./tapClock";

/** `WasmTransport.streamOutput()` (the engine's stream tap, docs/COLLAB.md §9.1). */
export interface StreamOutput {
  stream: MediaStream;
  context: AudioContext;
  tapClock: SharedArrayBuffer;
}

/** The transport's stream tap, when it has one (the web build's `WasmTransport`). */
export function streamOutputOf(transport: EngineTransport): StreamOutput | null {
  const t = transport as EngineTransport & { streamOutput?: () => StreamOutput | null };
  return typeof t.streamOutput === "function" ? t.streamOutput() : null;
}

/**
 * `SetHosting { ui_sender }`: this UI can stream the engine itself: WebRTC with encoded
 * transforms (for the anchors) and a transport exposing the stream tap (web build).
 */
export function canSendFromUi(transport: EngineTransport, globals?: Parameters<typeof observerKind>[0]): boolean {
  return observerKind(globals) !== null && streamOutputOf(transport) !== null;
}

/** Host transport state from the project store (loop, metronome, count-in pre-roll). */
export function storeTransport(): HostTransport & { preRoll: number } {
  const { transport, project } = useProjectStore.getState();
  const sig = transport?.time_signature ?? { numerator: 4, denominator: 4 };
  return {
    loop_enabled: transport?.loop_enabled ?? false,
    loop_region: transport?.loop_region ?? { start: 0, end: 0 },
    metronome: transport?.metronome ?? false,
    preRoll: barsToBeats(project?.settings.count_in_bars ?? 0, sig),
  };
}

/** The timeline of the engine behind `out` (tap clock + AudioContext timing). */
export function liveTimeline(out: StreamOutput): TimelineSource {
  const ctx = out.context;
  return {
    sampleRate: ctx.sampleRate,
    read: () => readTapClock(out.tapClock),
    frameStart(perfMs) {
      const ts = ctx.getOutputTimestamp();
      if (!ts.contextTime || !ts.performanceTime) return null;
      // The render runs ahead of what is heard by the output (+ base) latency.
      const latency = (ctx.outputLatency || 0) + (ctx.baseLatency || 0);
      return encodedFrameStart(perfMs, { contextTime: ts.contextTime, performanceTime: ts.performanceTime }, latency, ctx.sampleRate);
    },
    transport: storeTransport,
  };
}
