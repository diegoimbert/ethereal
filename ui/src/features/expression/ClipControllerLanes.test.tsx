import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Command } from "@/generated";
import { pickOption } from "@/kit/testing";
import { ConfirmDialog, useConfirm } from "@/kit";
import { useEditorStore, useProjectStore, useSelectionStore } from "@/state";
import { createTimelineViewStore, itemSelection, type TimelineViewStore } from "@/timeline";
import { cmd, TransportProvider } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { AutomationLanes } from "@/features/automation";
import { ClipControllerLanes, EditedClipControllerLanes } from "./index";
import { compareKinds, controllerBadgeText } from "./badge";

// jsdom boxes sit at (0, 0): client coordinates are lane-local px. The view: 10 px/beat from
// song beat 0; the clip starts at song beat 16 (content beat 0 at x = 160).
const PX = 10;
const H = 56;
const CLIP = "01CONTROLLERTESTCLIP000000";
const LANE = "01CONTROLLERTESTLANE000000";

let mock: MockTransport | undefined;
let view: TimelineViewStore;
const store = () => useProjectStore.getState();
const lanes = () => Object.values(store().project!.expression_lanes);

afterEach(() => {
  mock?.dispose();
  mock = undefined;
  store().reset();
  useEditorStore.getState().close();
  itemSelection.getState().clear();
  useConfirm.getState().close();
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

async function stroke(el: Element, from: [number, number], to: [number, number]) {
  fireEvent.pointerDown(el, { button: 0, clientX: from[0], clientY: from[1] });
  for (let i = 1; i <= 8; i++) {
    fireEvent.pointerMove(window, { clientX: from[0] + ((to[0] - from[0]) * i) / 8, clientY: from[1] + ((to[1] - from[1]) * i) / 8 });
    await flush();
  }
  fireEvent.pointerUp(window, { clientX: to[0], clientY: to[1] });
  await flush();
}

/** The mock project with a MIDI clip at song beat 16 (8 beats), its lanes rendered. */
async function setup(opts: { sustain?: boolean } = {}) {
  mock = new MockTransport({ timers: "manual", seed: 1 });
  view = createTimelineViewStore({ pxPerBeat: PX, widthPx: 800 });
  const Probe = () => {
    const clip = useProjectStore((s) => s.project?.clips[CLIP]);
    return clip ? <ClipControllerLanes clip={clip} view={view} headerWidth={200} /> : null;
  };
  render(
    <TransportProvider transport={mock}>
      <Probe />
      <ConfirmDialog />
    </TransportProvider>,
  );
  await waitFor(() => expect(store().project).not.toBeNull());
  const track = Object.values(store().project!.tracks).find((t) => t.kind === "Midi")!;
  await send(cmd("Clip", { type: "CreateMidi", id: CLIP, track: track.id, start: 16, length: 8, name: "Keys" }));
  if (opts.sustain) {
    await send(cmd("Expression", { type: "CreateLane", id: LANE, clip: CLIP, kind: { type: "Cc", controller: 64 } }));
    await send(
      cmd("Expression", {
        type: "SetPoints",
        lane: LANE,
        points: [
          { time: 0, value: 0, curve: { type: "Step" } },
          { time: 1, value: 1, curve: { type: "Step" } },
          { time: 3, value: 0, curve: { type: "Step" } },
        ],
      }),
    );
  }
  await flush();
  return track;
}

const rows = () => screen.queryAllByTestId("expression-lane");

describe("the clip's controller lanes (automation tab)", () => {
  it("shows the controllers the clip holds, and nothing else, by default", async () => {
    await setup({ sustain: true });
    expect(rows()).toHaveLength(1);
    expect(rows()[0]!.getAttribute("data-kind")).toBe("cc 64 Sustain");
    expect(rows()[0]!.getAttribute("data-points")).toBe("3");
    // Drawn: a curve over the clip, the time outside the clip dimmed.
    expect(within(rows()[0]!.parentElement!).getByTestId("expression-curve")).toBeTruthy();
    expect(screen.getByTestId("clip-controller-lanes")).toHaveTextContent("clip controllers");
    expect(screen.getByTestId("clip-controller-lanes")).toHaveTextContent("Keys");
  });

  it("a clip without controller data says so", async () => {
    await setup();
    expect(rows()).toHaveLength(0);
    expect(screen.getByTestId("clip-controller-lanes")).toHaveTextContent("this clip has no controller data");
  });

  it("+ controller adds an empty lane; a stroke on the song timeline writes content-relative times", async () => {
    await setup();
    pickOption(screen.getByRole("combobox", { name: "add a controller lane" }), "pitch bend");
    expect(rows()).toHaveLength(1);
    expect(rows()[0]!.getAttribute("data-kind")).toBe("pitch bend");
    expect(lanes()).toEqual([]);
    // Song beats 17..19 = content beats 1..3.
    await stroke(rows()[0]!, [17 * PX, 4], [19 * PX, H - 4]);
    expect(lanes()).toHaveLength(1);
    const l = lanes()[0]!;
    expect(l).toMatchObject({ clip: CLIP, kind: { type: "PitchBend" } });
    expect(l.points[0]!.time).toBeCloseTo(1, 1);
    expect(l.points[l.points.length - 1]!.time).toBeCloseTo(3, 1);
    expect(l.points[0]!.value).toBeGreaterThan(0.8);
    expect(l.points[l.points.length - 1]!.value).toBeLessThan(-0.8);
    // One undo step: the stroke and the lane it created.
    await send(cmd("Edit", { type: "Undo" }));
    expect(lanes()).toEqual([]);
  });

  it("other cc… adds any controller", async () => {
    await setup();
    pickOption(screen.getByRole("combobox", { name: "add a controller lane" }), "other cc…");
    const field = screen.getByLabelText("controller number");
    fireEvent.change(field, { target: { value: "20" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await flush();
    fireEvent.click(screen.getByRole("button", { name: "add" }));
    expect(rows().map((r) => r.getAttribute("data-kind"))).toEqual(["cc 20"]);
  });

  it("removing a lane with points asks first", async () => {
    await setup({ sustain: true });
    fireEvent.click(screen.getByRole("button", { name: "remove cc 64 sustain lane" }));
    await flush();
    expect(lanes()).toHaveLength(1);
    fireEvent.click(await screen.findByRole("button", { name: "remove" }));
    await flush();
    expect(lanes()).toEqual([]);
    expect(rows()).toHaveLength(0);
  });
});

describe("in the automation tab", () => {
  it("the open MIDI clip's lanes sit above the track's automation", async () => {
    mock = new MockTransport({ timers: "manual", seed: 1 });
    render(
      <TransportProvider transport={mock}>
        <AutomationLanes clipLanes={(v, w) => <EditedClipControllerLanes view={v} headerWidth={w} />} />
      </TransportProvider>,
    );
    await waitFor(() => expect(store().project).not.toBeNull());
    const track = Object.values(store().project!.tracks).find((t) => t.kind === "Midi")!;
    await send(cmd("Clip", { type: "CreateMidi", id: CLIP, track: track.id, start: 0, length: 4, name: "Keys" }));
    await send(cmd("Expression", { type: "CreateLane", id: LANE, clip: CLIP, kind: { type: "Cc", controller: 64 } }));
    expect(screen.queryByTestId("clip-controller-lanes")).toBeNull();
    // Opening the clip (no track selected): its track's tab, with the clip's lanes.
    act(() => useSelectionStore.getState().selectTrack(null));
    act(() => useEditorStore.getState().openClip(CLIP));
    await flush();
    expect(screen.getByTestId("clip-controller-lanes")).toBeTruthy();
    expect(rows().map((r) => r.getAttribute("data-kind"))).toEqual(["cc 64 Sustain"]);
  });
});

describe("clip badge text", () => {
  it("names one or two controllers, counts more", () => {
    expect(controllerBadgeText([])).toBeNull();
    expect(controllerBadgeText([{ type: "Cc", controller: 64 }])).toBe("sustain");
    expect(controllerBadgeText([{ type: "Cc", controller: 64 }, { type: "PitchBend" }])).toBe("bend · sustain");
    expect(controllerBadgeText([{ type: "Cc", controller: 20 }])).toBe("cc 20");
    expect(controllerBadgeText([{ type: "Cc", controller: 1 }, { type: "Cc", controller: 64 }, { type: "ChannelPressure" }])).toBe("3 controllers");
  });

  it("orders pitch bend, pressure, then ccs by number", () => {
    const kinds = [
      { type: "Cc" as const, controller: 64 },
      { type: "ChannelPressure" as const },
      { type: "Cc" as const, controller: 1 },
      { type: "PitchBend" as const },
    ];
    expect(kinds.sort(compareKinds).map((k) => (k.type === "Cc" ? k.controller : k.type))).toEqual(["PitchBend", "ChannelPressure", 1, 64]);
  });
});
