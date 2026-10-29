import { act, fireEvent, screen, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { Device, DeviceId } from "@/generated";
import { ContextMenuHost } from "@/kit";
import { useProjectStore } from "@/state";
import { BUILTIN_DESCRIPTORS, cmd, newId } from "@/transport";
import { type MockTransport } from "@/transport/mock";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/devices/testUtils";
import { useGestureSender } from "../../gesture";
import { DeviceLayoutView } from "../DeviceLayoutView";
import { freqAt, yOf } from "./geometry";

// `ether_devices::graphic_eq`: band b uses ids 7b..7b+6 (used, on, type, freq, gain, q, slope).
const GEQ = BUILTIN_DESCRIPTORS.GraphicEq;
const id = (band: number, offset: number) => band * 7 + offset;
const [USED, ON, TYPE, FREQ, GAIN, Q, SLOPE] = [0, 1, 2, 3, 4, 5, 6];
/** The plot box in jsdom (`useBoxSize`'s fallback) and the default display range. */
const BOX = { w: 200, h: 72 };
const RANGE = 12;

let mock: MockTransport | undefined;
let devId: DeviceId = "";
const rect = { left: 0, top: 0, right: 200, bottom: 72, width: 200, height: 72, x: 0, y: 0, toJSON: () => ({}) };
beforeAll(() => {
  stubPointerCapture();
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(rect as DOMRect);
  (SVGElement.prototype as unknown as { setPointerCapture: () => void }).setPointerCapture ??= () => {};
});
afterAll(() => vi.restoreAllMocks());
afterEach(() => {
  resetStores(mock);
  mock = undefined;
});

const dev = (): Device => store().project!.devices[devId]!;
const param = (p: number): number => dev().params[p] ?? GEQ.params[p]!.default;

function Panel() {
  const sender = useGestureSender();
  const device = useProjectStore((s) => s.project?.devices[devId]);
  if (!device) return null;
  return (
    <section aria-label="panel">
      <DeviceLayoutView device={device} descriptor={GEQ} sender={sender} />
    </section>
  );
}

async function renderGeq() {
  mock = await renderWithMock(
    <>
      <Panel />
      <ContextMenuHost />
    </>,
  );
  devId = newId();
  await act(async () => {
    await mock!.send(
      cmd("Device", { type: "Insert", id: devId, track: trackByName("Keys").id, device: { type: "Builtin", device: { type: "GraphicEq" } }, before: null }),
    );
  });
  await flush();
  const panel = screen.getByRole("region", { name: "panel" });
  const svg = within(panel).getByTestId("widget-graphic-eq").querySelector<SVGSVGElement>(".eth-geq__svg")!;
  return { panel, svg };
}

/** Plot coordinates of band `b`'s handle. */
function handle(svg: SVGSVGElement, b: number): { x: number; y: number; el: Element } {
  const el = svg.querySelector(`g[data-band="${b}"]`)!;
  const m = /translate\(([-\d.]+) ([-\d.]+)\)/.exec(el.getAttribute("transform")!)!;
  return { x: Number(m[1]), y: Number(m[2]), el };
}

const down = (svg: SVGSVGElement, p: { x: number; y: number }, mods: { altKey?: boolean } = {}) =>
  act(async () => {
    fireEvent.pointerDown(svg, { button: 0, pointerId: 1, clientX: p.x, clientY: p.y, ...mods });
  });
const move = (svg: SVGSVGElement, p: { x: number; y: number }) =>
  act(async () => {
    fireEvent.pointerMove(svg, { pointerId: 1, clientX: p.x, clientY: p.y });
  });
const up = async (svg: SVGSVGElement, p: { x: number; y: number }) => {
  await act(async () => {
    fireEvent.pointerUp(svg, { pointerId: 1, clientX: p.x, clientY: p.y });
  });
  await flush();
};

async function doubleClick(svg: SVGSVGElement, p: { x: number; y: number }) {
  await down(svg, p);
  await up(svg, p);
  await down(svg, p);
  await up(svg, p);
}

async function drag(svg: SVGSVGElement, from: { x: number; y: number }, to: { x: number; y: number }, mods: { altKey?: boolean } = {}) {
  await down(svg, from, mods);
  await move(svg, to);
  await up(svg, to);
}

describe("GraphicEq", () => {
  it("starts as a flat line with no band", async () => {
    const { panel, svg } = await renderGeq();
    expect(svg.querySelectorAll("g[data-band]")).toHaveLength(0);
    expect(svg.getAttribute("data-bands")).toBe("0");
    const zero = yOf(BOX, 0, RANGE).toFixed(1);
    const pts = svg.querySelector('[data-testid="geq-sum"]')!.getAttribute("points")!.trim().split(" ");
    expect(pts.every((p) => p.split(",")[1] === zero)).toBe(true);
    expect(within(panel).getByText("double-click the line to add a band")).toBeTruthy();
  });

  it("a double-click on the line places a bell there, one undo step", async () => {
    const { svg } = await renderGeq();
    const at = { x: 100, y: yOf(BOX, 0, RANGE) };
    await doubleClick(svg, at);
    expect(param(id(0, USED))).toBe(1);
    expect(param(id(0, TYPE))).toBe(0);
    expect(param(id(0, FREQ))).toBeCloseTo(freqAt(BOX, 100), 6);
    expect(param(id(0, GAIN))).toBe(0);
    expect(svg.querySelectorAll("g[data-band]")).toHaveLength(1);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    await flush();
    expect(param(id(0, USED))).toBe(0);
    expect(svg.querySelectorAll("g[data-band]")).toHaveLength(0);
  });

  it("double-click and hold drags the new band at once (one undo step)", async () => {
    const { svg } = await renderGeq();
    const at = { x: 60, y: yOf(BOX, 0, RANGE) };
    await down(svg, at);
    await up(svg, at);
    await down(svg, at);
    await move(svg, { x: 120, y: yOf(BOX, 6, RANGE) });
    await up(svg, { x: 120, y: yOf(BOX, 6, RANGE) });
    expect(param(id(0, USED))).toBe(1);
    expect(param(id(0, FREQ))).toBeCloseTo(freqAt(BOX, 120), 3);
    expect(param(id(0, GAIN))).toBeCloseTo(6, 3);
    const h = handle(svg, 0);
    expect(h.x).toBeCloseTo(120, 0);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    await flush();
    expect(param(id(0, USED))).toBe(0);
    expect(param(id(0, GAIN))).toBe(0);
  });

  it("drags a point (freq + gain), Alt-drag = Q, double-click on it frees the band", async () => {
    const { svg } = await renderGeq();
    await doubleClick(svg, { x: 100, y: yOf(BOX, 0, RANGE) });
    // A second band takes the next slot.
    await doubleClick(svg, { x: 160, y: yOf(BOX, 0, RANGE) });
    expect(param(id(1, USED))).toBe(1);
    const h = handle(svg, 0);
    await drag(svg, h, { x: h.x - 20, y: h.y - 8 });
    expect(param(id(0, FREQ))).toBeLessThan(freqAt(BOX, 100));
    expect(param(id(0, GAIN))).toBeCloseTo(4, 3);
    const h2 = handle(svg, 0);
    await drag(svg, h2, { x: h2.x + 30, y: h2.y - 24 }, { altKey: true });
    expect(param(id(0, Q))).toBeGreaterThan(1);
    expect(param(id(0, GAIN))).toBeCloseTo(4, 3);
    await doubleClick(svg, handle(svg, 0));
    expect(param(id(0, USED))).toBe(0);
    expect(param(id(1, USED))).toBe(1);
    expect(svg.querySelectorAll("g[data-band]")).toHaveLength(1);
  });

  it("the context menu changes the type (a cut starts flat) and the slope", async () => {
    const { svg } = await renderGeq();
    await doubleClick(svg, { x: 100, y: yOf(BOX, 0, RANGE) });
    const h = handle(svg, 0);
    fireEvent.contextMenu(svg, { clientX: h.x, clientY: h.y });
    expect(screen.getByRole("menuitem", { name: "✓ Bell" })).toBeTruthy();
    fireEvent.click(screen.getByRole("menuitem", { name: "Low Cut" }));
    await flush();
    expect(param(id(0, TYPE))).toBe(2);
    expect(param(id(0, Q))).toBeCloseTo(Math.SQRT1_2, 9);
    fireEvent.contextMenu(svg, { clientX: h.x, clientY: h.y });
    fireEvent.click(screen.getByRole("menuitem", { name: "slope" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "48 dB/oct" }));
    await flush();
    expect(param(id(0, SLOPE))).toBe(5);
    fireEvent.contextMenu(svg, { clientX: h.x, clientY: h.y });
    fireEvent.click(screen.getByRole("menuitem", { name: "disable band" }));
    await flush();
    expect(param(id(0, ON))).toBe(0);
    expect(handle(svg, 0).el.getAttribute("data-on")).toBe("false");
  });

  it("a selected band shows its panel; the wheel sets Q; Delete frees it", async () => {
    const { panel, svg } = await renderGeq();
    await doubleClick(svg, { x: 100, y: yOf(BOX, 0, RANGE) });
    await flush();
    const p = within(panel).getByTestId("geq-panel");
    expect(p.getAttribute("data-band")).toBe("0");
    expect(within(p).getAllByRole("slider").length).toBeGreaterThanOrEqual(3);
    const h = handle(svg, 0);
    const plot = svg.parentElement!;
    await act(async () => {
      plot.dispatchEvent(new WheelEvent("wheel", { deltaY: -100, clientX: h.x, clientY: h.y, bubbles: true, cancelable: true }));
    });
    await act(async () => {
      await new Promise((r) => setTimeout(r, 450));
    });
    await flush();
    expect(param(id(0, Q))).not.toBe(1);
    fireEvent.keyDown(handle(svg, 0).el, { key: "Delete" });
    await flush();
    expect(param(id(0, USED))).toBe(0);
    expect(within(panel).queryByTestId("geq-panel")).toBeNull();
  });

  it("the range button cycles the display range", async () => {
    const { panel } = await renderGeq();
    const b = within(panel).getByTestId("geq-range");
    expect(b.textContent).toBe("±12 db");
    fireEvent.click(b);
    expect(b.textContent).toBe("±24 db");
  });
});
