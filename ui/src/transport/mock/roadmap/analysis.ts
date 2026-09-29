/**
 * Mock of `Analysis::{Watch, Unwatch}` (v0.2; CONTRACTS.md §12.4.3). Owned by `fx-analysis`.
 * Watches are refcounted like the controller's. Every `step()` (the mock's ~30 Hz meter
 * tick) emits simulated `Event::Analysis` frames for the watched devices:
 *
 * - `SpectrumAnalyzer`: a moving pink-ish mix spectrum (256 log bins, 20 Hz – 20 kHz) with a
 *   bass line and a few partials, tilted by the device's `Slope` and smoothed by `Averaging`;
 * - `Eq`: the same as `Pre` + `Post` frames (for the EQ overlay);
 * - `Tuner`: an A2 string drifting a few cents around pitch, relative to `Reference`.
 *
 * Deterministic (own PRNG, frame counter), so tests can assert on it.
 */

import type { AnalysisCommand, AnalysisData, Device, DeviceId, ReplyValue } from "@/generated";
import type { MockHost } from "./host";

/** Log bins per spectrum frame (as the engine's analyzer). */
export const MOCK_SPECTRUM_BINS = 256;
const MIN_HZ = 20;
const MAX_HZ = 20_000;

export class MockAnalysis {
  /** Device → watch count. */
  readonly counts = new Map<DeviceId, number>();
  private frame = 0;
  private seed = 0x9e3779b9;
  /** `<device>:<stage>` → last spectrum (averaging). */
  private readonly smooth = new Map<string, number[]>();

  constructor(private readonly host?: Pick<MockHost, "project" | "emit">) {}

  /** Devices currently watched. */
  get watched(): ReadonlySet<DeviceId> {
    return new Set(this.counts.keys());
  }

  command(c: AnalysisCommand): ReplyValue {
    const n = this.counts.get(c.device) ?? 0;
    if (c.type === "Watch") this.counts.set(c.device, n + 1);
    else if (n <= 1) {
      this.counts.delete(c.device);
      for (const stage of ["Pre", "Post"]) this.smooth.delete(`${c.device}:${stage}`);
    } else this.counts.set(c.device, n - 1);
    return { type: "Unit" };
  }

  /** One analysis pass (~30 Hz): a frame per watched device and kind. */
  step(): void {
    if (!this.host || this.counts.size === 0) return;
    this.frame++;
    const project = this.host.project();
    for (const id of this.counts.keys()) {
      const device = project.devices[id];
      if (!device || device.kind.type !== "Builtin") continue;
      for (const data of this.framesOf(device)) this.host.emit({ type: "Analysis", event: { type: "Frame", device: id, data } });
    }
  }

  private framesOf(device: Device): AnalysisData[] {
    if (device.kind.type !== "Builtin") return [];
    switch (device.kind.device.type) {
      case "SpectrumAnalyzer": {
        const slope = device.params[4] ?? 3;
        const averaging = (device.params[1] ?? 60) / 100;
        return [this.spectrum(device.id, slope, averaging, "Post")];
      }
      case "Eq":
        return [this.spectrum(device.id, 0, 0.6, "Pre", 2), this.spectrum(device.id, 0, 0.6, "Post")];
      case "Tuner":
        return [this.tuner(device.params[0] ?? 440)];
      default:
        return [];
    }
  }

  private rand(): number {
    // xorshift32
    let s = this.seed;
    s ^= s << 13;
    s ^= s >>> 17;
    s ^= s << 5;
    this.seed = s >>> 0;
    return this.seed / 0x1_0000_0000;
  }

  private spectrum(id: DeviceId, slope: number, averaging: number, stage: "Pre" | "Post", lift = 0): AnalysisData {
    const t = this.frame / 30;
    const key = `${id}:${stage}`;
    const prev = this.smooth.get(key);
    const keep = prev ? 0.95 * averaging : 0;
    const bins: number[] = [];
    // Bass note moving every beat-ish, a few partials, hats up top.
    const bass = 55 * Math.pow(2, [0, 3, 5, 7][Math.floor(t * 2) % 4]! / 12);
    for (let i = 0; i < MOCK_SPECTRUM_BINS; i++) {
      const f = MIN_HZ * Math.pow(MAX_HZ / MIN_HZ, i / (MOCK_SPECTRUM_BINS - 1));
      const oct = Math.log2(f / 1000);
      // Pink-ish mix: -3 dB/oct around -30 dB at 1 kHz, rolled off at both ends.
      let db = -30 - 3 * oct - (f < 40 ? 18 * Math.log2(40 / f) : 0) - (f > 12_000 ? 24 * Math.log2(f / 12_000) : 0);
      for (let h = 1; h <= 4; h++) {
        const d = Math.log2(f / (bass * h)) * 24;
        db = Math.max(db, -12 - 6 * h - d * d);
      }
      const hat = Math.log2(f / 8000) * 3;
      db = Math.max(db, -38 + 8 * Math.sin(t * 9) - hat * hat);
      db += lift + (this.rand() - 0.5) * 6 + slope * oct;
      const v = prev ? keep * prev[i]! + (1 - keep) * db : db;
      bins.push(Math.round(v * 10) / 10);
    }
    this.smooth.set(key, bins);
    return { type: "Spectrum", min_hz: MIN_HZ, max_hz: MAX_HZ, bins_db: bins, stage };
  }

  private tuner(reference: number): AnalysisData {
    const t = this.frame / 30;
    // An A2 string settling towards pitch, with a little wobble.
    const cents = 14 * Math.exp(-((t % 8) / 3)) + 1.5 * Math.sin(t * 5);
    const hz = 110 * Math.pow(2, cents / 1200);
    const midi = 69 + 12 * Math.log2(hz / reference);
    const note = Math.round(midi);
    return {
      type: "Tuner",
      hz: Math.round(hz * 100) / 100,
      note,
      cents: Math.round((midi - note) * 1000) / 10,
      confidence: 0.97,
      level_db: -14,
    };
  }
}
