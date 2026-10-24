import { act, fireEvent, screen, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { DeviceId, ShaperCurves } from "@/generated";
import { ContextMenuHost } from "@/kit";
import { useProjectStore } from "@/state";
import { playheadStore } from "@/state/playhead";
import { BUILTIN_DESCRIPTORS, cmd, newId } from "@/transport";
import { type MockTransport } from "@/transport/mock";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/devices/testUtils";
import { useGestureSender } from "../../gesture";
import { DeviceLayoutView } from "../DeviceLayoutView";
import { DEFAULT_SHAPER_CURVES } from "../lfoCurve";

const DESC = BUILTIN_DESCRIPTORS.Shaper;
const W = 200;
const H = 72;
const rect = { left: 0, top: 0, right: W, bottom: H, width: W, height: H, x: 0, y: 0, toJSON: () => ({}) };

let mock: MockTransport | undefined;
let devId: DeviceId = "";
beforeAll(() => {
  stubPointerCapture();
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(rect as DOMRect);
});
afterAll(() => vi.restoreAllMocks());
afterEach(() => {
  resetStores(mock);
  mock = undefined;
  playheadStore.reset();
});

function curves(): ShaperCurves {
  const k = store().project!.devices[devId]!.kind;
  if (k.type !== "Builtin" || k.device.type !== "Shaper") throw new Error("not a shaper");
  return k.device.curves;
}

function Panel() {
  const sender = useGestureSender();
  const device = useProjectStore((s) => s.project?.devices[devId]);
  if (!device) return null;
  return <DeviceLayoutView device={device} descriptor={DESC} sender={sender} />;
}

async function setup() {
  mock = await renderWithMock(
    <>
      <Panel />
      <ContextMenuHost />
    </>,
  );
  devId = newId();
  await act(async () => {
    await mock!.send(
      cmd("Device", {
        type: "Insert",
        id: devId,
        track: trackByName("Keys").id,
        device: { type: "Builtin", device: { type: "Shaper", curves: DEFAULT_SHAPER_CURVES } },
        before: null,
      }),
    );
  });
  await flush();
}

const svgOf = (lane: string) => within(screen.getByTestId(`widget-shaper-${lane}`)).getByRole("img") as unknown as SVGSVGElement;
const at = (x: number, y: number) => ({ clientX: x * W, clientY: (1 - y) * H });

async function press(svg: SVGSVGElement, x: number, y: number) {
  await act(async () => {
    fireEvent.pointerDown(svg, { button: 0, pointerId: 1, ...at(x, y) });
    fireEvent.pointerUp(svg, { pointerId: 1, ...at(x, y) });
  });
  await flush();
}

describe("ShaperCurveWidget", () => {
  it("shows one editor per lane; the lanes that are off are dimmed", async () => {
    await setup();
    for (const lane of ["volume", "cutoff", "pan"]) expect(screen.getByTestId(`widget-shaper-${lane}`)).toBeTruthy();
    expect(screen.getByTestId("widget-shaper-volume").className).not.toContain("--off");
    expect(screen.getByTestId("widget-shaper-pan").className).toContain("--off");
    expect(svgOf("pan").getAttribute("data-points")).toBe("3");
  });

  it("an edit changes only its lane, in one undo step", async () => {
    await setup();
    const pan = svgOf("pan");
    // The pan lane runs at 1/2 (two beats, eight sixteenths): x snaps to 1/8.
    await press(pan, 0.4, 0.9);
    await press(pan, 0.4, 0.9);
    expect(curves().pan.points.map((p) => p.x)).toEqual([0, 0.25, 0.375, 0.75]);
    expect(curves().volume).toEqual(DEFAULT_SHAPER_CURVES.volume);
    expect(curves().cutoff).toEqual(DEFAULT_SHAPER_CURVES.cutoff);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(curves()).toEqual(DEFAULT_SHAPER_CURVES);
  });

  it("each lane's playhead follows its own rate", async () => {
    await setup();
    // Beat 10.5: half way through a 1/4 cycle (volume), 1/8 through 1 bar (cutoff).
    act(() => playheadStore.setPlayhead({ transport: { position: 10.5, seconds: 5.25, playing: true, bpm: 120 } }));
    const x = (lane: string) => svgOf(lane).querySelector(".eth-lfo-curve__playhead")!.getAttribute("x1");
    expect(x("volume")).toBe((0.5 * W).toFixed(1));
    expect(x("cutoff")).toBe((((10.5 % 4) / 4) * W).toFixed(1));
  });
});
