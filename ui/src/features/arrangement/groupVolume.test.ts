import { describe, expect, it } from "vitest";
import type { TrackId } from "@/generated";
import { SILENCE_DB } from "@/features/devices/paramScale";
import { MAX_DB } from "@/features/mixer/routing";
import { groupVolumes } from "./groupVolume";

const a = "a" as TrackId;
const b = "b" as TrackId;
const c = "c" as TrackId;

describe("groupVolumes", () => {
  it("moves every selected track by the same dB amount, keeping the balance", () => {
    const out = groupVolumes(
      new Map([
        [a, 0],
        [b, -6],
        [c, -12],
      ]),
      a,
      -3,
    );
    expect(out.get(a)).toBe(-3);
    expect(out.get(b)).toBe(-9);
    expect(out.get(c)).toBe(-15);
  });

  it("clamps each track to the fader's range without changing the others", () => {
    const out = groupVolumes(
      new Map([
        [a, 0],
        [b, 4],
      ]),
      a,
      5,
    );
    expect(out.get(a)).toBe(5);
    expect(out.get(b)).toBe(MAX_DB);
  });

  it("keeps silent tracks silent", () => {
    const out = groupVolumes(
      new Map([
        [a, 0],
        [b, SILENCE_DB],
      ]),
      a,
      -6,
    );
    expect(out.get(b)).toBe(SILENCE_DB);
  });

  it("moves faders by the same distance when the dragged track starts silent", () => {
    const out = groupVolumes(
      new Map([
        [a, SILENCE_DB],
        [b, -6],
      ]),
      a,
      -20,
    );
    expect(out.get(a)).toBe(-20);
    expect(out.get(b)!).toBeGreaterThan(-6);
  });
});
