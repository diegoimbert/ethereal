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

// Volume shaper param ids (`ether_devices::fx_modulation::volume_lfo`).
const BANDS = 8;
const CROSSOVER_1 = 9;
const CROSSOVER_2 = 10;

describe("LfoCurveWidget bands", () => {
  const params = () => store().project!.devices[devId]!.params;
  const bands = () => {
    const k = store().project!.devices[devId]!.kind;
    if (k.type !== "Builtin" || k.device.type !== "VolumeLfo") throw new Error("not a volume lfo");
    return k.device.bands ?? [];
  };
  const editor = () => within(screen.getByTestId("widget-lfo-curve")).getByRole("img") as unknown as SVGSVGElement;
  const strip = () => within(screen.getByTestId("lfo-bands-strip")).getByRole("img") as unknown as SVGSVGElement;
  const setCount = async (name: string) => {
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name }));
    });
    await flush();
  };
  const stripDrag = async (from: number, to: number) => {
    const svg = strip();
    await act(async () => {
      fireEvent.pointerDown(svg, { button: 0, pointerId: 2, clientX: from, clientY: 10 });
    });
    await act(async () => {
      fireEvent.pointerMove(svg, { pointerId: 2, clientX: to, clientY: 10 });
    });
    await act(async () => {
      fireEvent.pointerUp(svg, { pointerId: 2, clientX: to, clientY: 10 });
    });
    await flush();
  };

  it("one band by default; the count picker splits it into tabs in the band colours", async () => {
    await setup();
    expect(screen.getAllByRole("tab")).toHaveLength(1);
    expect(strip().querySelectorAll("[data-handle]")).toHaveLength(0);
    await setCount("3 bands");
    expect(params()[BANDS]).toBe(2);
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((t) => t.querySelector(".eth-lfo-bands__name")!.textContent)).toEqual(["low", "mid", "high"]);
    expect(tabs.map((t) => /eth-lfo-band--\d/.exec(t.className)![0])).toEqual(["eth-lfo-band--0", "eth-lfo-band--1", "eth-lfo-band--2"]);
    expect(strip().querySelectorAll("[data-handle]")).toHaveLength(2);
    expect(screen.getByTestId("widget-lfo-curve")).toHaveAttribute("data-band", "0");
  });

  it("edits only the selected band's curve, each edit one undo step", async () => {
    await setup();
    await setCount("3 bands");
    fireEvent.click(screen.getByRole("tab", { name: /^mid/ }));
    await flush();
    expect(screen.getByTestId("widget-lfo-curve")).toHaveAttribute("data-band", "1");
    // A band without a curve is flat.
    expect(editor().getAttribute("data-points")).toBe("1");
    await press(editor(), 0.5, 0.25);
    await press(editor(), 0.5, 0.25);
    expect(bands()).toHaveLength(1);
    expect(bands()[0]!.points.map((p) => p.x)).toEqual([0, 0.5]);
    expect(curve()).toEqual(DEFAULT_LFO_CURVE);
    // The high band: the mid one is kept.
    fireEvent.click(screen.getByRole("tab", { name: /^high/ }));
    await flush();
    await press(editor(), 0.25, 0.5);
    await press(editor(), 0.25, 0.5);
    expect(bands()).toHaveLength(2);
    expect(bands()[0]!.points).toHaveLength(2);
    await undo();
    expect(bands()).toHaveLength(1);
    await undo();
    expect(bands()).toEqual([]);
  });

  it("clicking a band in the strip selects it", async () => {
    await setup();
    await setCount("3 bands");
    // 200 Hz sits at 1/3 of the log axis, 15 kHz at ~0.96.
    await stripDrag(0.6 * W, 0.6 * W);
    expect(screen.getByTestId("widget-lfo-curve")).toHaveAttribute("data-band", "1");
    expect(screen.getByRole("tab", { name: /^mid/ })).toHaveAttribute("aria-selected", "true");
    await stripDrag(0.1 * W, 0.1 * W);
    expect(screen.getByTestId("widget-lfo-curve")).toHaveAttribute("data-band", "0");
    // Back to one band: the editor draws the whole signal's curve.
    fireEvent.click(screen.getByRole("tab", { name: /^high/ }));
    await setCount("1 band");
    expect(screen.getByTestId("widget-lfo-curve")).not.toHaveAttribute("data-band");
    expect(editor().getAttribute("data-points")).toBe("3");
  });

  it("drags a crossover in one undo step, kept under the next one", async () => {
    await setup();
    await setCount("3 bands");
    await stripDrag(W / 3, W / 2);
    // Half way along 20 Hz..20 kHz: ~632 Hz.
    expect(params()[CROSSOVER_1]).toBeCloseTo(20 * Math.sqrt(1000), 0);
    await stripDrag(W / 2, W);
    // Stopped at crossover 2.
    expect(params()[CROSSOVER_1]).toBeCloseTo(params()[CROSSOVER_2] ?? 15000, -1);
    await undo();
    expect(params()[CROSSOVER_1]).toBeCloseTo(20 * Math.sqrt(1000), 0);
    await undo();
    expect(params()[CROSSOVER_1] ?? 200).toBeCloseTo(200, 0);
  });
});
