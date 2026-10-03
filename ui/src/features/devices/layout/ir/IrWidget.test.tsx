import { act, createEvent, fireEvent, screen, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { DeviceId, FactoryIr, IrSource } from "@/generated";
import { BROWSER_DRAG_MIME } from "@/features/browser/dragPayload";
import { useProjectStore } from "@/state";
import { BUILTIN_DESCRIPTORS, cmd, newId, type MockTransport } from "@/transport";
import { LIBRARY_ID } from "@/transport/mock/library";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/mixer/testUtils";
import { useGestureSender } from "../../gesture";
import { DeviceLayoutView } from "../DeviceLayoutView";
import {
  axisSeconds,
  decayAt,
  envelopePath,
  factoryEnvelope,
  irFromKey,
  irKey,
  preDelayAt,
  resetFactoryIrCache,
  shapedAt,
  shapingOf,
  stepFactory,
  timeToX,
} from ".";

const DESC = BUILTIN_DESCRIPTORS.ConvolutionReverb;
let mock: MockTransport | undefined;
let rvId: DeviceId = "";
// jsdom has no layout: every box is 300 × 80.
const rect = { left: 0, top: 0, right: 300, bottom: 80, width: 300, height: 80, x: 0, y: 0, toJSON: () => ({}) };
beforeAll(() => {
  stubPointerCapture();
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(rect as DOMRect);
  (SVGElement.prototype as unknown as { setPointerCapture: () => void }).setPointerCapture ??= () => {};
});
afterAll(() => vi.restoreAllMocks());
afterEach(() => {
  resetStores(mock);
  mock = undefined;
  resetFactoryIrCache();
});

const irOf = (): IrSource | null | undefined => {
  const k = store().project!.devices[rvId]!.kind;
  return k.type === "Builtin" && k.device.type === "ConvolutionReverb" ? k.device.ir : undefined;
};
const param = (id: number) => store().project!.devices[rvId]!.params[id];

function Panel() {
  const sender = useGestureSender();
  const device = useProjectStore((s) => s.project?.devices[rvId]);
  if (!device) return null;
  return (
    <section aria-label="panel">
      <DeviceLayoutView device={device} descriptor={DESC} sender={sender} />
    </section>
  );
}

async function renderReverb(ir: IrSource | null = null) {
  mock = await renderWithMock(<Panel />);
  rvId = newId();
  await act(async () => {
    await mock!.send(
      cmd("Device", { type: "Insert", id: rvId, track: trackByName("Keys").id, device: { type: "Builtin", device: { type: "ConvolutionReverb", ir: null } }, before: null }),
    );
    if (ir) await mock!.send(cmd("Device", { type: "SetIr", device: rvId, ir }));
  });
  await flush();
  await flush();
  const panel = screen.getByRole("region", { name: "panel" });
  return { panel, widget: within(panel).getByTestId("ir-widget") };
}

describe("IrWidget", () => {
  it("is drawn in the reverb's waveform slot, empty until an IR is chosen", async () => {
    const { widget } = await renderReverb();
    expect(within(widget).getByRole("img", { name: "No impulse response" })).toBeTruthy();
    expect(within(widget).queryByTestId("ir-envelope")).toBeNull();
    expect(within(widget).queryByRole("button", { name: "Remove impulse response" })).toBeNull();
  });

  it("picks a factory IR from the list (one SetIr) and draws it", async () => {
    const { widget } = await renderReverb();
    await act(async () => {
      fireEvent.click(within(widget).getByTestId("ir-select"));
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("option", { name: "Concert Hall" }));
    });
    await flush();
    expect(irOf()).toEqual({ type: "Factory", id: "hall" });
    expect(within(widget).getByTestId("ir-meta").textContent).toBe("2.8 s · Stereo");
    expect(within(widget).getByTestId("ir-envelope").getAttribute("d")).toMatch(/^M/);
  });

  it("steps through the factory IRs and removes the IR", async () => {
    const { widget } = await renderReverb({ type: "Factory", id: "room" });
    await act(async () => {
      fireEvent.click(within(widget).getByRole("button", { name: "Next impulse response" }));
    });
    await flush();
    expect(irOf()).toEqual({ type: "Factory", id: "chamber" });
    await act(async () => {
      fireEvent.click(within(widget).getByRole("button", { name: "Previous impulse response" }));
    });
    await act(async () => {
      fireEvent.click(within(widget).getByRole("button", { name: "Previous impulse response" }));
    });
    await flush();
    expect(irOf()).toEqual({ type: "Factory", id: "ambience" }); // wraps around
    await act(async () => {
      fireEvent.click(within(widget).getByRole("button", { name: "Remove impulse response" }));
    });
    await flush();
    expect(irOf()).toBeNull();
  });

  it("drags the Decay marker (one undo step)", async () => {
    const { widget } = await renderReverb({ type: "Factory", id: "hall" });
    const svg = within(widget).getByTestId("widget-ir").querySelector("svg")!;
    const len = 2.8;
    const xEnd = timeToX(len, len, 300);
    await act(async () => {
      fireEvent.pointerDown(svg, { button: 0, pointerId: 1, clientX: xEnd, clientY: 40 });
    });
    await act(async () => {
      fireEvent.pointerMove(svg, { pointerId: 1, clientX: xEnd / 2, clientY: 40 });
    });
    await act(async () => {
      fireEvent.pointerUp(svg, { pointerId: 1, clientX: xEnd / 2, clientY: 40 });
    });
    await flush();
    expect(param(2)).toBeCloseTo(50, 0);
    expect(within(widget).getByTestId("ir-meta").textContent).toBe("1.4 s of 2.8 s · Stereo");
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    await flush();
    expect(param(2) ?? 100).toBe(100);
  });

  it("a Browser audio file dropped on it becomes the IR", async () => {
    const { widget } = await renderReverb({ type: "Factory", id: "hall" });
    const payload = {
      version: 1,
      kind: "media",
      source: { type: "Location", location: { type: "Library", id: LIBRARY_ID }, path: "Drums/Snare.wav" },
      name: "Snare.wav",
      file_kind: "Audio",
    };
    const dataTransfer = { types: [BROWSER_DRAG_MIME], getData: (t: string) => (t === BROWSER_DRAG_MIME ? JSON.stringify(payload) : ""), dropEffect: "none" };
    await act(async () => {
      for (const make of [createEvent.dragOver, createEvent.drop]) fireEvent(widget, make(widget, { dataTransfer }));
    });
    await flush();
    await flush();
    const ir = irOf();
    expect(ir?.type).toBe("Media");
    const media = ir?.type === "Media" ? store().project!.media[ir.media] : undefined;
    expect(media?.name).toBe("Snare.wav");
    expect(within(widget).getByRole("combobox", { name: "Impulse response" }).textContent).toContain("Snare.wav");
  });
});

describe("irMath", () => {
  const hall: FactoryIr = { id: "hall", name: "Concert Hall", category: "Hall", length: 2.8, channels: 2 };
  const env = factoryEnvelope(hall);

  it("shapes like the engine: decay cuts, size stretches, reverse flips, pre-delay offsets", () => {
    const plain = shapingOf({ decay: 100, size: 100, reverse: 0, preDelay: 0 });
    expect(shapedAt(env, 2.8, plain, 0.5)).toBeCloseTo(env(0.5));
    const cut = shapingOf({ decay: 50, size: 100, reverse: 0, preDelay: 0 });
    expect(shapedAt(env, 2.8, cut, 1.5)).toBe(0);
    expect(shapedAt(env, 2.8, cut, 1.39)).toBeLessThan(0.05 * env(1.39));
    const big = shapingOf({ decay: 100, size: 150, reverse: 0, preDelay: 0 });
    expect(shapedAt(env, 2.8, big, 1.5)).toBeCloseTo(env(1.0));
    const rev = shapingOf({ decay: 100, size: 100, reverse: 1, preDelay: 0 });
    expect(shapedAt(env, 2.8, rev, 2.7)).toBeCloseTo(env(0.1));
    const pre = shapingOf({ decay: 100, size: 100, reverse: 0, preDelay: 100 });
    expect(envelopePath(env, 2.8, pre, 100, 10)).toContain("M0.0,5.0");
    expect(axisSeconds(2.8)).toBeCloseTo(2.8 * 1.5 + 0.25);
  });

  it("maps drags back to params", () => {
    const s = shapingOf({ decay: 100, size: 100, reverse: 0, preDelay: 0 });
    expect(decayAt(timeToX(1.4, 2.8, 300), 2.8, s, 300)).toBeCloseTo(50);
    expect(decayAt(0, 2.8, s, 300)).toBe(10);
    expect(preDelayAt(timeToX(0.1, 2.8, 300), 2.8, 300)).toBeCloseTo(100);
    expect(preDelayAt(300, 2.8, 300)).toBe(250);
  });

  it("encodes picker values and steps through factory IRs", () => {
    for (const ir of [null, { type: "Factory", id: "plate" }, { type: "Media", media: "01K00000000000000000000001" }] as const) {
      expect(irFromKey(irKey(ir))).toEqual(ir);
    }
    expect(irFromKey("garbage")).toBeUndefined();
    const irs = [hall, { ...hall, id: "room" }];
    expect(stepFactory(irs, null, 1)).toEqual({ type: "Factory", id: "hall" });
    expect(stepFactory(irs, null, -1)).toEqual({ type: "Factory", id: "room" });
    expect(stepFactory(irs, { type: "Factory", id: "room" }, 1)).toEqual({ type: "Factory", id: "hall" });
  });
});
