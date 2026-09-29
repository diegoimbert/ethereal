import { act, fireEvent, screen, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { Device, DeviceId, Event } from "@/generated";
import { ContextMenuHost } from "@/kit";
import { useProjectStore } from "@/state";
import { BUILTIN_DESCRIPTORS, cmd, newId, type MockTransport } from "@/transport";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/mixer/testUtils";
import { useGestureSender } from "../../gesture";
import { DeviceLayoutView } from "../DeviceLayoutView";

// `ether_devices::eq`: band b uses ids 5b..5b+4 (on, type, freq, gain, q).
const EQ = BUILTIN_DESCRIPTORS.Eq;
const id = (band: number, offset: number) => band * 5 + offset;
const [ON, TYPE, FREQ, GAIN, Q] = [0, 1, 2, 3, 4];

let mock: MockTransport | undefined;
let eqId: DeviceId = "";
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

const eq = (): Device => store().project!.devices[eqId]!;
const param = (p: number): number => eq().params[p] ?? EQ.params[p]!.default;

function Panel() {
  const sender = useGestureSender();
  const device = useProjectStore((s) => s.project?.devices[eqId]);
  if (!device) return null;
  return (
    <section aria-label="panel">
      <DeviceLayoutView device={device} descriptor={EQ} sender={sender} />
    </section>
  );
}

async function renderEq() {
  mock = await renderWithMock(
    <>
      <Panel />
      <ContextMenuHost />
    </>,
  );
  eqId = newId();
  await act(async () => {
    await mock!.send(cmd("Device", { type: "Insert", id: eqId, track: trackByName("Keys").id, device: { type: "Builtin", device: { type: "Eq" } }, before: null }));
  });
  await flush();
  const panel = screen.getByRole("region", { name: "panel" });
  const svg = within(panel).getByTestId("widget-eq-curve").querySelector("svg")!;
  return { panel, svg };
}

/** Plot coordinates of band `b`'s handle. */
function handle(svg: SVGSVGElement, b: number): { x: number; y: number; el: Element } {
  const el = svg.querySelector(`[data-band="${b}"]`)!;
  const m = /translate\(([-\d.]+) ([-\d.]+)\)/.exec(el.getAttribute("transform")!)!;
  return { x: Number(m[1]), y: Number(m[2]), el };
}

async function drag(svg: SVGSVGElement, from: { x: number; y: number }, to: { x: number; y: number }, mods: { altKey?: boolean } = {}) {
  await act(async () => {
    fireEvent.pointerDown(svg, { button: 0, pointerId: 1, clientX: from.x, clientY: from.y, ...mods });
  });
  await act(async () => {
    fireEvent.pointerMove(svg, { pointerId: 1, clientX: to.x, clientY: to.y, ...mods });
  });
  await act(async () => {
    fireEvent.pointerUp(svg, { pointerId: 1, clientX: to.x, clientY: to.y });
  });
  await flush();
}

describe("EqCurve", () => {
  it("draws the summed curve and one handle per band (off bands marked)", async () => {
    const { svg } = await renderEq();
    expect(svg.querySelectorAll("[data-band]")).toHaveLength(8);
    expect(svg.querySelector('[data-testid="eq-sum"]')).not.toBeNull();
    // Defaults: band 1 (low cut) and band 8 (high cut) are off.
    expect(handle(svg, 0).el.getAttribute("data-on")).toBe("false");
    expect(handle(svg, 3).el.getAttribute("data-on")).toBe("true");
    expect(handle(svg, 3).el.getAttribute("aria-label")).toMatch(/^Band 4 · Bell · 1(\.0+)? kHz · 0\.0 dB · Q 0\.71/);
  });

  it("a handle drag sets frequency and gain in one undo step", async () => {
    const { svg } = await renderEq();
    const h = handle(svg, 3);
    const undo = store().history.undo_label;
    await drag(svg, h, { x: h.x + 20, y: h.y - 10 });
    expect(param(id(3, FREQ))).toBeGreaterThan(1500);
    expect(param(id(3, GAIN))).toBeGreaterThan(5);
    expect(param(id(3, Q))).toBeCloseTo(Math.SQRT1_2, 9);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(param(id(3, FREQ))).toBe(1000);
    expect(param(id(3, GAIN))).toBe(0);
    expect(store().history.undo_label).toBe(undo);
  });

  it("a cut band drags horizontally only; Alt-drag changes Q only", async () => {
    const { svg } = await renderEq();
    // Band 8 is a high cut: vertical movement doesn't touch its (unused) gain.
    const cut = handle(svg, 7);
    await drag(svg, cut, { x: cut.x - 10, y: cut.y - 20 });
    expect(param(id(7, FREQ))).toBeLessThan(18000);
    expect(param(id(7, GAIN))).toBe(0);
    const bell = handle(svg, 4);
    await drag(svg, bell, { x: bell.x + 30, y: bell.y - 24 }, { altKey: true });
    expect(param(id(4, Q))).toBeGreaterThan(1);
    expect(param(id(4, FREQ))).toBe(2500);
    expect(param(id(4, GAIN))).toBe(0);
  });

  it("double-click on a handle toggles its band", async () => {
    const { svg } = await renderEq();
    const h = handle(svg, 2);
    await drag(svg, h, h);
    await drag(svg, h, h);
    await act(async () => {
      fireEvent.doubleClick(svg, { clientX: h.x, clientY: h.y });
    });
    await flush();
    expect(param(id(2, ON))).toBe(0);
  });

  it("the handle's context menu sets the type, toggles and resets the band", async () => {
    const { svg, panel } = await renderEq();
    const wrap = panel.querySelector(".eth-eq")!;
    const h = handle(svg, 3);
    fireEvent.contextMenu(wrap, { clientX: h.x, clientY: h.y });
    expect(screen.getByRole("menuitem", { name: "✓ Bell" })).toBeTruthy();
    fireEvent.click(screen.getByRole("menuitem", { name: "High Shelf" }));
    await flush();
    expect(param(id(3, TYPE))).toBe(4);
    fireEvent.contextMenu(wrap, { clientX: h.x, clientY: h.y });
    fireEvent.click(screen.getByRole("menuitem", { name: "Disable Band" }));
    await flush();
    expect(param(id(3, ON))).toBe(0);
    await drag(svg, handle(svg, 3), { x: h.x + 10, y: h.y - 10 });
    fireEvent.contextMenu(wrap, { clientX: handle(svg, 3).x, clientY: handle(svg, 3).y });
    fireEvent.click(screen.getByRole("menuitem", { name: "Reset Band" }));
    await flush();
    expect(param(id(3, FREQ))).toBe(1000);
    expect(param(id(3, GAIN))).toBe(0);
    expect(param(id(3, TYPE))).toBe(2);
  });

  it("the wheel over a handle changes Q; arrow keys move a focused handle", async () => {
    const { svg, panel } = await renderEq();
    const wrap = panel.querySelector(".eth-eq")!;
    const h = handle(svg, 3);
    await act(async () => {
      wrap.dispatchEvent(new WheelEvent("wheel", { deltaY: -100, clientX: h.x, clientY: h.y, bubbles: true, cancelable: true }));
    });
    await flush();
    expect(param(id(3, Q))).toBeGreaterThan(0.9);
    // Away from any handle the wheel does nothing (the page scrolls).
    const q = param(id(3, Q));
    const ev = new WheelEvent("wheel", { deltaY: -100, clientX: 1, clientY: 1, bubbles: true, cancelable: true });
    wrap.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(false);
    expect(param(id(3, Q))).toBe(q);

    fireEvent.keyDown(handle(svg, 3).el, { key: "ArrowUp" });
    await flush();
    expect(param(id(3, GAIN))).toBeCloseTo(1, 6);
  });

  it("draws the pre/post spectrum from the EQ's analysis frames while watched", async () => {
    const { svg } = await renderEq();
    const analysis = (mock as unknown as { analysis: { watched: Set<DeviceId> } }).analysis;
    expect(analysis.watched.has(eqId)).toBe(true);
    const frame = (stage: "Pre" | "Post"): Event => ({
      type: "Analysis",
      event: { type: "Frame", device: eqId, data: { type: "Spectrum", min_hz: 20, max_hz: 20000, bins_db: Array.from({ length: 64 }, (_, i) => -30 - i / 2), stage } },
    });
    await act(async () => {
      const emit = (mock as unknown as { emit(e: Event): void }).emit.bind(mock);
      emit(frame("Pre"));
      emit(frame("Post"));
      await new Promise((r) => setTimeout(r, 40));
    });
    expect(svg.querySelector('[data-testid="eq-spectrum-pre"]')).not.toBeNull();
    expect(svg.querySelector('[data-testid="eq-spectrum-post"]')).not.toBeNull();
  });
});
