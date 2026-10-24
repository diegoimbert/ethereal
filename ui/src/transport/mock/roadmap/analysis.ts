/**
 * Mock of `Analysis::{Watch, Unwatch}` (v0.2; CONTRACTS.md §12.4.3). Owned by `fx-analysis`.
 * Watches are refcounted like the controller's. Every `step()` (the mock's ~30 Hz meter
 * tick) emits simulated `Event::Analysis` frames for the watched devices:
 *
 * - `SpectrumAnalyzer`: a moving pink-ish mix spectrum (256 log bins, 20 Hz – 20 kHz) with a
 *   bass line and a few partials, tilted by the device's `Slope` and smoothed by `Averaging`;
 * - `Eq`: the same as `Pre` + `Post` frames (for the EQ overlay);
 * - `Tuner`: an A2 string drifting a few cents around pitch, relative to `Reference`;
 * - `Loudness`: a song-like loudness curve around -14 LUFS (verses and choruses), its
 *   history at 10 points per second and the readouts, in the engine's `Levels` layout;
 * - `Maximizer`: drum-like peaks driven by the `Threshold` into the `Ceiling`, the gain
 *   reduction they cost and 4 s of history (50 points per second), in the engine's layout;
 * - `PitchCorrection`: a sung phrase (a few notes with vibrato and slides, breaths between)
 *   snapped to the nearest semitone, with its history, in the engine's layout;
 * - `AdaptiveEq`: a mix with a boomy low end, a harsh 3 kHz and dull highs, and the curve
 *   that answers it (scaled by `Recover` / `Tame`), in the engine's layout.
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
      case "Loudness":
        return [this.loudness(device.params[1] ?? 1)];
      case "AdaptiveEq":
        return [this.adaptiveEq(device.params[0] ?? 50, device.params[1] ?? 50)];
      case "PitchCorrection":
        return [this.pitchCorrection()];
      case "Maximizer":
        return [this.maximizer(device.params[0] ?? 0, device.params[1] ?? -1)];
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

  /** Momentary loudness of the simulated song at `t` seconds. */
  private static songLufs(t: number): number {
    const chorus = Math.floor(t / 8) % 2 === 1 ? 4 : 0;
    return -18 + chorus + 2.5 * Math.sin(t * 7.3) + 1.2 * Math.sin(t * 2.1);
  }

  private loudness(history: number): AnalysisData {
    const t = this.frame / 30;
    const window = [10, 30, 60, 300, 900][Math.round(history)] ?? 30;
    const bucket = Math.ceil((window * 10) / 300);
    const step = bucket / 10;
    const n = Math.min(Math.floor((t * 10) / bucket), Math.ceil((window * 10) / bucket));
    const at = (k: number) => t - (n - 1 - k) * step;
    const mom = Array.from({ length: n }, (_, k) => Math.round(MockAnalysis.songLufs(at(k)) * 10) / 10);
    const short = Array.from({ length: n }, (_, k) => Math.round((MockAnalysis.songLufs(at(k)) - 2.5 * Math.sin(at(k) * 7.3)) * 10) / 10);
    const momentary = MockAnalysis.songLufs(t);
    return {
      type: "Levels",
      values: [
        momentary,
        short[n - 1] ?? -120,
        t > 0.4 ? -15.2 : -120,
        t > 3 ? 6.4 : 0,
        -1.4,
        -2 - Math.abs(Math.sin(t * 3)) * 4,
        window,
        step,
        n,
        ...mom,
        ...short,
      ],
    };
  }

  private adaptiveEq(recover: number, tame: number): AnalysisData {
    const t = this.frame / 30;
    const bands = 32;
    const lo = 40;
    const ratio = Math.pow(16000 / lo, 1 / (bands - 1));
    const freqs = Array.from({ length: bands }, (_, k) => lo * Math.pow(ratio, k));
    const oct = (f: number) => Math.log2(f / 1000);
    const bump = (f: number, at: number, db: number, width: number) => db * Math.exp(-0.5 * Math.pow(Math.log2(f / at) / width, 2));
    const levels = freqs.map((f) => bump(f, 120, 6, 0.7) + bump(f, 3000, 5, 0.25) + Math.min(0, -2.5 * (oct(f) - 2)) + 0.6 * Math.sin(t * 1.3 + f / 900));
    const gains = levels.map((l) => Math.max(-6, Math.min(6, l > 0 ? (-l * tame) / 100 : (-l * recover) / 100)));
    const n = 64;
    const pts = Array.from({ length: n }, (_, i) => 20 * Math.pow(1000, i / (n - 1)));
    const response = pts.map((f) => {
      let k = 0;
      while (k + 1 < bands && freqs[k + 1]! < f) k++;
      return f < freqs[0]! ? gains[0]! * Math.max(0, 1 - Math.log2(freqs[0]! / f)) : (gains[k] ?? 0);
    });
    const auto = -gains.reduce((a, g) => a + g, 0) / bands / 2;
    return { type: "Levels", values: [n, auto, ...pts, ...response, ...freqs, ...levels, ...gains] };
  }

  /** Detected pitch (MIDI note) of the simulated singer at `t` seconds, -1 in a breath. */
  private static sung(t: number): number {
    const phrase = t % 4;
    if (phrase > 3.4) return -1;
    const melody = [57, 60, 62, 60, 64, 62];
    const note = melody[Math.floor(phrase / 0.6)] ?? 57;
    return note + 0.3 * Math.sin(t * 2 * Math.PI * 5.5) - 0.25 * Math.exp(-((phrase % 0.6) * 12));
  }

  private pitchCorrection(): AnalysisData {
    const t = this.frame / 30;
    const step = 0.02;
    const n = Math.min(200, Math.floor(t / step));
    const at = (k: number) => t - (n - 1 - k) * step;
    const round = (v: number) => Math.round(v * 100) / 100;
    const detected = Array.from({ length: n }, (_, k) => round(MockAnalysis.sung(at(k))));
    const output = detected.map((v) => (v < 0 ? -1 : Math.round(v) + 0.05 * (v - Math.round(v))));
    const now = MockAnalysis.sung(t);
    const out = now < 0 ? -1 : Math.round(now) + 0.05 * (now - Math.round(now));
    return {
      type: "Levels",
      values: [now, out, now < 0 ? -1 : Math.round(now), now < 0 ? 0 : (out - now) * 100, now < 0 ? 0.1 : 0.95, step, n, ...detected, ...output],
    };
  }

  /** Input peak (dBFS) of the simulated drum loop at `t` seconds: a hit every 0.25 s. */
  private static drumPeak(t: number): number {
    const phase = (t * 4) % 1;
    return -14 + 10 * Math.exp(-phase * 12) + 1.5 * Math.sin(t * 1.7);
  }

  private maximizer(threshold: number, ceiling: number): AnalysisData {
    const t = this.frame / 30;
    const step = 0.02;
    const n = Math.min(200, Math.floor(t / step));
    const at = (k: number) => t - (n - 1 - k) * step;
    const round = (v: number) => Math.round(v * 10) / 10;
    const input = Array.from({ length: n }, (_, k) => round(MockAnalysis.drumPeak(at(k))));
    const gr = input.map((v) => round(Math.min(0, ceiling - (v - threshold))));
    const output = input.map((v, k) => round(v - threshold + gr[k]!));
    const now = MockAnalysis.drumPeak(t);
    const grNow = Math.min(0, ceiling - (now - threshold));
    const deepest = gr.reduce((a, b) => Math.min(a, b), 0);
    const outMax = output.reduce((a, b) => Math.max(a, b), -90);
    return { type: "Levels", values: [grNow, now, now - threshold + grNow, n ? outMax : -90, deepest, step, n, ...input, ...output, ...gr] };
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
