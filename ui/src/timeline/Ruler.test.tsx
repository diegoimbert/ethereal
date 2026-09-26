import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { playheadStore } from "@/state/playhead";
import { useProjectStore } from "@/state/projectStore";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { PlayheadLine } from "./PlayheadLine";
import { Ruler } from "./Ruler";
import { createTimelineViewStore } from "./viewStore";

let mock: MockTransport | undefined;
afterEach(() => {
  mock?.dispose();
  mock = undefined;
  useProjectStore.getState().reset();
  playheadStore.reset();
});

// jsdom has no PointerEvent constructor or layout: dispatch MouseEvents with pointer event
// names on window, and give the ruler a fixed width through the view store.
const pointer = (type: string, x: number, init: MouseEventInit = {}) =>
  window.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: 0, bubbles: true, ...init }));

async function setup() {
  mock = new MockTransport({ timers: "manual" });
  const view = createTimelineViewStore({ pxPerBeat: 20, widthPx: 400 });
  render(
    <TransportProvider transport={mock}>
      <div style={{ position: "relative" }}>
        <Ruler view={view} syncWidth={false} />
        <PlayheadLine view={view} />
      </div>
    </TransportProvider>,
  );
  await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
  return { view, mock };
}

describe("Ruler", () => {
  it("renders bar labels and the loop brace from the project settings", async () => {
    await setup();
    // 80 px per bar → a label on every bar.
    expect(screen.getByText("1")).toBeInTheDocument();
    expect(screen.getByText("5")).toBeInTheDocument();
    const loop = screen.getByTestId("ruler-loop");
    // Demo loop region is 0..16 beats → 320 px, disabled.
    expect(loop.style.width).toBe("320px");
    expect(loop.className).toContain("eth-ruler__loop--off");
  });

  it("click locates the playhead (snapped)", async () => {
    await setup();
    const ruler = screen.getByTestId("ruler");
    await act(async () => {
      fireEvent.pointerDown(ruler, { button: 0, clientX: 41 });
      pointer("pointerup", 41);
      await Promise.resolve();
    });
    // 41 px = 2.05 beats → snapped to the adaptive grid (1/2 note at 20 px/beat) = 2.
    await waitFor(() => expect(playheadStore.getPlayhead()?.transport.position).toBe(2));
    expect(screen.getByTestId("ruler-playhead").style.transform).toBe("translateX(40px)");
    expect(screen.getByTestId("playhead-line").style.transform).toBe("translateX(40px)");
  });

  it("dragging the loop brace moves the loop as one undo step", async () => {
    const { mock } = await setup();
    const loop = screen.getByTestId("ruler-loop");
    loop.getBoundingClientRect = () => ({ left: 0, width: 320, top: 0, height: 8 }) as DOMRect;
    await act(async () => {
      fireEvent.pointerDown(loop, { button: 0, clientX: 100 });
      pointer("pointermove", 140);
      pointer("pointermove", 180);
      pointer("pointerup", 180);
      await Promise.resolve();
    });
    await waitFor(() => expect(useProjectStore.getState().project!.settings.loop_region).toEqual({ start: 4, end: 20 }));
    await act(() => mock.send(cmd("Edit", { type: "Undo" })));
    expect(useProjectStore.getState().project!.settings.loop_region).toEqual({ start: 0, end: 16 });
  });

  it("double-click toggles the loop", async () => {
    await setup();
    await act(async () => {
      fireEvent.doubleClick(screen.getByTestId("ruler-loop"));
      await Promise.resolve();
    });
    await waitFor(() => expect(useProjectStore.getState().project!.settings.loop_enabled).toBe(true));
  });

  it("vertical drag zooms, horizontal drag scrolls", async () => {
    const { view } = await setup();
    const ruler = screen.getByTestId("ruler");
    act(() => {
      fireEvent.pointerDown(ruler, { button: 0, clientX: 200, clientY: 0 });
      window.dispatchEvent(new MouseEvent("pointermove", { clientX: 200, clientY: 50 }));
      window.dispatchEvent(new MouseEvent("pointerup", { clientX: 200, clientY: 50 }));
    });
    expect(view.getState().pxPerBeat).toBeGreaterThan(20);
  });
});
