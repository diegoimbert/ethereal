// fm-synth: the algorithm picker draws one diagram per routing and selects on click and arrow
// keys. Bound to the demo Synth's `Waveform` (four labels), which the mock accepts; the
// diagram geometry is checked against the FM Synth's own table.
import { act, fireEvent, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { DeviceDescriptor, DeviceLayout, FmRouting, Widget } from "@/generated";
import { devicesOfTrack, useProjectStore } from "@/state";
import { BUILTIN_DESCRIPTORS, type MockTransport } from "@/transport";
import { flush, renderWithMock, resetStores, store, trackByName } from "@/features/devices/testUtils";
import { useGestureSender } from "../../gesture";
import { DeviceLayoutView } from "../DeviceLayoutView";
import { fmDiagram } from "../fmDiagram";
import { boundParams } from "../model";

let mock: MockTransport | undefined;
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

const STACK: FmRouting = { modulators: [2, 4, 8, 0], carriers: 1 };
const PAIRS: FmRouting = { modulators: [2, 0, 8, 0], carriers: 5 };
const ADDITIVE: FmRouting = { modulators: [0, 0, 0, 0], carriers: 15 };
const FAN: FmRouting = { modulators: [14, 0, 0, 0], carriers: 1 };

describe("fmDiagram", () => {
  it("stacks modulators above their carriers", () => {
    const d = fmDiagram(STACK);
    expect(d.rows).toBe(4);
    const y = d.nodes.map((n) => n.y);
    // D on top, A (the only carrier) at the bottom, the output below it.
    expect(y[3]!).toBeLessThan(y[2]!);
    expect(y[2]!).toBeLessThan(y[1]!);
    expect(y[1]!).toBeLessThan(y[0]!);
    expect(d.outputY).toBeGreaterThan(y[0]!);
    expect(d.nodes.filter((n) => n.carrier).map((n) => n.name)).toEqual(["A"]);
    expect(d.edges).toEqual([
      { from: 1, to: 0 },
      { from: 2, to: 1 },
      { from: 3, to: 2 },
    ]);
  });

  it("puts parallel carriers on one row and fans modulators out", () => {
    const add = fmDiagram(ADDITIVE);
    expect(add.rows).toBe(1);
    expect(new Set(add.nodes.map((n) => n.y)).size).toBe(1);
    expect(add.nodes.map((n) => n.x)).toEqual([0.125, 0.375, 0.625, 0.875]);
    const pairs = fmDiagram(PAIRS);
    expect(pairs.rows).toBe(2);
    expect(pairs.nodes.filter((n) => n.carrier).map((n) => n.name)).toEqual(["A", "C"]);
    const fan = fmDiagram(FAN);
    expect(fan.edges.map((e) => e.from)).toEqual([1, 2, 3]);
    expect(new Set(fan.nodes.filter((n) => !n.carrier).map((n) => n.y)).size).toBe(1);
  });

  it("lays out the FM Synth's eleven algorithms inside the box", () => {
    const fm = BUILTIN_DESCRIPTORS.FmSynth;
    const widget = fm.layout!.sections.flatMap((s) => s.items).find((i) => i.widget.type === "FmAlgorithm")!.widget as Extract<Widget, { type: "FmAlgorithm" }>;
    expect(widget.algorithms).toHaveLength(11);
    const labels = fm.params.find((p) => p.id === widget.algorithm)!.labels!;
    expect(labels).toHaveLength(11);
    for (const r of widget.algorithms) {
      const d = fmDiagram(r);
      for (const n of d.nodes) {
        expect(n.x).toBeGreaterThan(0);
        expect(n.x).toBeLessThan(1);
        expect(n.y).toBeGreaterThan(0);
        expect(n.y).toBeLessThan(d.outputY);
      }
      // Every edge goes down (modulator above what it modulates).
      for (const e of d.edges) expect(d.nodes[e.from]!.y).toBeLessThan(d.nodes[e.to]!.y);
    }
  });
});

describe("FmAlgorithm widget", () => {
  async function renderPanel() {
    const widget: Widget = { type: "FmAlgorithm", algorithm: 0, algorithms: [STACK, PAIRS, FAN, ADDITIVE] };
    const layout: DeviceLayout = {
      sections: [{ id: "alg", title: "Algorithm", span: 1, columns: 1, items: [{ widget, size: "Medium", colspan: 1, label: null }] }],
    };
    mock = await renderWithMock(<Panel descriptor={{ ...BUILTIN_DESCRIPTORS.Synth, layout }} />);
    expect(boundParams(widget)).toEqual([0]);
    return screen.getByRole("region", { name: "panel" });
  }

  it("shows one diagram per algorithm and selects on click and arrow keys", async () => {
    const panel = await renderPanel();
    const group = within(panel).getByTestId("widget-fm-algorithm");
    const cells = within(group).getAllByRole("radio");
    expect(cells).toHaveLength(4);
    const initial = synth().params[0] ?? BUILTIN_DESCRIPTORS.Synth.params[0]!.default;
    expect(cells.map((c) => c.getAttribute("aria-checked"))).toEqual([0, 1, 2, 3].map((i) => String(i === initial)));
    expect(cells[1]!.querySelectorAll("rect")).toHaveLength(4);
    await act(async () => {
      fireEvent.click(cells[2]!);
    });
    await flush();
    expect(synth().params[0]).toBe(2);
    const now = within(within(panel).getByTestId("widget-fm-algorithm")).getAllByRole("radio");
    expect(now[2]!.getAttribute("aria-checked")).toBe("true");
    await act(async () => {
      fireEvent.keyDown(now[2]!, { key: "ArrowRight" });
    });
    await flush();
    expect(synth().params[0]).toBe(3);
    expect(panel.querySelector(".eth-widget--missing")).toBeNull();
  });
});
