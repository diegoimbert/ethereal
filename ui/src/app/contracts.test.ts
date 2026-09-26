/**
 * Contract alignment between the Rust engine side and the UI, over shared vector files:
 * - `crates/ether-protocol/tests/param_scale_vectors.json`: normalized↔plain param
 *   scales (`ether_protocol::devices::scale_to_plain`) and the mixer automation targets'
 *   `ParamInfo` (`ether_controller::compile::track_param_info`: volume and send level =
 *   Fader over -144..+6 dB, pan = linear -1..1). Also checked by
 *   `crates/ether-protocol/tests/param_scale_vectors.rs` and
 *   `crates/ether-controller/tests/contract_vectors.rs`.
 * - Tempo vectors are checked in `ui/src/timeline/tempoVectors.test.ts`.
 */
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { ParamInfo, ParamScale, ParamUnit } from "@/generated";
import { PAN_INFO, SEND_INFO, VOLUME_INFO } from "@/features/automation";
import { scaleToNormalized, scaleToPlain } from "@/features/devices/paramScale";
import { dbToFader, faderToDb } from "@/features/mixer/routing";

interface Case {
  scale: ParamScale;
  min: number;
  max: number;
  normalized: number;
  plain: number;
}
interface Target {
  unit: ParamUnit;
  min: number;
  max: number;
  default: number;
  scale: ParamScale;
}
interface Vectors {
  tolerance: number;
  cases: Case[];
  mixer_targets: Record<"TrackVolume" | "SendLevel" | "TrackPan", Target>;
}

const PATH = resolve(import.meta.dirname, "../../../crates/ether-protocol/tests/param_scale_vectors.json");
const vectors = JSON.parse(readFileSync(PATH, "utf8")) as Vectors;

describe("param scale vectors (shared with ether-protocol)", () => {
  it("scaleToPlain / scaleToNormalized match Rust", () => {
    expect(vectors.cases.length).toBeGreaterThan(20);
    for (const c of vectors.cases) {
      const got = scaleToPlain(c.scale, c.min, c.max, c.normalized);
      expect(Math.abs(got - c.plain), JSON.stringify(c)).toBeLessThanOrEqual(vectors.tolerance);
      if (c.normalized > 0 && c.plain > c.min) {
        const back = scaleToNormalized(c.scale, c.min, c.max, c.plain);
        expect(Math.abs(back - c.normalized), JSON.stringify(c)).toBeLessThanOrEqual(1e-6);
      }
    }
  });
});

describe("mixer automation mapping (shared with ether-controller)", () => {
  const same = (name: string, info: ParamInfo, t: Target) => {
    expect({ unit: info.unit, min: info.min, max: info.max, default: info.default, scale: info.scale }, name).toEqual(t);
  };

  it("automation lanes use the controller's ParamInfo", () => {
    same("volume", VOLUME_INFO, vectors.mixer_targets.TrackVolume);
    same("send", SEND_INFO, vectors.mixer_targets.SendLevel);
    same("pan", PAN_INFO, vectors.mixer_targets.TrackPan);
  });

  it("mixer faders use the same law and range as automation (volume and sends: -144..+6 dB)", () => {
    for (const t of [vectors.mixer_targets.TrackVolume, vectors.mixer_targets.SendLevel]) {
      for (const n of [0, 0.1, 0.5, 0.8, 1]) {
        expect(faderToDb(n)).toBeCloseTo(scaleToPlain(t.scale, t.min, t.max, n), 9);
      }
      expect(faderToDb(1)).toBeCloseTo(t.max, 9);
      expect(dbToFader(t.max)).toBeCloseTo(1, 9);
    }
  });
});
