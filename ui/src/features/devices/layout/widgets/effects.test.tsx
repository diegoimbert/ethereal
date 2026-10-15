// devices-pass-effects: the effect-window visualisations render from their params and drag
// them (one gesture). Bound to the demo Synth's params, which the mock accepts.
import { act, fireEvent, screen, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { DeviceDescriptor, DeviceLayout, LayoutItem, Widget } from "@/generated";
import { devicesOfTrack, useProjectStore } from "@/state";
import { BUILTIN_DESCRIPTORS, type MockTransport } from "@/transport";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/devices/testUtils";
import { useGestureSender } from "../../gesture";
import { DeviceLayoutView } from "../DeviceLayoutView";
import { boundParams } from "../model";

const SYNTH = BUILTIN_DESCRIPTORS.Synth;
let mock: MockTransport | undefined;
const rect = { left: 0, top: 0, right: 200, bottom: 100, width: 200, height: 100, x: 0, y: 0, toJSON: () => ({}) };
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

const synth = () => devicesOfTrack(store().project!, trackByName("Keys").id).find((d) => d.name === "Synth")!;

function Panel({ descriptor }: { descriptor: DeviceDescriptor }) {
  const sender = useGestureSender();
  const device = useProjectStore((s) => (s.project ? devicesOfTrack(s.project, trackByName("Keys").id).find((d) => d.name === "Synth") : undefined));
  if (!device) return null;
  return (
    <section aria-label="panel">
      <DeviceLayoutView device={device} descriptor={descriptor} sender={sender} />
    </section>
  );
}

const it_ = (widget: Widget): LayoutItem => ({ widget, size: "Large", colspan: 1, label: null });

// Synth params: 0 Waveform (enum), 2 Attack ms, 3 Decay ms, 4 Sustain %, 7 Resonance %,
// 8 Volume dB (-60..6).
const WIDGETS: Widget[] = [
  { type: "DynamicsCurve", threshold: 8, ratio: 7, knee: null, mode: null, floor: null, input_gain: null, level: 1 },
  { type: "DelayTaps", time: 3, feedback: 4, sync: null, division: null, ping_pong: null, freeze: null },
  { type: "ReverbDecay", decay: 3, pre_delay: 2, size: null, damping: 7, freeze: null },
  { type: "VelocityCurve", drive: 7, compand: null, in_low: null, in_high: null, out_low: null, out_high: null, mode: 0, random: null },
];

async function renderPanel() {
  const layout: DeviceLayout = { sections: [{ id: "fx", title: "FX", span: 4, columns: 2, items: WIDGETS.map(it_) }] };
  mock = await renderWithMock(<Panel descriptor={{ ...SYNTH, layout }} />);
  return screen.getByRole("region", { name: "panel" });
}

async function drag(svg: Element, from: [number, number], to: [number, number]) {
  await act(async () => {
    fireEvent.pointerDown(svg, { button: 0, pointerId: 1, clientX: from[0], clientY: from[1] });
  });
  await act(async () => {
    fireEvent.pointerMove(svg, { pointerId: 1, clientX: to[0], clientY: to[1] });
  });
  await act(async () => {
    fireEvent.pointerUp(svg, { pointerId: 1 });
  });
  await flush();
}

describe("effect visualisations", () => {
  it("render with their bound controls and no missing params", async () => {
    const panel = await renderPanel();
    for (const t of ["DynamicsCurve", "DelayTaps", "ReverbDecay", "VelocityCurve"]) {
      expect(panel.querySelector(`[data-widget="${t}"]`), t).not.toBeNull();
    }
    expect(panel.querySelector(".eth-widget--missing")).toBeNull();
    // The controls row under each plot: every bound control is a learnable param widget.
    const taps = within(panel).getByTestId("widget-delay-taps").closest("[data-widget]")!;
    expect(within(taps as HTMLElement).getByRole("slider", { name: "Decay" })).toBeTruthy();
    // Decay 300 ms with Sustain 70 %: echoes every 0.3 s decaying by 0.7.
    expect(taps.querySelectorAll("[data-tap]").length).toBeGreaterThan(3);
    expect(boundParams(WIDGETS[3]!)).toEqual([7, 0]);
  });

  it("dragging the transfer curve sets the threshold; the delay plot sets time and feedback", async () => {
    const panel = await renderPanel();
    const curve = within(panel).getByTestId("widget-dynamics-curve").querySelector("svg")!;
    await drag(curve, [100, 50], [150, 50]);
    // x 150 of 200 → -15 dB on the -60..0 axis.
    expect(synth().params[8]).toBeCloseTo(-15, 0);
    const taps = within(panel).getByTestId("widget-delay-taps").querySelector("svg")!;
    await drag(taps, [50, 50], [50, 10]);
    expect(synth().params[4]).toBeCloseTo(90, 0);
    const decay = within(panel).getByTestId("widget-reverb-decay").querySelector("svg")!;
    const before = synth().params[3] ?? 300;
    await drag(decay, [50, 50], [190, 50]);
    expect(synth().params[3]!).toBeGreaterThan(before);
    const velocity = within(panel).getByTestId("widget-velocity-curve").querySelector("svg")!;
    await drag(velocity, [100, 50], [100, 20]);
    expect(synth().params[7]).toBeCloseTo(80, 0);
  });
});
