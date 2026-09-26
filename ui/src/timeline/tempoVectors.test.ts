/**
 * Cross-checks the UI tempo map against the shared vectors committed by `ether-model`
 * (the source of truth): `crates/ether-model/tests/tempo_vectors.json`. Read-only.
 * Skipped while the file doesn't exist yet.
 *
 * Accepted shape (fields optional unless noted):
 * {
 *   "tolerance": 1e-9,
 *   "cases": [{
 *     "name": "...",
 *     "tempo": [TempoPoint], "signatures": [TimeSignaturePoint]   // or nested in "map"
 *     "tolerance": 1e-9,
 *     "beats_to_seconds": [[beats, seconds], ...] | [{ "beats", "seconds" }, ...],
 *     "seconds_to_beats": [[seconds, beats], ...] | [{ "seconds", "beats" }, ...],
 *     "bpm_at": [[beats, bpm], ...] | [{ "beats", "bpm" }, ...],
 *     "bar_beat": [[beats, { bar, beat, fraction }], ...] | [{ "beats", "bar", "beat", "fraction" }, ...]
 *   }]
 * }
 */

import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { TempoPoint, TimeSignaturePoint } from "@/generated";
import { TempoMap } from "./tempoMap";

const PATH = resolve(import.meta.dirname, "../../../crates/ether-model/tests/tempo_vectors.json");
const exists = existsSync(PATH);

type Json = Record<string, unknown>;

function pairs(raw: unknown, a: string, b: string): Array<[number, unknown]> {
  if (!Array.isArray(raw)) return [];
  return raw.map((v) => (Array.isArray(v) ? [v[0] as number, v[1]] : [(v as Json)[a] as number, (v as Json)[b] ?? v]));
}

function points<T>(raw: unknown, kind: "tempo" | "sig"): T[] {
  if (!Array.isArray(raw)) return [];
  return raw.map((p: Json, i) => {
    const time = (p.time ?? p.beat ?? p.beats) as number;
    return (
      kind === "tempo"
        ? { id: String(p.id ?? `t${i}`), time, bpm: p.bpm, curve: p.curve ?? "Step" }
        : { id: String(p.id ?? `s${i}`), time, signature: p.signature }
    ) as T;
  });
}

describe.skipIf(!exists)("shared tempo vectors (ether-model)", () => {
  const file = exists ? (JSON.parse(readFileSync(PATH, "utf8")) as Json) : {};
  const cases = (Array.isArray(file) ? file : ((file.cases ?? file.vectors ?? []) as Json[])) as Json[];
  const defaultTol = (file.tolerance as number | undefined) ?? 1e-9;

  it("has cases", () => {
    expect(cases.length).toBeGreaterThan(0);
  });

  cases.forEach((c, i) => {
    const map = (c.map ?? c) as Json;
    const tol = (c.tolerance as number | undefined) ?? defaultTol;
    it(String(c.name ?? `case ${i}`), () => {
      const m = new TempoMap(
        points<TempoPoint>(map.tempo ?? map.tempo_points, "tempo"),
        points<TimeSignaturePoint>(map.signatures ?? map.time_signatures, "sig"),
      );
      for (const [beats, s] of pairs(c.beats_to_seconds, "beats", "seconds")) {
        expect(Math.abs(m.beatsToSeconds(beats) - (s as number)), `beats_to_seconds(${beats})`).toBeLessThanOrEqual(tol);
      }
      for (const [s, beats] of pairs(c.seconds_to_beats, "seconds", "beats")) {
        expect(Math.abs(m.secondsToBeats(s) - (beats as number)), `seconds_to_beats(${s})`).toBeLessThanOrEqual(tol);
      }
      for (const [beats, bpm] of pairs(c.bpm_at, "beats", "bpm")) {
        expect(Math.abs(m.bpmAt(beats) - (bpm as number)), `bpm_at(${beats})`).toBeLessThanOrEqual(tol);
      }
      for (const [beats, bb] of pairs(c.bar_beat, "beats", "bar_beat")) {
        const want = bb as { bar: number; beat: number; fraction: number };
        const got = m.barBeat(beats);
        expect([got.bar, got.beat], `bar_beat(${beats})`).toEqual([want.bar, want.beat]);
        expect(Math.abs(got.fraction - want.fraction)).toBeLessThanOrEqual(Math.max(tol, 1e-9));
      }
    });
  });
});
