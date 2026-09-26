import { describe, expect, it } from "vitest";
import type { TimeSignaturePoint } from "@/generated";
import {
  barPosition,
  clampBpm,
  formatBarPosition,
  formatCpu,
  formatSeconds,
  formatSignature,
  parseBpm,
  parseSignature,
} from "./format";

const sig = (time: number, numerator: number, denominator: number): TimeSignaturePoint => ({
  id: `ts${time}`,
  time,
  signature: { numerator, denominator },
});

const pos = (beats: number, points: TimeSignaturePoint[] = [sig(0, 4, 4)]) => formatBarPosition(barPosition(beats, points));

describe("barPosition", () => {
  it("counts bars, beats and sixteenths in 4/4", () => {
    expect(pos(0)).toBe("1.1.1");
    expect(pos(0.25)).toBe("1.1.2");
    expect(pos(1)).toBe("1.2.1");
    expect(pos(3.75)).toBe("1.4.4");
    expect(pos(4)).toBe("2.1.1");
    expect(pos(17.5)).toBe("5.2.3");
  });

  it("absorbs float error at grid lines", () => {
    expect(pos(3.9999999)).toBe("2.1.1");
    expect(pos(0.1 + 0.2 + 0.7)).toBe("1.2.1");
  });

  it("uses the denominator as beat unit", () => {
    const six8 = [sig(0, 6, 8)];
    expect(pos(0.5, six8)).toBe("1.2.1");
    expect(pos(2.75, six8)).toBe("1.6.2");
    expect(pos(3, six8)).toBe("2.1.1");
  });

  it("follows signature changes", () => {
    // 2 bars of 4/4 (8 beats), then 3/4.
    const map = [sig(8, 3, 4), sig(0, 4, 4)];
    expect(pos(7, map)).toBe("2.4.1");
    expect(pos(8, map)).toBe("3.1.1");
    expect(pos(11, map)).toBe("4.1.1");
    expect(pos(13.5, map)).toBe("4.3.3");
  });

  it("defaults to 4/4 and clamps negatives", () => {
    expect(pos(5, [])).toBe("2.2.1");
    expect(pos(-3)).toBe("1.1.1");
  });
});

describe("formatting and parsing", () => {
  it("formats seconds", () => {
    expect(formatSeconds(0)).toBe("0:00.000");
    expect(formatSeconds(65.25)).toBe("1:05.250");
    expect(formatSeconds(-1)).toBe("0:00.000");
  });

  it("parses tempo", () => {
    expect(parseBpm("128")).toBe(128);
    expect(parseBpm(" 97,5 ")).toBe(97.5);
    expect(parseBpm("120.123")).toBe(120.12);
    expect(parseBpm("10")).toBeNull();
    expect(parseBpm("1000")).toBeNull();
    expect(parseBpm("fast")).toBeNull();
    expect(parseBpm("")).toBeNull();
    expect(clampBpm(5)).toBe(20);
    expect(clampBpm(120.456)).toBe(120.46);
  });

  it("parses time signatures", () => {
    expect(parseSignature("7/8")).toEqual({ numerator: 7, denominator: 8 });
    expect(parseSignature(" 3 / 4 ")).toEqual({ numerator: 3, denominator: 4 });
    expect(parseSignature("4/3")).toBeNull();
    expect(parseSignature("0/4")).toBeNull();
    expect(parseSignature("33/4")).toBeNull();
    expect(parseSignature("4")).toBeNull();
    expect(formatSignature({ numerator: 6, denominator: 8 })).toBe("6/8");
  });

  it("formats CPU load", () => {
    expect(formatCpu(0.374)).toBe("37%");
    expect(formatCpu(2)).toBe("100%");
  });
});
