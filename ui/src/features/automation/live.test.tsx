import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { AutomationLane, AutomationPoint, AutomationTarget, Clip, Device, Project } from "@/generated";
import { playheadStore, useProjectStore } from "@/state";
import { createDemoProject } from "@/transport/mock/demoProject";
import { hasEnabledLane, liveValueAt, useDeviceOn, useLiveAutomation } from "./live";

const STEP = { type: "Step" } as const;
const LINEAR = { type: "Linear" } as const;

let n = 0;
const id = (prefix: string) => `${prefix}${String(++n).padStart(6, "0")}`;

/** The demo project with its own automation removed. */
function base(): Project {
  const p = structuredClone(createDemoProject());
  p.automation_lanes = {};
  p.automation_points = {};
  return p;
}

function firstDevice(p: Project): Device {
  return Object.values(p.devices)[0]!;
}

function addLane(
  p: Project,
  owner: AutomationLane["owner"],
  target: AutomationTarget,
  points: Array<[number, number, AutomationPoint["curve"]?]>,
  enabled = true,
): AutomationLane {
  const lane: AutomationLane = { id: id("lane"), owner, target, enabled };
  p.automation_lanes[lane.id] = lane;
  for (const [time, value, curve] of points) {
    const pt: AutomationPoint = { id: id("pt"), lane: lane.id, time, value, curve: curve ?? LINEAR };
    p.automation_points[pt.id] = pt;
  }
  return lane;
}

function addClip(p: Project, track: string, start: number, length: number, extra: Partial<Clip> = {}): Clip {
  const template = Object.values(p.clips)[0]!;
  const clip: Clip = {
    ...structuredClone(template),
    id: id("clip"),
    track,
    start,
    length,
    offset: 0,
    muted: false,
    looping: { enabled: false, start: 0, end: 0 },
    ...extra,
  };
  delete clip.lane; // main lane
  p.clips[clip.id] = clip;
  return clip;
}

afterEach(() => {
  playheadStore.reset();
  useProjectStore.getState().reset();
});

describe("liveValueAt", () => {
  it("evaluates the arrangement lane like the engine (linear, step, before/after)", () => {
    const p = base();
    const d = firstDevice(p);
    const target: AutomationTarget = { type: "DeviceParam", device: d.id, param: 0 };
    expect(liveValueAt(base(), target, 0)).toBeNull();
    addLane(p, { type: "Track", track: d.track }, target, [
      [4, 0.2],
      [8, 0.6, STEP],
      [12, 1],
    ]);
    expect(liveValueAt(p, target, 0)).toBe(0.2);
    expect(liveValueAt(p, target, 6)).toBeCloseTo(0.4);
    expect(liveValueAt(p, target, 10)).toBe(0.6);
    expect(liveValueAt(p, target, 20)).toBe(1);
  });

  it("ignores disabled lanes and lanes without points", () => {
    const p = base();
    const d = firstDevice(p);
    const target: AutomationTarget = { type: "DeviceEnabled", device: d.id };
    addLane(p, { type: "Track", track: d.track }, target, [[0, 0]], false);
    expect(liveValueAt(p, target, 1)).toBeNull();
    const p2 = base();
    addLane(p2, { type: "Track", track: d.track }, target, []);
    expect(liveValueAt(p2, target, 1)).toBeNull();
    expect(hasEnabledLane(p2, target)).toBe(true);
  });

  it("an imported on/off envelope with steps at the same beat (Ableton BoolEvents)", () => {
    // AWIP 115, Track 24 Auto Filter: Manual off, automated on 0–44.5 and 80.5–108.5.
    const p = base();
    const d = firstDevice(p);
    const target: AutomationTarget = { type: "DeviceEnabled", device: d.id };
    addLane(p, { type: "Track", track: d.track }, target, [
      [0, 1, STEP],
      [44.5, 1, STEP],
      [44.5, 0, STEP],
      [80.5, 0, STEP],
      [80.5, 1, STEP],
      [108.5, 1, STEP],
      [108.5, 0, STEP],
    ]);
    expect(liveValueAt(p, target, 10)).toBe(1);
    expect(liveValueAt(p, target, 50)).toBe(0);
    expect(liveValueAt(p, target, 90)).toBe(1);
    expect(liveValueAt(p, target, 120)).toBe(0);
  });

  it("a clip envelope overrides the lane where the clip plays (not when muted)", () => {
    const p = base();
    const d = firstDevice(p);
    const target: AutomationTarget = { type: "DeviceParam", device: d.id, param: 0 };
    addLane(p, { type: "Track", track: d.track }, target, [[0, 0.1]]);
    const clip = addClip(p, d.track, 100, 8, { looping: { enabled: true, start: 0, end: 4 } });
    addLane(p, { type: "Clip", clip: clip.id }, target, [
      [0, 0],
      [4, 1],
    ]);
    expect(liveValueAt(p, target, 99)).toBe(0.1);
    expect(liveValueAt(p, target, 102)).toBeCloseTo(0.5);
    // Looped: content beat 2 again at timeline 106.
    expect(liveValueAt(p, target, 106)).toBeCloseTo(0.5);
    expect(liveValueAt(p, target, 109)).toBe(0.1);
    // A patch replaces the table (the index is cached per table).
    p.clips = { ...p.clips, [clip.id]: { ...clip, muted: true } };
    expect(liveValueAt(p, target, 102)).toBe(0.1);
  });
});

function Probe({ target }: { target: AutomationTarget }) {
  const v = useLiveAutomation(target);
  return <span data-testid="v">{v === null ? "none" : v.toFixed(2)}</span>;
}

function Power({ device }: { device: Device }) {
  const { on, automated } = useDeviceOn(device);
  return <span data-testid="on">{`${on ? "on" : "off"}${automated ? " (auto)" : ""}`}</span>;
}

const frame = (position: number) => ({ transport: { position, seconds: 0, playing: true, bpm: 120 } });

describe("useLiveAutomation", () => {
  it("follows the playhead and re-renders when the value changes", () => {
    const p = base();
    const d = firstDevice(p);
    const target: AutomationTarget = { type: "DeviceParam", device: d.id, param: 0 };
    addLane(p, { type: "Track", track: d.track }, target, [
      [0, 0],
      [4, 1],
    ]);
    useProjectStore.getState().loadProject(p);
    render(<Probe target={target} />);
    expect(screen.getByTestId("v").textContent).toBe("0.00");
    act(() => playheadStore.setPlayhead(frame(1)));
    expect(screen.getByTestId("v").textContent).toBe("0.25");
    act(() => playheadStore.setPlayhead(frame(3)));
    expect(screen.getByTestId("v").textContent).toBe("0.75");
  });

  it("a device switched off in the document runs where its automation turns it on", () => {
    const p = base();
    const d = { ...firstDevice(p), enabled: false };
    p.devices[d.id] = d;
    addLane(p, { type: "Track", track: d.track }, { type: "DeviceEnabled", device: d.id }, [
      [0, 1, STEP],
      [8, 0, STEP],
    ]);
    useProjectStore.getState().loadProject(p);
    render(<Power device={d} />);
    act(() => playheadStore.setPlayhead(frame(2)));
    expect(screen.getByTestId("on").textContent).toBe("on (auto)");
    act(() => playheadStore.setPlayhead(frame(9)));
    expect(screen.getByTestId("on").textContent).toBe("off (auto)");
  });
});
