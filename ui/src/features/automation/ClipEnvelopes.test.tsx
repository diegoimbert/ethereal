import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { AutomationLane, AutomationPoint, Command } from "@/generated";
import { pickOption } from "@/kit/testing";
import { useEditorStore, useProjectStore } from "@/state";
import { itemSelection } from "@/timeline";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { PianoRoll } from "@/features/piano-roll";
import { clipEnvelopeOptions, ENVELOPE_PREFIX, envelopeKeyOf } from "./clipEnvelopeTargets";
import { PAN_INFO, VOLUME_INFO, type TargetInfo } from "./params";

// The piano roll's view starts at 40 px/beat, scrolled to content beat 0; jsdom boxes are at
// (0, 0), so client coordinates are lane-local px.
const PX = 40;

let mock: MockTransport | undefined;
const store = () => useProjectStore.getState();
const clipLanes = (): AutomationLane[] => Object.values(store().project!.automation_lanes).filter((l) => l.owner.type === "Clip");
const pointsOf = (lane: string): AutomationPoint[] =>
  Object.values(store().project!.automation_points)
    .filter((p) => p.lane === lane)
    .sort((a, b) => a.time - b.time);

afterEach(() => {
  mock?.dispose();
  mock = undefined;
  store().reset();
  useEditorStore.getState().close();
  itemSelection.getState().clear();
});

async function flush() {
  await act(async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  });
}

async function send(command: Command) {
  await act(async () => {
    await mock!.send(command);
  });
}

const CLIP = "01CLIPENVELOPETESTCLIP0000";

async function setup(): Promise<string> {
  mock = new MockTransport({ timers: "manual", seed: 1 });
  render(
    <TransportProvider transport={mock}>
      <PianoRoll />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  const track = Object.values(store().project!.tracks).find((t) => t.kind === "Midi")!;
  await send(cmd("Clip", { type: "CreateMidi", id: CLIP, track: track.id, start: 64, length: 8, name: "Env" }));
  act(() => useEditorStore.getState().openClip(CLIP));
  await flush();
  return track.id;
}

const picker = () => screen.getByRole("combobox", { name: "lane" });

describe("clip envelopes in the piano roll", () => {
  it("drawing a Volume envelope creates a clip-owned lane in content time", async () => {
    const track = await setup();
    pickOption(picker(), { value: `${ENVELOPE_PREFIX}volume:${track}` });
    await flush();
    const env = screen.getByTestId("clip-envelope");
    expect(env.getAttribute("data-target")).toBe(`volume:${track}`);
    // No lane until the first point.
    expect(clipLanes()).toEqual([]);
    expect(screen.queryByTestId("clip-envelope-enabled")).toBeNull();

    const svg = within(env).getByTestId("automation-lane-svg");
    // A double-click: the lane adds the point on the second press.
    for (let i = 0; i < 2; i++) {
      await act(async () => {
        fireEvent.pointerDown(svg, { button: 0, clientX: 2 * PX, clientY: 20 });
      });
      await act(async () => {
        fireEvent.pointerUp(window, { clientX: 2 * PX, clientY: 20 });
      });
    }
    await flush();
    const [lane] = clipLanes();
    expect(lane).toBeDefined();
    expect(lane!.owner).toEqual({ type: "Clip", clip: CLIP });
    expect(lane!.target).toEqual({ type: "TrackVolume", track });
    const pts = pointsOf(lane!.id);
    expect(pts).toHaveLength(1);
    // Content-relative: beat 2 of the clip, not of the song (the clip starts at 64).
    expect(pts[0]!.time).toBeCloseTo(2, 6);

    // The picker marks it as used, in its own group.
    fireEvent.click(picker());
    const used = screen.getByRole("option", { name: "volume •" });
    expect(used.getAttribute("data-value")).toBe(`${ENVELOPE_PREFIX}volume:${track}`);
    fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });

    // Off / on, then delete.
    fireEvent.click(screen.getByTestId("clip-envelope-enabled"));
    await flush();
    expect(clipLanes()[0]!.enabled).toBe(false);
    fireEvent.click(screen.getByTestId("clip-envelope-delete"));
    await flush();
    expect(clipLanes()).toEqual([]);
    expect(pointsOf(lane!.id)).toEqual([]);
  });

  it("clip gain / transposition envelopes are for audio clips only", async () => {
    await setup();
    const create = (target: { type: "ClipGain" } | { type: "ClipTranspose" }) =>
      act(async () => {
        await mock!.send(
          cmd("Automation", {
            type: "CreateLane",
            id: `01CLIPENVELOPETESTLANE${target.type === "ClipGain" ? "0001" : "0002"}`,
            owner: { type: "Clip", clip: CLIP },
            target,
          }),
        );
      });
    await expect(create({ type: "ClipGain" })).rejects.toMatchObject({ code: "InvalidArgument" });
    await expect(create({ type: "ClipTranspose" })).rejects.toMatchObject({ code: "InvalidArgument" });
    expect(clipLanes()).toEqual([]);
    // The MIDI clip's picker offers no clip targets.
    fireEvent.click(picker());
    expect(screen.queryByRole("option", { name: "clip gain" })).toBeNull();
    expect(screen.queryByRole("option", { name: "Transposition" })).toBeNull();
  });

  it("a deleted clip takes its envelopes along", async () => {
    const track = await setup();
    await send(
      cmd("Automation", { type: "CreateLane", id: "01CLIPENVELOPETESTLANE0000", owner: { type: "Clip", clip: CLIP }, target: { type: "TrackPan", track } }),
    );
    expect(clipLanes()).toHaveLength(1);
    await send(cmd("Clip", { type: "Delete", ids: [CLIP] }));
    expect(clipLanes()).toEqual([]);
  });
});

describe("clipEnvelopeOptions", () => {
  const t = (key: string, name: string, group: string): TargetInfo => ({
    key,
    name,
    group,
    target: { type: "TrackVolume", track: "t" },
    info: key.startsWith("pan") ? PAN_INFO : VOLUME_INFO,
  });
  const targets = [t("volume:t", "Volume", "Mixer"), t("pan:t", "Pan", "Mixer"), t("param:d:1", "Synth: Cutoff", "Synth")];

  it("groups by device and lists envelopes holding points first", () => {
    const lane = { id: "L", owner: { type: "Clip", clip: "c" }, target: { type: "TrackPan", track: "t" }, enabled: true } as AutomationLane;
    const opts = clipEnvelopeOptions(targets, new Map([["pan:t", lane]]), new Set(["L"]));
    expect(opts.map((o) => [o.value, o.label, o.group])).toEqual([
      ["env:pan:t", "Pan •", "envelopes in this clip"],
      ["env:volume:t", "Volume", "envelope · Mixer"],
      ["env:param:d:1", "Synth: Cutoff", "envelope · Synth"],
    ]);
  });

  it("an empty lane is not marked", () => {
    const lane = { id: "L", owner: { type: "Clip", clip: "c" }, target: { type: "TrackPan", track: "t" }, enabled: true } as AutomationLane;
    expect(clipEnvelopeOptions(targets, new Map([["pan:t", lane]]), new Set()).find((o) => o.value === "env:pan:t")!.label).toBe("Pan");
  });

  it("envelopeKeyOf", () => {
    expect(envelopeKeyOf("env:volume:t")).toBe("volume:t");
    expect(envelopeKeyOf("velocity")).toBeNull();
  });
});
