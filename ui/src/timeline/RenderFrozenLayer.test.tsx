import { act, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { playheadStore } from "@/state/playhead";
import { useProjectStore } from "@/state/projectStore";
import { cmd, TransportProvider } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { RenderFrozenLayer } from "./RenderFrozenLayer";
import { Ruler } from "./Ruler";
import { createTimelineViewStore } from "./viewStore";

let mock: MockTransport | undefined;
afterEach(() => {
  mock?.dispose();
  mock = undefined;
  useProjectStore.getState().reset();
  playheadStore.reset();
});

async function setup() {
  mock = new MockTransport({ timers: "manual" });
  const view = createTimelineViewStore({ pxPerBeat: 20, widthPx: 400 });
  render(
    <TransportProvider transport={mock}>
      <Ruler view={view} syncWidth={false} />
      <div data-testid="lanes" style={{ position: "relative" }}>
        <RenderFrozenLayer view={view} />
      </div>
    </TransportProvider>,
  );
  await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
  return { view, mock };
}

const exportCustom = (m: MockTransport, start: number, end: number) =>
  m.send(
    cmd("Export", {
      type: "Render",
      job: "j",
      request: {
        range: { type: "Custom", start, end },
        format: { container: "Wav", bit_depth: "Int16", sample_rate: null },
        mode: { type: "Mix" },
        normalize: false,
        tail_seconds: 0,
        name: null,
      },
    }),
  );

describe("RenderFrozenLayer", () => {
  it("freezes the timeline past the rendered part, the boundary moving as the render goes", async () => {
    const { view, mock } = await setup();
    expect(screen.queryByTestId("render-frozen")).toBeNull();

    await act(() => exportCustom(mock, 2, 10));
    // On the ruler (labelled) and on the lanes.
    const afters = await screen.findAllByTestId("render-frozen-after");
    expect(afters).toHaveLength(2);
    const [rulerAfter, lanesAfter] = afters as [HTMLElement, HTMLElement];
    expect(rulerAfter).toHaveTextContent("not rendered yet");
    expect(lanesAfter).not.toHaveTextContent("not rendered yet");
    // Nothing rendered yet: frozen from the start (beat 2 = 40 px), and before it.
    expect(lanesAfter.style.left).toBe("40px");
    const before = screen.getAllByTestId("render-frozen-before")[1]!;
    expect(before.style.width).toBe("40px");
    expect(lanesAfter).toHaveAttribute("data-edge");

    // Half rendered: beat 6.
    act(() => mock.tick(16));
    await waitFor(() => expect(lanesAfter.style.left).toBe("120px"));

    // Panning moves it with the view; past the right edge it's out of view.
    act(() => view.getState().setViewport({ scrollBeats: 4 }));
    expect(lanesAfter.style.left).toBe("40px");
    expect(before.hidden).toBe(true);

    // Done: everything plays again.
    act(() => mock.tick(16));
    await waitFor(() => expect(screen.queryByTestId("render-frozen")).toBeNull());
  });
});
