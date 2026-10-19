// Shared fixture of the WebGL lane specs (`webgl-lanes*.spec.ts`): automation lanes (linear,
// step, curve, a stepped DeviceEnabled lane, a selected point), and a group with child clips
// (its summary lane), built through the engine directly. The UI mirror is `window.__ether`.
import { expect, test, type Page } from "@playwright/test";
import type { Command, PointSpec } from "@/generated";
import { projectBackend } from "./clips";
import { headerButton } from "./ui";
import { doc, installProbe, sendBatched, ulid } from "./perfProject";

/** The backend this spec must run on: the project's (`canvas2d`: the fallback, else WebGL). */
export async function expectRenderer(page: Page): Promise<"webgl" | "canvas2d"> {
  const want = projectBackend(test.info().project.name);
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __ether: { arrangement?: { renderer(): string } } }).__ether.arrangement?.renderer() ?? null))
    .toBe(want);
  return want;
}

export interface LanesFixture {
  group: string;
  auto: string;
  volumeLane: string;
  enabledLane: string;
  /** Volume points, by time. */
  points: string[];
}

export async function buildLanesFixture(page: Page): Promise<LanesFixture> {
  await installProbe(page);
  const group = ulid(8, 0);
  const auto = ulid(8, 1);
  const kids = [ulid(8, 2), ulid(8, 3)];
  const cmds: Command[] = [
    { domain: "Track", command: { type: "Create", id: auto, kind: "Midi", name: "Automated", color: null, parent: null, before: null } },
    { domain: "Track", command: { type: "Create", id: group, kind: "Group", name: "group", color: null, parent: null, before: null } },
    ...kids.map((id, i): Command => ({
      domain: "Track",
      command: { type: "Create", id, kind: "Midi", name: `Child ${i}`, color: null, parent: group, before: null },
    })),
    { domain: "Device", command: { type: "Insert", id: ulid(8, 9), track: auto, device: { type: "Builtin", device: { type: "Synth" } }, before: null } },
  ];
  await sendBatched(page, cmds);
  const clips: Command[] = [];
  const placed: Array<[number, number, number]> = [
    [0, 0, 4],
    [0, 6, 2],
    [1, 3, 4],
    [1, 12, 3],
    [0, 20, 1],
  ];
  placed.forEach(([k, start, length], i) =>
    clips.push({ domain: "Clip", command: { type: "CreateMidi", id: ulid(9, i), track: kids[k]!, start, length, name: `C${i}` } }),
  );
  clips.push({ domain: "Clip", command: { type: "CreateMidi", id: ulid(9, 9), track: auto, start: 2, length: 4, name: "Keys" } });
  await sendBatched(page, clips);

  const volumeLane = ulid(0, 0);
  const enabledLane = ulid(0, 1);
  const synth = (await doc(page)).devices[ulid(8, 9)];
  const spec = (n: number, time: number, value: number, curve: PointSpec["curve"]): PointSpec => ({ id: ulid(2, n), time, value, curve });
  const volume = [
    spec(0, 1, 0.3, { type: "Linear" }),
    spec(1, 3, 0.8, { type: "Step" }),
    spec(2, 5, 0.2, { type: "Curve", tension: 0.6 }),
    spec(3, 9, 0.9, { type: "Curve", tension: -0.5 }),
    spec(4, 12, 0.5, { type: "Linear" }),
    spec(5, 14, 0.65, { type: "Linear" }),
  ];
  const lanes: Command[] = [
    {
      domain: "Automation",
      command: { type: "CreateLane", id: volumeLane, owner: { type: "Track", track: auto }, target: { type: "TrackVolume", track: auto } },
    },
    { domain: "Automation", command: { type: "AddPoints", lane: volumeLane, points: volume } },
  ];
  if (synth) {
    lanes.push(
      {
        domain: "Automation",
        command: { type: "CreateLane", id: enabledLane, owner: { type: "Track", track: auto }, target: { type: "DeviceEnabled", device: synth.id } },
      },
      {
        domain: "Automation",
        command: {
          type: "AddPoints",
          lane: enabledLane,
          points: [spec(20, 2, 1, { type: "Step" }), spec(21, 6, 0, { type: "Step" }), spec(22, 10, 1, { type: "Step" })],
        },
      },
    );
  }
  await sendBatched(page, lanes);
  await expect.poll(async () => Object.keys((await doc(page)).automation_points).length).toBeGreaterThanOrEqual(volume.length);
  return { group, auto, volumeLane, enabledLane, points: volume.map((p) => p.id) };
}

/** Opens the automation lanes of the fixture's automated track (its existing lanes show). */
export async function openAutomation(page: Page): Promise<void> {
  await (await headerButton(page, "Automated", "show automation of Automated")).click();
  await expect(page.getByRole("group", { name: "volume automation" })).toBeVisible();
}

/** Client rect of an automation point (through the arranger test API). */
export async function automationPoint(page: Page, id: string): Promise<{ x: number; y: number }> {
  // Where it is at rest: the same over a few frames (lanes animate open and closed).
  let p: { x: number; y: number } | null = null;
  await expect
    .poll(async () => {
      const seen = await page.evaluate(
        (id) =>
          new Promise<Array<{ x: number; y: number } | null>>((done) => {
            const api = (window as unknown as { __ether: { arrangement: { automationPoint(id: string): { x: number; y: number } | null } } }).__ether
              .arrangement;
            const out: Array<{ x: number; y: number } | null> = [];
            const tick = () => {
              out.push(api.automationPoint(id));
              if (out.length < 4) requestAnimationFrame(tick);
              else done(out);
            };
            requestAnimationFrame(tick);
          }),
        id,
      );
      p = seen[0] ?? null;
      return p !== null && seen.every((s) => s && Math.abs(s.x - p!.x) < 0.01 && Math.abs(s.y - p!.y) < 0.01);
    })
    .toBe(true);
  return p!;
}
