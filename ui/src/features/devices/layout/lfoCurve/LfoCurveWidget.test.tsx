import { act, fireEvent, screen, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { DeviceId, LfoCurve } from "@/generated";
import { ContextMenuHost } from "@/kit";
import { useProjectStore } from "@/state";
import { playheadStore } from "@/state/playhead";
import { BUILTIN_DESCRIPTORS, cmd, newId } from "@/transport";
import { type MockTransport } from "@/transport/mock";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/devices/testUtils";
import { useGestureSender } from "../../gesture";
import { DeviceLayoutView } from "../DeviceLayoutView";
import { DEFAULT_LFO_CURVE } from "./lfoCurveMath";

const DESC = BUILTIN_DESCRIPTORS.VolumeLfo;
// The plot is 200 × 72 px (the box fallback and the stubbed rect).
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

function curve(): LfoCurve {
  const k = store().project!.devices[devId]!.kind;
  if (k.type !== "Builtin" || k.device.type !== "VolumeLfo") throw new Error("not a volume lfo");
  return k.device.curve;
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
        device: { type: "Builtin", device: { type: "VolumeLfo", curve: DEFAULT_LFO_CURVE } },
        before: null,
      }),
    );
  });
  await flush();
  return within(screen.getByTestId("widget-lfo-curve")).getByRole("img") as unknown as SVGSVGElement;
}

const at = (x: number, y: number) => ({ clientX: x * W, clientY: (1 - y) * H });

async function press(svg: SVGSVGElement, x: number, y: number, mods: { altKey?: boolean } = {}) {
  await act(async () => {
    fireEvent.pointerDown(svg, { button: 0, pointerId: 1, ...at(x, y), ...mods });
    fireEvent.pointerUp(svg, { pointerId: 1, ...at(x, y) });
  });
  await flush();
}

async function drag(svg: SVGSVGElement, from: [number, number], to: [number, number], mods: { altKey?: boolean } = {}) {
  await act(async () => {
    fireEvent.pointerDown(svg, { button: 0, pointerId: 1, ...at(...from), ...mods });
  });
  await act(async () => {
    fireEvent.pointerMove(svg, { pointerId: 1, ...at(...to), ...mods });
  });
  await act(async () => {
    fireEvent.pointerUp(svg, { pointerId: 1, ...at(...to) });
  });
  await flush();
}

const undo = () => act(() => mock!.send(cmd("Edit", { type: "Undo" })));

describe("LfoCurveWidget", () => {
  it("draws the curve and one handle per point", async () => {
    const svg = await setup();
    expect(svg.getAttribute("data-points")).toBe("3");
    expect(svg.querySelectorAll("[data-point]")).toHaveLength(3);
    expect(svg.querySelector('[data-testid="lfo-curve-line"]')!.getAttribute("points")).toMatch(/^0\.0,0\.0 /);
  });

  it("double-click adds a point on the grid, in one undo step", async () => {
    const svg = await setup();
    await press(svg, 0.27, 0.5);
    expect(curve().points).toHaveLength(3);
    await press(svg, 0.27, 0.5);
    // 1/4 cycle = 4 sixteenths: x snaps to 0.25.
    expect(curve().points.map((p) => p.x)).toEqual([0, 0.02, 0.25, 0.6]);
    expect(curve().points[2]!.y).toBeCloseTo(0.5, 5);
    await undo();
    expect(curve()).toEqual(DEFAULT_LFO_CURVE);
  });

  it("drags a point on the grid, alt = no snap, one undo step", async () => {
    const svg = await setup();
    await drag(svg, [0.6, 1], [0.8, 0.9]);
    expect(curve().points[2]!.x).toBe(0.75);
    expect(curve().points[2]!.y).toBeCloseTo(0.9, 5);
    await drag(svg, [0.75, 0.9], [0.83, 0.9], { altKey: true });
    expect(curve().points[2]!.x).toBeCloseTo(0.83, 5);
    await undo();
    expect(curve().points[2]!.x).toBe(0.75);
    await undo();
    expect(curve()).toEqual(DEFAULT_LFO_CURVE);
  });

  it("double-click on a point deletes it", async () => {
    const svg = await setup();
    await press(svg, 0.6, 1);
    await press(svg, 0.6, 1);
    expect(curve().points.map((p) => p.x)).toEqual([0, 0.02]);
  });

  it("alt-drag on a segment bends it", async () => {
    const svg = await setup();
    // The rising segment from 0.02 to 0.6 (tension -0.45): dragging up = a faster start.
    await drag(svg, [0.3, 0.5], [0.3, 0.8], { altKey: true });
    const c = curve().points[1]!.curve;
    expect(c.type).toBe("Curve");
    expect(c.type === "Curve" && c.tension).toBeLessThan(-0.45);
    // A plain press on a segment changes nothing.
    await drag(svg, [0.3, 0.5], [0.3, 0.9]);
    expect(curve().points[1]!.curve).toEqual(c);
  });

  it("right-click: delete a point, or set a segment's shape", async () => {
    const svg = await setup();
    fireEvent.contextMenu(svg, at(0.6, 1));
    fireEvent.click(screen.getByRole("menuitem", { name: "delete point" }));
    await flush();
    expect(curve().points).toHaveLength(2);
    fireEvent.contextMenu(svg, at(0.3, 0.5));
    fireEvent.click(screen.getByRole("menuitem", { name: "step curve" }));
    await flush();
    expect(curve().points[1]!.curve).toEqual({ type: "Step" });
  });

  it("shows the playhead at the song-locked phase while playing", async () => {
    const svg = await setup();
    const line = svg.querySelector(".eth-lfo-curve__playhead") as SVGLineElement;
    expect(line.style.visibility).toBe("hidden");
    // Synced 1/4 (one beat per cycle): beat 10.25 is a quarter into the cycle.
    act(() => playheadStore.setPlayhead({ transport: { position: 10.25, seconds: 5.125, playing: true, bpm: 120 } }));
    expect(line.style.visibility).toBe("visible");
    expect(line.getAttribute("x1")).toBe((0.25 * W).toFixed(1));
    act(() => playheadStore.setPlayhead({ transport: { position: 10.25, seconds: 5.125, playing: false, bpm: 120 } }));
    expect(line.style.visibility).toBe("hidden");
  });
});
