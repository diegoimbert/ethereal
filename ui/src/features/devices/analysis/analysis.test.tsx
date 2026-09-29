/** UI side of the analysis channel: watch refcounting, visibility, latest-frame store, hook. */
import { act, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { AnalysisData, Command, Event } from "@/generated";
import { Emitter, TransportContext, type EngineTransport } from "@/transport";
import { AnalysisStore, useDeviceAnalysis } from ".";

class FakeTransport {
  readonly kind = "mock" as const;
  readonly sent: Command[] = [];
  readonly events = new Emitter<Event>();
  connect = () => Promise.reject(new Error("unused"));
  send(c: Command) {
    this.sent.push(c);
    return Promise.resolve({ type: "Unit" } as const);
  }
  onEvent(l: (e: Event) => void) {
    return this.events.on(l);
  }
  subscribePlayhead = () => () => {};
  subscribeMeters = () => () => {};
  dispose() {}
  watches() {
    return this.sent.map((c) => (c.domain === "Analysis" ? `${c.command.type}:${c.command.device}` : "?"));
  }
}

const fake = () => new FakeTransport();
const asTransport = (t: FakeTransport) => t as unknown as EngineTransport;
const spectrum = (db: number, stage: "Pre" | "Post" = "Post"): AnalysisData => ({ type: "Spectrum", min_hz: 20, max_hz: 20000, bins_db: [db, db], stage });
const frame = (device: string, data: AnalysisData): Event => ({ type: "Analysis", event: { type: "Frame", device, data } });

describe("AnalysisStore", () => {
  it("refcounts watches: one Watch on the first widget, one Unwatch after the last", () => {
    const t = fake();
    const s = new AnalysisStore(asTransport(t));
    const a = s.watch("d1");
    const b = s.watch("d1");
    expect(t.watches()).toEqual(["Watch:d1"]);
    a();
    a(); // idempotent
    expect(s.watchers("d1")).toBe(1);
    b();
    expect(t.watches()).toEqual(["Watch:d1", "Unwatch:d1"]);
  });

  it("keeps the latest frame per kind and stage, only for watched devices", () => {
    const t = fake();
    const s = new AnalysisStore(asTransport(t));
    t.events.emit(frame("d1", spectrum(-10)));
    expect(s.frames("d1")).toEqual([]);
    const off = s.watch("d1");
    t.events.emit(frame("d1", spectrum(-10)));
    t.events.emit(frame("d1", spectrum(-20, "Pre")));
    const before = s.frames("d1");
    t.events.emit(frame("d1", spectrum(-5)));
    const after = s.frames("d1");
    expect(after).not.toBe(before);
    expect(after).toHaveLength(2);
    expect(after.find((f) => f.type === "Spectrum" && f.stage === "Post")).toEqual(spectrum(-5));
    off();
    expect(s.frames("d1")).toEqual([]);
  });

  it("releases watches while the page is hidden", () => {
    const t = fake();
    const s = new AnalysisStore(asTransport(t));
    const off = s.watch("d1");
    s.setVisible(false);
    expect(s.engineWatching("d1")).toBe(false);
    s.setVisible(true);
    expect(t.watches()).toEqual(["Watch:d1", "Unwatch:d1", "Watch:d1"]);
    s.setVisible(false);
    off();
    s.setVisible(true);
    expect(t.watches()).toEqual(["Watch:d1", "Unwatch:d1", "Watch:d1", "Unwatch:d1"]);
  });
});

function Probe({ device }: { device: string }) {
  const frames = useDeviceAnalysis(device);
  const f = frames.find((d) => d.type === "Tuner");
  return <span data-testid="probe">{f && f.type === "Tuner" ? String(f.hz) : "none"}</span>;
}

describe("useDeviceAnalysis", () => {
  it("watches while mounted and re-renders on frames", () => {
    const t = fake();
    const { unmount } = render(
      <TransportContext.Provider value={{ transport: asTransport(t), connection: { state: "connected" } as never }}>
        <Probe device="d9" />
      </TransportContext.Provider>,
    );
    expect(t.watches()).toEqual(["Watch:d9"]);
    expect(screen.getByTestId("probe").textContent).toBe("none");
    act(() => t.events.emit(frame("d9", { type: "Tuner", hz: 440, note: 69, cents: 0, confidence: 1, level_db: -12 })));
    expect(screen.getByTestId("probe").textContent).toBe("440");
    unmount();
    expect(t.watches()).toEqual(["Watch:d9", "Unwatch:d9"]);
  });

  it("returns no frames outside a transport", () => {
    render(<Probe device="d1" />);
    expect(screen.getByTestId("probe").textContent).toBe("none");
  });
});
