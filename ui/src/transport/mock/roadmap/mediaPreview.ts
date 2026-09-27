/**
 * Mock of `Media::{Preview, StopPreview}` (`MockPreview`, owned by `media-preview`): a
 * preview "plays" for a fixed number of playhead steps. `Preview` validates the source,
 * emits `PreviewEnded { Replaced }` for a playing one, then `PreviewStarted`; `StopPreview`
 * emits `PreviewEnded { Stopped }`; after `PREVIEW_STEPS` steps (`MockTransport.tick()`
 * with manual timers) it emits `PreviewEnded { Finished }`. Same rules as the real
 * controller (CONTRACTS.md §11.15): exactly one `PreviewEnded` per preview; `Finished` only
 * for the current preview (a replaced one never reports `Finished`).
 */

import type { MediaSource, ReplyValue } from "@/generated";
import type { MockHost } from "./host";

const UNIT: ReplyValue = { type: "Unit" };
/** Playhead steps (~16 ms each) a mock preview lasts. */
export const PREVIEW_STEPS = 30;

export class MockPreview {
  private playing: { source: MediaSource; left: number } | null = null;

  constructor(private readonly host: MockHost) {}

  /** `Media::Preview` (the caller validated the source). */
  play(source: MediaSource): ReplyValue {
    this.end("Replaced");
    this.playing = { source, left: PREVIEW_STEPS };
    this.host.emit({ type: "Media", event: { type: "PreviewStarted", source } });
    return UNIT;
  }

  /** `Media::StopPreview`. */
  stop(): ReplyValue {
    this.end("Stopped");
    return UNIT;
  }

  /** Called on every playhead step. */
  step(): void {
    if (!this.playing) return;
    this.playing.left -= 1;
    if (this.playing.left <= 0) this.end("Finished");
  }

  private end(reason: "Finished" | "Stopped" | "Replaced"): void {
    const p = this.playing;
    if (!p) return;
    this.playing = null;
    this.host.emit({ type: "Media", event: { type: "PreviewEnded", source: p.source, reason } });
  }
}
