import { beforeEach, describe, expect, it } from "vitest";
import type { AutomationPoint, Project } from "@/generated";
import { createDemoProject } from "@/transport";
import { curveFraction, type CurvePoint } from "./curve";
import { addPointCommand, bendTension, moveEdits, removePointsCommand, setCurveCommand, TENSION_DRAG_PX } from "./edit";
import { LANE_PAD, lanePath, pointRect, segmentAt, timeToX, valueToY, xToTime, yToValue, type LaneGeometry } from "./geometry";
import { formatNormalized, PAN_INFO, targetKey, trackTargets, VOLUME_INFO } from "./params";
import {
  AUTOMATION_BAR_HEIGHT,
  automationHeight,
  automationLaneHeights,
  LANE_HEIGHT,
  resetAutomationUi,
  shownKeys,
  useAutomationUi,
} from "./uiStore";

const g: LaneGeometry = { vp: { pxPerBeat: 10, scrollBeats: 2 }, height: 110 };

const pt = (id: string, time: number, value: number, curve: AutomationPoint["curve"] = { type: "Linear" }): AutomationPoint => ({
  id,
  lane: "L",
  time,
  value,
  curve,
});

describe("geometry", () => {
  it("maps values to y inside the padding and back", () => {
    expect(valueToY(1, 110)).toBe(LANE_PAD);
    expect(valueToY(0, 110)).toBe(110 - LANE_PAD);
    expect(yToValue(valueToY(0.3, 110), 110)).toBeCloseTo(0.3, 12);
    expect(yToValue(-50, 110)).toBe(1);
    expect(yToValue(500, 110)).toBe(0);
  });

  it("maps lane time through the viewport and offset", () => {
    expect(timeToX(4, g)).toBe(20);
    expect(xToTime(20, g)).toBe(4);
    const clipLane = { ...g, offset: 8 };
    expect(timeToX(0, clipLane)).toBe(60);
    expect(xToTime(60, clipLane)).toBe(0);
  });

  it("draws linear, step and curve segments with the engine formula", () => {
    const pts: CurvePoint[] = [
      { time: 2, value: 0, curve: { type: "Step" } },
      { time: 4, value: 1, curve: { type: "Curve", tension: 1 } },
      { time: 6, value: 0, curve: { type: "Linear" } },
    ];
    const d = lanePath(pts, g, 0, 100, 1);
    const coords = [...d.matchAll(/[ML](-?[\d.]+),(-?[\d.]+)/g)].map((m) => [Number(m[1]), Number(m[2])] as const);
    // Starts at the left edge holding the first value, ends at the right edge holding the last.
    expect(coords[0]).toEqual([0, valueToY(0, 110)]);
    expect(coords.at(-1)).toEqual([100, valueToY(0, 110)]);
    // Step: holds 0 until x=20 then jumps to 1.
    expect(d).toContain(`L20,${valueToY(0, 110)}L20,${valueToY(1, 110)}`);
    // Curve from (20, v=1) to (40, v=0): every sample follows 1 - x^(4^1).
    const curve = coords.filter(([x]) => x > 20 && x < 40);
    expect(curve.length).toBeGreaterThan(10);
    for (const [x, y] of curve) {
      const v = 1 - curveFraction((x - 20) / 20, 1);
      expect(Math.abs(y - valueToY(v, 110))).toBeLessThan(0.01);
    }
  });

  it("returns an empty path without points and hit-tests points", () => {
    expect(lanePath([], g, 0, 100)).toBe("");
    const r = pointRect({ time: 4, value: 1, curve: { type: "Linear" } }, g, 4);
    expect(r).toEqual({ x0: 16, y0: LANE_PAD - 4, x1: 24, y1: LANE_PAD + 4 });
    const pts = [pt("a", 0, 0), pt("b", 4, 1), pt("c", 8, 0)];
    expect(segmentAt(pts, 5)).toBe(1);
    expect(segmentAt(pts, 9)).toBe(-1);
  });
});

describe("edit commands", () => {
  it("creates the lane with the first point in one batch", () => {
    const c = addPointCommand(null, { type: "Track", track: "T" }, { type: "TrackVolume", track: "T" }, { id: "p", time: 1, value: 0.5, curve: { type: "Linear" } }, "NEW");
    expect(c.domain).toBe("Edit");
    expect(c.command).toMatchObject({
      type: "Batch",
      commands: [
        { domain: "Automation", command: { type: "CreateLane", id: "NEW" } },
        { domain: "Automation", command: { type: "AddPoints", lane: "NEW" } },
      ],
    });
    const lane = { id: "L", owner: { type: "Track" as const, track: "T" }, target: { type: "TrackVolume" as const, track: "T" }, enabled: true };
    expect(addPointCommand(lane, lane.owner, lane.target, { id: "p", time: 1, value: 0.5, curve: { type: "Linear" } }, "x").command).toMatchObject({
      type: "AddPoints",
      lane: "L",
    });
  });

  it("moves a selection as a group, snapped on the anchor and clamped", () => {
    const a = pt("a", 1, 0.2);
    const b = pt("b", 3, 0.9);
    const snap = (t: number) => Math.round(t);
    const edits = moveEdits([a, b], a, 1.4, 0.05, snap);
    expect(edits).toEqual([
      { id: "a", time: 2, value: expect.closeTo(0.25, 12), curve: null },
      { id: "b", time: 4, value: expect.closeTo(0.95, 12), curve: null },
    ]);
    // Value clamped by the highest point, time by the earliest.
    const clamped = moveEdits([a, b], b, -5, 0.5, null);
    expect(clamped[0]).toMatchObject({ time: 0, value: expect.closeTo(0.3, 12) });
    expect(clamped[1]).toMatchObject({ time: 2, value: 1 });
    // Axis locks.
    expect(moveEdits([a], a, 2, 0.3, null, { lockTime: true })[0]).toEqual({ id: "a", time: null, value: 0.5, curve: null });
    expect(moveEdits([a], a, 2, 0.3, null, { lockValue: true })[0]).toEqual({ id: "a", time: 3, value: null, curve: null });
  });

  it("builds remove and curve edits", () => {
    expect(removePointsCommand([])).toBeNull();
    expect(removePointsCommand(["a"])?.command).toEqual({ type: "RemovePoints", ids: ["a"] });
    expect(setCurveCommand(["a"], { type: "Step" })?.command).toEqual({
      type: "EditPoints",
      edits: [{ id: "a", time: null, value: null, curve: { type: "Step" } }],
    });
  });

  it("bends: dragging up bulges up, clamped to -1..1", () => {
    expect(bendTension(0, TENSION_DRAG_PX / 4, true)).toBe(-0.5);
    expect(bendTension(0, TENSION_DRAG_PX / 4, false)).toBe(0.5);
    expect(bendTension(0.5, -TENSION_DRAG_PX, true)).toBe(1);
  });
});

describe("params", () => {
  let project: Project;
  beforeEach(() => {
    project = createDemoProject();
  });

  it("lists mixer targets and loaded device params", () => {
    const track = Object.values(project.tracks).find((t) => Object.values(project.sends).some((s) => s.from === t.id))!;
    const device = Object.values(project.devices)[0];
    const descriptors = new Map();
    if (device && device.track === track.id) {
      descriptors.set(device.id, {
        device_type: { type: "Builtin", device: "Synth" },
        name: "Synth",
        category: "Instrument",
        params: [
          { ...PAN_INFO, id: 7, name: "Cutoff", automatable: true },
          { ...PAN_INFO, id: 8, name: "Hidden", hidden: true },
        ],
        audio_inputs: 0,
        audio_outputs: 2,
        midi_input: true,
      });
    }
    const list = trackTargets(project, track.id, descriptors);
    expect(list[0]).toMatchObject({ key: `volume:${track.id}`, name: "Volume" });
    expect(list[1]).toMatchObject({ key: `pan:${track.id}`, name: "Pan" });
    expect(list.some((t) => t.target.type === "SendLevel")).toBe(true);
    expect(list.some((t) => t.info.name === "Hidden")).toBe(false);
  });

  it("formats normalized values through ParamInfo.scale", () => {
    expect(formatNormalized(VOLUME_INFO, 0)).toBe("-inf dB");
    // Fader law: 1.0 is the top of the range.
    expect(formatNormalized(VOLUME_INFO, 1)).toBe("6.0 dB");
    expect(formatNormalized(PAN_INFO, 0.5)).toBe("C");
    expect(formatNormalized(PAN_INFO, 0)).toBe("100L");
    expect(targetKey({ type: "DeviceParam", device: "D", param: 3 })).toBe("param:D:3");
  });
});

describe("ui store heights", () => {
  beforeEach(() => resetAutomationUi());

  it("is the bar when closed and grows by a lane per shown parameter", () => {
    const s = () => useAutomationUi.getState();
    expect(automationHeight(s(), "T")).toBe(AUTOMATION_BAR_HEIGHT);
    s().setOpen("T", true, ["volume:T"]);
    expect(automationLaneHeights("T")).toBe(AUTOMATION_BAR_HEIGHT + LANE_HEIGHT);
    s().show("T", "pan:T");
    s().show("T", "pan:T");
    expect(shownKeys(s(), "T")).toEqual(["volume:T", "pan:T"]);
    s().replace("T", "volume:T", "send:S");
    expect(shownKeys(s(), "T")).toEqual(["send:S", "pan:T"]);
    s().hide("T", "send:S");
    expect(automationHeight(s(), "T")).toBe(AUTOMATION_BAR_HEIGHT + LANE_HEIGHT);
    // Closing keeps the list for next time.
    s().setOpen("T", false);
    expect(automationHeight(s(), "T")).toBe(AUTOMATION_BAR_HEIGHT);
    s().setOpen("T", true, ["ignored"]);
    expect(shownKeys(s(), "T")).toEqual(["pan:T"]);
  });
});
