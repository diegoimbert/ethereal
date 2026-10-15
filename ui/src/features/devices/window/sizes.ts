/**
 * Default width of a factory device's window, per device type (devices-pass-instruments).
 * A device whose layout has more sections than fit side by side in the base width
 * (`--eth-size-device-window-width`) opens wider, so its sections sit in rows instead of one
 * long column. The user's own size (kit `FloatingWindow` geometry) always wins.
 *
 * Factors of the base width, never pixels: the window follows the size tokens. APPEND-ONLY
 * table: each device pass adds its own entries (instruments here, effects below).
 */
import type { BuiltinDeviceType, Device } from "@/generated";
import { size } from "@/theme/tokens";

const BASE = parseFloat(size.deviceWindowWidth);

const FACTOR: Partial<Record<BuiltinDeviceType, number>> = {
  // --- instruments (devices-pass-instruments) ---
  // Osc 1 / Osc 2 / Sub, filter, three envelopes, two LFOs, unison, voice, output.
  PolySynth: 1.75,
  // Sample (waveform), playback, loop, envelope, filter, output.
  Sampler: 1.5,
  // Zone map and inspector, then the global pitch / voice / envelopes / filter.
  MultiSampler: 1.5,
  // Pad grid beside the selected pad's settings and its (compact) device chain.
  DrumRack: 1.35,
  // Algorithm picker and the four operators side by side, then pitch / LFO / filter / output.
  FmSynth: 2,
  // --- effects (devices-pass-effects; widths set with the instruments pass) ---
  // Eight bands side by side under the curve (~760 px): every band's controls readable.
  Eq: 1.35,
  // Transfer curve and gain-reduction meter, then timing / output / sidechain in one row.
  Compressor: 1.25,
  Gate: 1.25,
  Limiter: 1.15,
  // Three band sections side by side under the crossover.
  MultibandCompressor: 1.5,
  // Echo taps, then filter / modulation / output.
  Delay: 1.25,
  // Decay display, then input filter / character / output.
  Reverb: 1.25,
  ConvolutionReverb: 1.25,
  // Controls beside the output meters.
  Utility: 1.15,
  // The spectrum is the device.
  SpectrumAnalyzer: 1.5,
};

/** The default width (px) of `device`'s window. */
export function deviceWindowWidth(device: Pick<Device, "kind">): number {
  const factor = device.kind.type === "Builtin" ? (FACTOR[device.kind.device.type] ?? 1) : 1;
  return Math.round(BASE * factor);
}
