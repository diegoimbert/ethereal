/**
 * The analysis channel end to end in the UI: a Spectrum / Tuner device panel rendered by the
 * shared renderer watches its device while mounted, and draws the mock's simulated frames
 * (Range floor, UI peak hold, tuner note).
 */
import { act, screen } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { BuiltinDeviceType, Device, DeviceLayout } from "@/generated";
import { devicesOfTrack, useProjectStore } from "@/state";
import { BUILTIN_DESCRIPTORS, cmd, newId } from "@/transport";
import { type MockTransport } from "@/transport/mock";
import { renderWithMock, resetStores, store, trackByName } from "@/features/devices/testUtils";
import { useGestureSender } from "../gesture";
import { DeviceLayoutView } from "../layout/DeviceLayoutView";
import { analysisStore } from ".";

let mock: MockTransport | undefined;
const rect = { left: 0, top: 0, right: 400, bottom: 120, width: 400, height: 120, x: 0, y: 0, toJSON: () => ({}) };
beforeAll(() => {
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue(rect as DOMRect);
});
afterAll(() => vi.restoreAllMocks());
afterEach(() => {
  resetStores(mock);
  mock = undefined;
});

function Panel({ id, type, layout }: { id: string; type: BuiltinDeviceType; layout?: DeviceLayout }) {
  const sender = useGestureSender();
  const device = useProjectStore((s) => s.project?.devices[id]) as Device | undefined;
  if (!device) return null;
  return (
    <section aria-label="panel">
      <DeviceLayoutView device={device} descriptor={{ ...BUILTIN_DESCRIPTORS[type]!, ...(layout ? { layout } : {}) }} sender={sender} />
    </section>
  );
}

/** Inserts a `type` device on the Keys track and renders its panel. */
async function mount(type: BuiltinDeviceType, layout?: DeviceLayout) {
  const id = newId();
  function Root() {
    return <Panel id={id} type={type} layout={layout} />;
  }
  mock = await renderWithMock(<Root />);
  const track = trackByName("Keys").id;
  await act(() => mock!.send(cmd("Device", { type: "Insert", id, track, device: { type: "Builtin", device: { type } } as never, before: null })));
  expect(devicesOfTrack(store().project!, track).some((d) => d.id === id)).toBe(true);
  return id;
}

const tick = (ms: number) => act(() => mock!.tick(ms));

describe("analysis widgets", () => {
  it("the spectrum panel watches its device and draws frames with the Range floor", async () => {
    const id = await mount("SpectrumAnalyzer");
    expect(screen.getByTestId("widget-spectrum").textContent).toContain("no signal");
    await tick(100);
    const plot = screen.getByTestId("widget-spectrum");
    expect(plot.querySelector("polyline.eth-plot__line")?.getAttribute("points")?.split(" ").length).toBe(256);
    // Default Range -90 dB: axis labels at quarters of 0..-90.
    expect([...plot.querySelectorAll('[data-axis="db"]')].map((e) => e.textContent)).toEqual(["-23", "-45", "-68"]);
    expect([...plot.querySelectorAll('[data-axis="hz"]')].map((e) => e.textContent)).toEqual(["100", "1k", "10k"]);
    expect(plot.querySelector('[data-testid="spectrum-peak"]')).toBeNull();
    await act(() => mock!.send(cmd("Device", { type: "SetParam", device: id, param: 3, value: -60 })));
    await act(() => mock!.send(cmd("Device", { type: "SetParam", device: id, param: 5, value: 1 })));
    await tick(100);
    expect([...plot.querySelectorAll('[data-axis="db"]')].map((e) => e.textContent)).toEqual(["-15", "-30", "-45"]);
    const peak = plot.querySelector('[data-testid="spectrum-peak"]');
    expect(peak?.getAttribute("points")?.split(" ").length).toBe(256);
  });

  it("a gain-reduction meter is empty until its first frame", async () => {
    const id = await mount("Tuner", {
      sections: [
        {
          id: "gr",
          title: null,
          span: 1,
          columns: 1,
          items: [{ widget: { type: "Meter", index: 0, min_db: -24, max_db: 0 }, size: "Medium", colspan: 1, label: "gr" }],
        },
      ],
    });
    const fill = () => (screen.getByTestId("widget-meter").querySelector(".eth-level__fill") as HTMLElement).style.height;
    expect(fill()).toBe("0%");
    act(() => analysisStore(mock!).handle({ type: "Analysis", event: { type: "Frame", device: id, data: { type: "Levels", values: [-6] } } }));
    expect(fill()).toBe("25%");
  });

  it("the tuner panel shows the simulated note", async () => {
    await mount("Tuner");
    await tick(100);
    const tuner = screen.getByTestId("widget-tuner");
    expect(tuner.querySelector(".eth-tuner__note")?.textContent).toBe("A2");
    expect(tuner.querySelector(".eth-tuner__readout")?.textContent).toMatch(/ct · 11\d\.\d hz/);
  });

  it("the loudness panel shows the readouts, the history and the target line", async () => {
    const id = await mount("Loudness");
    const panel = () => screen.getByTestId("widget-loudness");
    const value = (name: string) => panel().querySelector(`[data-readout="${name}"] .eth-loudness__value`)?.textContent;
    expect(value("integrated")).toBe("–");
    expect(panel().textContent).toContain("no signal");
    await tick(5000);
    expect(Number(value("integrated"))).toBeCloseTo(-15.2, 1);
    expect(value("true peak")).toBe("-1.4");
    // -1.4 dBTP is under the -1 ceiling: not flagged.
    expect(panel().querySelector('[data-readout="true peak"]')?.className).not.toContain("danger");
    const points = panel().querySelector('[data-testid="loudness-short-term"]')?.getAttribute("points")?.split(" ").length ?? 0;
    expect(points).toBeGreaterThan(10);
    // The graph follows the target (the line stays put, the axis moves): -14 by default.
    const axis = () => [...panel().querySelectorAll('[data-axis="lufs"]')].map((e) => e.textContent);
    expect(axis()).toEqual(["-14", "-23", "-32"]);
    await act(() => mock!.send(cmd("Device", { type: "SetParam", device: id, param: 0, value: -9 })));
    expect(axis()).toEqual(["-9", "-18", "-27"]);
    // A frame in the engine's layout with a hot true peak.
    act(() =>
      analysisStore(mock!).handle({
        type: "Analysis",
        event: { type: "Frame", device: id, data: { type: "Levels", values: [-10, -11, -12, 5, 0.3, -2, 30, 0.1, 2, -10, -10, -11, -11] } },
      }),
    );
    expect(value("integrated")).toBe("-12.0");
    expect(value("range")).toBe("5.0");
    expect(panel().querySelector('[data-readout="true peak"]')?.className).toContain("danger");
  });

  it("the maximizer panel shows the history, the ceiling and the readouts", async () => {
    const id = await mount("Maximizer");
    const panel = () => screen.getByTestId("widget-maximizer");
    const value = (name: string) => panel().querySelector(`[data-readout="${name}"] .eth-maximizer__value`)?.textContent;
    expect(panel().textContent).toContain("no signal");
    await act(() => mock!.send(cmd("Device", { type: "SetParam", device: id, param: 0, value: -8 })));
    await tick(2000);
    expect(value("boost")).toBe("8.0");
    for (const t of ["maximizer-input", "maximizer-output", "maximizer-reduction"]) {
      expect(panel().querySelector(`[data-testid="${t}"]`)?.getAttribute("points")?.split(" ").length, t).toBeGreaterThan(20);
    }
    expect(Number(value("deepest"))).toBeGreaterThan(1);
    // The default -1 dB ceiling sits a sixth of the way to the -6 dB grid line.
    const y = (sel: string) =>
      Number(
        panel()
          .querySelector(sel)
          ?.getAttribute(sel.includes("ceiling") ? "y1" : "y"),
      );
    expect(y('[data-testid="maximizer-ceiling"]') * 6).toBeCloseTo(y('[data-axis="db"]'), 3);
    // An output over the ceiling is flagged.
    act(() =>
      analysisStore(mock!).handle({
        type: "Analysis",
        event: { type: "Frame", device: id, data: { type: "Levels", values: [-3, -4, -0.5, -0.5, -6, 0.02, 0] } },
      }),
    );
    expect(value("reduction")).toBe("3.0");
    expect(panel().querySelector('[data-readout="true peak"]')?.className).toContain("danger");
  });
});
