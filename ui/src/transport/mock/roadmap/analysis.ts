/**
 * Mock of `Analysis::{Watch, Unwatch}` (v0.2, contracts-3). Owned by `fx-analysis` (the
 * channel itself is frozen: CONTRACTS.md §12.4.3). Watches are accepted like the engine;
 * `fx-analysis` adds simulated `Event::Analysis` frames (spectrum/tuner) for watched
 * devices at ≤ 30 Hz, `fx-dynamics` its gain-reduction `Levels`, `racks-modulation` the
 * `Modulation` readback.
 */

import type { AnalysisCommand, DeviceId, ReplyValue } from "@/generated";

export class MockAnalysis {
  readonly watched = new Set<DeviceId>();

  command(c: AnalysisCommand): ReplyValue {
    if (c.type === "Watch") this.watched.add(c.device);
    else this.watched.delete(c.device);
    return { type: "Unit" };
  }
}
