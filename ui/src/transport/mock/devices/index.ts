/**
 * v0.2 built-in device descriptors for the MockTransport (contracts-3).
 *
 * Each JSON file is **generated from Rust** (`ether_devices::descriptor`) and owned by its
 * device node; `crates/ether-devices/tests/v02_descriptors.rs` fails when one is stale.
 * Regenerate with `UPDATE_MOCK_DESCRIPTORS=1 cargo test -p ether-devices --test
 * v02_descriptors`; never edit them by hand. Layout specs (`DeviceDescriptor::layout`) travel
 * with them, so the shared device renderer (`device-ui`) shows the same panels on the mock
 * and on the real engine.
 */

import type { DeviceDescriptor, ModulatorDescriptor } from "@/generated";
import eq from "./eq.json";
import external from "./external.json";
import fxAnalysis from "./fxAnalysis.json";
import fxColor from "./fxColor.json";
import fxDynamics from "./fxDynamics.json";
import fxModulation from "./fxModulation.json";
import fxSpace from "./fxSpace.json";
import midiFx from "./midiFx.json";
import modulators from "./modulators.json";
import multisampler from "./multisampler.json";
import polySynth from "./polySynth.json";
import racks from "./racks.json";

/** The v0.2 built-in types (after `DrumRack` in `BuiltinDeviceType::ALL`). */
export type V02DeviceType =
  | "PolySynth"
  | "MultiSampler"
  | "Saturator"
  | "Bitcrusher"
  | "AutoFilter"
  | "Chorus"
  | "Phaser"
  | "Flanger"
  | "Tremolo"
  | "Gate"
  | "MultibandCompressor"
  | "TransientShaper"
  | "SpectrumAnalyzer"
  | "Tuner"
  | "Arpeggiator"
  | "Chord"
  | "ScaleQuantize"
  | "NoteLength"
  | "Velocity"
  | "Randomizer"
  | "InstrumentRack"
  | "AudioEffectRack"
  | "MidiEffectRack";

// JSON imports widen enum strings to `string`; the files are generated from the typed Rust
// values, so the cast is sound.
const groups = [polySynth, multisampler, fxColor, fxModulation, fxDynamics, fxAnalysis, midiFx, racks] as unknown as Record<
  V02DeviceType,
  DeviceDescriptor
>[];

export const V02_DESCRIPTORS: Readonly<Record<V02DeviceType, DeviceDescriptor>> = Object.assign({}, ...groups);

/** The v0.3 built-in types (contracts-4; after `MidiEffectRack` in `BuiltinDeviceType::ALL`). */
export type V03DeviceType = "ConvolutionReverb" | "ExternalInstrument" | "ExternalAudioEffect";

/**
 * v0.3 descriptors (contracts-4), generated from Rust like the v0.2 ones: `fxSpace.json`
 * (fx-space) and `external.json` (external-instrument). Regenerate with
 * `UPDATE_MOCK_DESCRIPTORS=1 cargo test -p ether-devices --test v03_descriptors`.
 */
export const V03_DESCRIPTORS: Readonly<Record<V03DeviceType, DeviceDescriptor>> = Object.assign(
  {},
  ...([fxSpace, external] as unknown as Record<V03DeviceType, DeviceDescriptor>[]),
);

/** The EQ (devices-2) with its v0.2 `EqCurve` layout (graphical-eq), from Rust. */
export const EQ_DESCRIPTOR: DeviceDescriptor = eq as unknown as DeviceDescriptor;

/** `Modulation::ListModulatorKinds` (from `ether_devices::modulators`). */
export const MODULATOR_DESCRIPTORS: ReadonlyArray<ModulatorDescriptor> = modulators as unknown as ModulatorDescriptor[];
