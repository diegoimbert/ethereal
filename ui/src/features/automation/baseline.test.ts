import { describe, expect, it } from "vitest";
import type { Device, ParamInfo, Track, TrackSend } from "@/generated";
import { paramToPlain } from "@/domain/paramScale";
import { emptyLaneBaseline, targetValue, type ValueTables } from "./baseline";
import { LANE_PAD, valueToY } from "./geometry";
import { CLIP_GAIN_INFO, CLIP_TRANSPOSE_INFO, DEVICE_ON_INFO, PAN_INFO, SEND_INFO, VOLUME_INFO } from "./params";

const CUTOFF: ParamInfo = {
  id: 3,
  name: "cutoff",
  group: null,
  unit: "Hertz",
  min: 20,
  max: 20000,
  default: 1000,
  scale: { type: "Log" },
  labels: null,
  automatable: true,
  hidden: false,
};
const WAVE: ParamInfo = {
  ...CUTOFF,
  id: 4,
  name: "wave",
  unit: "None",
  min: 0,
  max: 3,
  default: 0,
  scale: { type: "Linear" },
  labels: ["Sine", "Tri", "Saw", "Square"],
};
const SYNC: ParamInfo = { ...CUTOFF, id: 5, name: "sync", unit: "Toggle", min: 0, max: 1, default: 0, scale: { type: "Linear" }, labels: null };

const tables = (over: { volume?: number; pan?: number; level?: number; params?: Record<number, number>; enabled?: boolean } = {}): ValueTables => ({
  tracks: { t: { id: "t", mixer: { volume: over.volume ?? 0, pan: over.pan ?? 0, mute: false, solo: false } } as unknown as Track },
  sends: { s: { id: "s", from: "t", to: "r", level: over.level ?? -12, pre_fader: false } as TrackSend },
  devices: { d: { id: "d", track: "t", enabled: over.enabled ?? true, params: over.params ?? {} } as unknown as Device },
});

describe("targetValue: the document value, normalized like the lane's points", () => {
  it("track volume and send level go through the fader law", () => {
    const v = targetValue(tables({ volume: -6 }), { type: "TrackVolume", track: "t" }, VOLUME_INFO)!;
    expect(paramToPlain(VOLUME_INFO, v)).toBeCloseTo(-6, 9);
    expect(targetValue(tables(), { type: "TrackVolume", track: "t" }, VOLUME_INFO)).toBeCloseTo(Math.cbrt(10 ** (-6 / 20)), 9);
    const s = targetValue(tables({ level: -12 }), { type: "SendLevel", send: "s" }, SEND_INFO)!;
    expect(paramToPlain(SEND_INFO, s)).toBeCloseTo(-12, 9);
  });

  it("pan is bipolar: center is the middle of the lane", () => {
    expect(targetValue(tables({ pan: 0 }), { type: "TrackPan", track: "t" }, PAN_INFO)).toBe(0.5);
    expect(targetValue(tables({ pan: -0.5 }), { type: "TrackPan", track: "t" }, PAN_INFO)).toBe(0.25);
    expect(valueToY(0.5, 64)).toBe(32);
  });

  it("log params map on the log scale; a missing value is the default", () => {
    const target = { type: "DeviceParam" as const, device: "d", param: 3 };
    // 632 Hz is halfway between 20 Hz and 20 kHz on a log axis.
    expect(targetValue(tables({ params: { 3: Math.sqrt(20 * 20000) } }), target, CUTOFF)).toBeCloseTo(0.5, 9);
    expect(targetValue(tables(), target, CUTOFF)).toBeCloseTo(Math.log(1000 / 20) / Math.log(1000), 9);
  });

  it("enums sit on their step, toggles at their on / off level", () => {
    expect(targetValue(tables({ params: { 4: 2 } }), { type: "DeviceParam", device: "d", param: 4 }, WAVE)).toBeCloseTo(2 / 3, 9);
    expect(targetValue(tables({ params: { 5: 1 } }), { type: "DeviceParam", device: "d", param: 5 }, SYNC)).toBe(1);
    expect(targetValue(tables({ enabled: true }), { type: "DeviceEnabled", device: "d" }, DEVICE_ON_INFO)).toBe(1);
    expect(targetValue(tables({ enabled: false }), { type: "DeviceEnabled", device: "d" }, DEVICE_ON_INFO)).toBe(0);
    // On: the top of the lane; off: the bottom.
    expect(valueToY(1, 64)).toBe(LANE_PAD);
    expect(valueToY(0, 64)).toBe(64 - LANE_PAD);
  });

  it("clip envelopes are offsets: neutral (0 dB, 0 st)", () => {
    expect(paramToPlain(CLIP_GAIN_INFO, targetValue(tables(), { type: "ClipGain" }, CLIP_GAIN_INFO)!)).toBeCloseTo(0, 9);
    expect(targetValue(tables(), { type: "ClipTranspose" }, CLIP_TRANSPOSE_INFO)).toBe(0.5);
  });

  it("is null when the entity is gone", () => {
    expect(targetValue(tables(), { type: "TrackVolume", track: "gone" }, VOLUME_INFO)).toBeNull();
    expect(targetValue(tables(), { type: "DeviceParam", device: "gone", param: 3 }, CUTOFF)).toBeNull();
  });
});

describe("emptyLaneBaseline", () => {
  it("draws the current value only while the lane has no points", () => {
    expect(emptyLaneBaseline(0, 0.4)).toBe(0.4);
    expect(emptyLaneBaseline(1, 0.4)).toBeNull();
    expect(emptyLaneBaseline(0, null)).toBeNull();
  });
});
