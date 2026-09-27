import { act, fireEvent, screen, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { Device, DeviceDescriptor, DeviceLayout, LayoutItem, Widget } from "@/generated";
import { ContextMenuHost } from "@/kit";
import { devicesOfTrack, useProjectStore } from "@/state";
import { BUILTIN_DESCRIPTORS, cmd, newId, type MockTransport } from "@/transport";
import { flush, renderWithMock, resetStores, store, stubPointerCapture, trackByName } from "@/features/mixer/testUtils";
import { resetAutomationUi, useAutomationUi } from "@/features/automation/uiStore";
import { useGestureSender } from "../gesture";
import { DeviceLayoutView } from "./DeviceLayoutView";

// The Keys track's Synth of the demo project: its params (ids 0..8, `ether_devices::synth`)
// are what the mock accepts, so test layouts bind those.
const SYNTH = BUILTIN_DESCRIPTORS.Synth;

let mock: MockTransport | undefined;
const rect = { left: 0, top: 0, right: 200, bottom: 80, width: 200, height: 80, x: 0, y: 0, toJSON: () => ({}) };
beforeAll(() => {
  stubPointerCapture();
  // jsdom has no layout: give every element a 200×80 box (plots map pointer coords by it).
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(rect as DOMRect);
  (SVGElement.prototype as unknown as { setPointerCapture: () => void }).setPointerCapture ??= () => {};
});
afterAll(() => vi.restoreAllMocks());
afterEach(() => {
  resetStores(mock);
  resetAutomationUi();
  mock = undefined;
});

const synth = (): Device => devicesOfTrack(store().project!, trackByName("Keys").id).find((d) => d.name === "Synth")!;

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

const it_ = (widget: Widget, size: LayoutItem["size"] = "Medium", colspan = 1): LayoutItem => ({ widget, size, colspan, label: null });

/** Every widget of the catalog, bound to the synth's params. */
const EVERY_WIDGET: DeviceLayout = {
  sections: [
    {
      id: "params",
      title: "Params",
      span: 2,
      columns: 4,
      items: [
        it_({ type: "Knob", param: 6 }, "Large"),
        it_({ type: "Slider", param: 7, vertical: false }),
        it_({ type: "Slider", param: 8, vertical: true }),
        it_({ type: "Choice", param: 0 }),
        it_({ type: "Number", param: 1 }, "Small"),
      ],
    },
    {
      id: "typed",
      title: "Typed",
      span: 4,
      columns: 2,
      items: [
        it_({ type: "Envelope", attack: 2, decay: 3, sustain: 4, release: 5, delay: null, hold: null }, "Medium", 2),
        it_({ type: "FilterCurve", cutoff: 6, resonance: 7, mode: null, drive: null, gain: null }),
        it_({ type: "TransferCurve", drive: 7, curve: null, bias: null }),
        it_({ type: "Oscillator", shape: 0, position: null }),
        it_({ type: "Lfo", shape: 0, rate: 6, amount: null }),
        it_({ type: "StepEditor", first: 2, count: 3 }),
        it_({ type: "XyPad", x: 6, y: 7 }),
        it_({ type: "Crossover", frequencies: [6] }),
      ],
    },
    {
      id: "data",
      title: null,
      span: 1,
      columns: 2,
      items: [
        it_({ type: "SampleWaveform", start: null, end: null }),
        it_({ type: "ZoneMap" }),
        it_({ type: "Spectrum" }),
        it_({ type: "Tuner" }),
        it_({ type: "Meter", index: 0, min_db: -24, max_db: 0 }),
        it_({ type: "RackChains" }),
        it_({ type: "Macros" }),
        it_({ type: "EqCurve", bands: [{ on: null, kind: null, shapes: ["Bell"], freq: 6, gain: null, q: null }], crossovers: [], spectrum: "None" }),
      ],
    },
  ],
};

async function renderPanel(layout: DeviceLayout | undefined) {
  const descriptor: DeviceDescriptor = { ...SYNTH, ...(layout ? { layout } : { layout: undefined }) };
  mock = await renderWithMock(
    <>
      <Panel descriptor={descriptor} />
      <ContextMenuHost />
    </>,
  );
  return screen.getByRole("region", { name: "panel" });
}

describe("DeviceLayoutView", () => {
  it("renders every catalog widget; every param control is MIDI-learnable and a modulation target", async () => {
    const panel = await renderPanel(EVERY_WIDGET);
    const types = new Set([...panel.querySelectorAll("[data-widget]")].map((e) => e.getAttribute("data-widget")));
    for (const t of ["Envelope", "FilterCurve", "TransferCurve", "Oscillator", "Lfo", "StepEditor", "XyPad", "Crossover", "SampleWaveform", "ZoneMap", "Spectrum", "Tuner", "Meter", "RackChains", "Macros", "EqCurve"]) {
      expect(types.has(t), t).toBe(true);
    }
    expect(panel.querySelector(".eth-device__group-name")?.textContent).toBe("Params");
    expect(panel.querySelector('[data-widget="Missing"], .eth-widget--missing')).toBeNull();
    const params = [...panel.querySelectorAll(".eth-param")];
    expect(params.length).toBeGreaterThan(10);
    for (const el of params) {
      expect(el.getAttribute("data-midi-target"), el.outerHTML.slice(0, 80)).toBeTruthy();
      expect(el.getAttribute("data-mod-target")).toMatch(/:\d+$/);
      expect(el.querySelector(".eth-param__mod")).not.toBeNull();
    }
    // Section weights and columns reach the CSS grid; colspans wrap their item.
    const typed = panel.querySelector('[data-section="typed"]') as HTMLElement;
    expect(typed.style.getPropertyValue("--section-columns")).toBe("2");
    expect(typed.querySelector('[data-colspan="2"]')).not.toBeNull();
    // The synth's params the layout doesn't cover fold under "More" (none: all bound).
    expect(within(panel).queryByRole("button", { name: /More .* controls/ })).toBeNull();
  });

  it("devices without a layout get the generic one (leading groups, the rest under More)", async () => {
    const panel = await renderPanel(undefined);
    expect([...panel.querySelectorAll(".eth-device__group-name")].map((e) => e.textContent)).toEqual(["Oscillator"]);
    fireEvent.click(within(panel).getByRole("button", { name: "More Synth controls (7)".replace(" (7)", "") }));
    expect([...panel.querySelectorAll(".eth-device__group-name")].map((e) => e.textContent)).toEqual(["Oscillator", "Envelope", "Filter", "Output"]);
  });

  it("stepped params snap (number field, knob text)", async () => {
    const panel = await renderPanel(EVERY_WIDGET);
    const field = within(panel).getByRole("spinbutton", { name: "Transpose" });
    fireEvent.change(field, { target: { value: "3.4" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await flush();
    expect(synth().params[1]).toBe(3);
  });

  it("an XY pad drag sets both params in one undo step", async () => {
    const panel = await renderPanel(EVERY_WIDGET);
    const svg = within(panel).getByTestId("widget-xy-pad").querySelector("svg")!;
    const undo = store().history.undo_label;
    await act(async () => {
      fireEvent.pointerDown(svg, { button: 0, pointerId: 1, clientX: 100, clientY: 40 });
    });
    await act(async () => {
      fireEvent.pointerMove(svg, { pointerId: 1, clientX: 150, clientY: 20 });
    });
    await act(async () => {
      fireEvent.pointerUp(svg, { pointerId: 1, clientX: 150, clientY: 20 });
    });
    await flush();
    const res = synth().params[7]!;
    expect(res).toBeCloseTo(75, 6);
    expect(synth().params[6]!).toBeGreaterThan(2400);
    await act(() => mock!.send(cmd("Edit", { type: "Undo" })));
    expect(synth().params[7]).toBe(25);
    expect(synth().params[6]).toBe(2400);
    expect(store().history.undo_label).toBe(undo);
  });

  it("dragging an envelope handle edits its time param", async () => {
    const panel = await renderPanel(EVERY_WIDGET);
    const svg = within(panel).getByTestId("widget-envelope").querySelector("svg")!;
    const handle = svg.querySelector('[data-handle="Attack"]')!;
    const cx = Number(handle.getAttribute("cx"));
    const cy = Number(handle.getAttribute("cy"));
    const before = synth().params[2] ?? 5;
    await act(async () => {
      fireEvent.pointerDown(svg, { button: 0, pointerId: 1, clientX: cx, clientY: cy });
    });
    await act(async () => {
      fireEvent.pointerMove(svg, { pointerId: 1, clientX: cx + 20, clientY: cy });
    });
    await act(async () => {
      fireEvent.pointerUp(svg, { pointerId: 1 });
    });
    await flush();
    expect(synth().params[2]!).toBeGreaterThan(before);
  });

  it("the param context menu opens its automation lane; automated params show an indicator", async () => {
    const panel = await renderPanel(EVERY_WIDGET);
    const cutoff = panel.querySelector('[data-param="6"]')!;
    fireEvent.contextMenu(cutoff);
    fireEvent.click(screen.getByRole("menuitem", { name: "Show automation lane" }));
    const d = synth();
    expect(useAutomationUi.getState().open.has(d.track)).toBe(true);
    expect(useAutomationUi.getState().shown[d.track]).toContain(`param:${d.id}:6`);

    expect(within(panel).queryAllByRole("img", { name: "Cutoff is automated" })).toHaveLength(0);
    await act(async () => {
      await mock!.send(
        cmd("Automation", { type: "CreateLane", id: newId(), owner: { type: "Track", track: d.track }, target: { type: "DeviceParam", device: d.id, param: 6 } }),
      );
    });
    await flush();
    expect(within(panel).getAllByRole("img", { name: "Cutoff is automated" }).length).toBeGreaterThan(0);

    // Reset to default from the menu.
    fireEvent.contextMenu(panel.querySelector('[data-param="7"]')!);
    fireEvent.click(screen.getByRole("menuitem", { name: "Reset to default" }));
    await flush();
    expect(synth().params[7] ?? SYNTH.params[7]!.default).toBe(SYNTH.params[7]!.default);
  });

  it("short enum label sets render as a segmented control", async () => {
    const descriptor = {
      ...SYNTH,
      params: SYNTH.params.map((p) => (p.id === 0 ? { ...p, labels: ["Sin", "Saw", "Sqr", "Tri"] } : p)),
    };
    const layout: DeviceLayout = { sections: [{ id: "s", title: null, span: 1, columns: 1, items: [it_({ type: "Choice", param: 0 })] }] };
    mock = await renderWithMock(<Panel descriptor={{ ...descriptor, layout }} />);
    const group = screen.getByRole("group", { name: "Waveform" });
    fireEvent.click(within(group).getByRole("button", { name: "Tri" }));
    await flush();
    expect(synth().params[0]).toBe(3);
    expect(within(group).getByRole("button", { name: "Tri" })).toHaveAttribute("aria-pressed", "true");
  });
});
