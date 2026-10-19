// The guided-tour engine (docs/TUTORIALS.md): steps moving on with the app's state, input
// blocked outside the highlighted parts, the fallback of a missing part, completion kept
// across reloads, and the welcome screen's "do it again?".
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { create } from "zustand";
import type { EngineCommands } from "@/features/transport-bar/engine";
import { GetStarted } from "@/features/project/GetStarted";
import { zIndex } from "@/theme/tokens";
import { useProjectScreen } from "@/features/project/screenStore";
import { MockTransport } from "@/transport";
import { dimPath, holes, placeBubble } from "./geometry";
import { isTourDone, markTourDone, resetTutorialProgress } from "./progress";
import { backStep, canAdvance, finishTour, MISSING_GRACE_MS, nextStep, resetTour, startTour, tickTour, useTour } from "./store";
import { TourRoot } from "./TourRoot";
import { RECORD_AUDIO_TOUR } from "./tours";
import type { TourContext, TourDef } from "./types";

vi.mock("./start", async (orig) => ({ ...(await orig<typeof import("./start")>()), startTutorial: vi.fn(async () => undefined) }));
const { startTutorial } = await import("./start");

const app = create<{ open: boolean }>()(() => ({ open: false }));
const CTX: TourContext = { transport: null, env: { host: "mock", canRecord: true, devicesListable: true } };

const TOUR: TourDef = {
  id: "test",
  platforms: ["desktop", "web"],
  version: 2,
  // Any catalog keys (the text checked is the bodies').
  title: "tutorials.record.title",
  projectName: "tutorials.record.project_name",
  project: "empty",
  steps: [
    { id: "hello", title: "tutorials.record.intro.title", body: "Welcome" },
    {
      id: "open",
      targets: ["thing"],
      title: "tutorials.record.intro.title",
      body: "Click the thing",
      hint: "tutorials.record.intro.title",
      until: () => app.getState().open,
    },
    { id: "web-only", title: "tutorials.record.intro.title", body: "Only on the web", when: (c) => c.env.host === "wasm" },
    { id: "bye", title: "tutorials.record.done.title", body: "The end", fallback: "Gone" },
  ],
};

const step = () => TOUR.steps[useTour.getState().index]!.id;

/** jsdom lays nothing out: give `el` a box. */
function layOut(el: Element, x = 10, y = 10, w = 80, h = 24): void {
  el.getBoundingClientRect = () => ({ left: x, top: y, width: w, height: h, right: x + w, bottom: y + h, x, y, toJSON: () => ({}) }) as DOMRect;
}

beforeEach(() => {
  localStorage.clear();
  resetTutorialProgress(true);
  resetTour();
  app.setState({ open: false });
});
afterEach(() => resetTour());

describe("tour engine", () => {
  it("waits on a step until the app reaches its state, skipping steps that don't apply", () => {
    startTour(TOUR, CTX);
    expect(step()).toBe("hello");
    nextStep();
    expect(step()).toBe("open");
    expect(canAdvance()).toBe(false);
    tickTour(0, 1);
    expect(step()).toBe("open");
    app.setState({ open: true });
    tickTour(16, 1);
    // "web-only" is not shown on this host.
    expect(step()).toBe("bye");
  });

  it("skips an already satisfied waiting step going forward, but not going back", () => {
    app.setState({ open: true });
    startTour(TOUR, CTX);
    nextStep();
    expect(step()).toBe("bye");
    backStep();
    expect(step()).toBe("open");
    expect(useTour.getState().entered).toBe("back");
    // Satisfied on arrival: stays, and "next" is offered.
    tickTour(0, 1);
    expect(step()).toBe("open");
    expect(canAdvance()).toBe(true);
    // Undone then done again: moves on.
    app.setState({ open: false });
    tickTour(16, 1);
    app.setState({ open: true });
    tickTour(32, 1);
    expect(step()).toBe("bye");
  });

  it("falls back to a centered bubble with next when the highlighted part is missing", () => {
    startTour(TOUR, CTX);
    nextStep();
    tickTour(0, 0);
    expect(useTour.getState().missing).toBe(false);
    tickTour(MISSING_GRACE_MS - 1, 0);
    expect(canAdvance()).toBe(false);
    tickTour(MISSING_GRACE_MS + 1, 0);
    expect(useTour.getState().missing).toBe(true);
    expect(canAdvance()).toBe(true);
    // It shows up again: back to normal.
    tickTour(MISSING_GRACE_MS + 20, 1);
    expect(useTour.getState().missing).toBe(false);
  });

  it("remembers finished tours per version, across reloads", () => {
    const onEnd = vi.fn();
    startTour(TOUR, CTX, { onEnd });
    finishTour();
    expect(onEnd).toHaveBeenCalledWith("finished");
    expect(isTourDone(TOUR)).toBe(true);
    expect(isTourDone({ ...TOUR, version: 3 })).toBe(false);
    // A reload reads it back.
    resetTutorialProgress();
    expect(isTourDone(TOUR)).toBe(true);
    expect(JSON.parse(localStorage.getItem("eth.tutorials.done")!)).toHaveProperty("test@2");
  });

  it("an action ends the tour as done without coming back to the welcome screen", async () => {
    const run = vi.fn();
    const onEnd = vi.fn();
    startTour(TOUR, CTX, { onEnd });
    finishTour({ id: "go", label: "tutorials.record.done.title", run });
    await Promise.resolve();
    expect(run).toHaveBeenCalledWith(CTX);
    expect(onEnd).toHaveBeenCalledWith("action");
    expect(isTourDone(TOUR)).toBe(true);
  });
});

describe("geometry", () => {
  it("grows holes by the padding, inside the viewport", () => {
    expect(holes([{ x: 2, y: 10, w: 20, h: 10 }], { w: 100, h: 100 }, 6)).toEqual([{ x: 0, y: 4, w: 28, h: 22 }]);
    expect(dimPath({ w: 10, h: 10 }, [{ x: 1, y: 1, w: 2, h: 2 }])).toBe("M0 0h10v10h-10zM1 1h2v2h-2z");
  });

  it("puts the bubble where it fits, else in the middle", () => {
    const vp = { w: 1000, h: 800 };
    const bubble = { w: 300, h: 200 };
    expect(placeBubble({ x: 900, y: 10, w: 60, h: 30 }, bubble, vp).side).toBe("bottom");
    expect(placeBubble({ x: 10, y: 600, w: 60, h: 190 }, bubble, vp).side).toBe("right");
    expect(placeBubble(null, bubble, vp)).toEqual({ x: 350, y: 300, side: "center" });
    const placed = placeBubble({ x: 900, y: 10, w: 60, h: 30 }, bubble, vp);
    expect(placed.x + bubble.w).toBeLessThanOrEqual(vp.w);
  });
});

describe("TourRoot", () => {
  function setup() {
    const onThing = vi.fn(() => app.setState({ open: true }));
    const onOther = vi.fn();
    render(
      <>
        <button type="button" data-tour="thing" onClick={onThing}>
          Thing
        </button>
        <button type="button" onClick={onOther}>
          Other
        </button>
        <TourRoot />
      </>,
    );
    layOut(screen.getByRole("button", { name: "Thing" }));
    return { onThing, onOther };
  }

  it("shows the bubble, and only the highlighted part (when it waits for it) takes clicks", async () => {
    const { onThing, onOther } = setup();
    act(() => startTour(TOUR, CTX));
    const bubble = await screen.findByTestId("tour-bubble");
    expect(bubble).toHaveTextContent("Welcome");
    expect(bubble).toHaveTextContent("1 / 3");
    // A press outside the bubble does nothing, the highlighted part neither (it's not this step's).
    fireEvent.click(screen.getByRole("button", { name: "Other" }));
    fireEvent.click(screen.getByRole("button", { name: "Thing" }));
    expect(onOther).not.toHaveBeenCalled();
    expect(onThing).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "next" }));
    expect(bubble).toHaveTextContent("Click the thing");
    expect(screen.queryByRole("button", { name: "next" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Other" }));
    expect(onOther).not.toHaveBeenCalled();
    // The thing works now, and the step moves on.
    fireEvent.click(screen.getByRole("button", { name: "Thing" }));
    expect(onThing).toHaveBeenCalledTimes(1);
    await act(async () => {
      tickTour(performance.now(), 1);
    });
    expect(screen.getByTestId("tour-bubble")).toHaveTextContent("The end");
    expect(screen.getByRole("button", { name: "finish" })).toBeInTheDocument();
  });

  it("keeps the app's keys away and asks before leaving on Escape", async () => {
    setup();
    const onKey = vi.fn();
    window.addEventListener("keydown", onKey);
    act(() => startTour(TOUR, CTX));
    await screen.findByTestId("tour-bubble");
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
    expect(onKey).not.toHaveBeenCalled();
    fireEvent.keyDown(document.body, { key: "Escape" });
    expect(await screen.findByText(/exit the tutorial\?/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "keep going" }));
    expect(useTour.getState().tour).not.toBeNull();
    fireEvent.keyDown(document.body, { key: "Escape" });
    fireEvent.click(await screen.findByRole("button", { name: "exit" }));
    expect(useTour.getState().tour).toBeNull();
    // Gone: keys reach the app again.
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
    expect(onKey).toHaveBeenCalled();
    window.removeEventListener("keydown", onKey);
  });

  it("shows the fallback when the highlighted part never shows up", async () => {
    render(<TourRoot />);
    const tour: TourDef = {
      ...TOUR,
      steps: [
        { id: "x", targets: ["nowhere"], title: "tutorials.record.intro.title", body: "Look here", fallback: "Not on this platform", until: () => false },
      ],
    };
    vi.useFakeTimers({ toFake: ["performance", "requestAnimationFrame", "cancelAnimationFrame", "setTimeout"] });
    try {
      act(() => startTour(tour, CTX));
      expect(screen.getByTestId("tour-bubble")).toHaveTextContent("Look here");
      act(() => vi.advanceTimersByTime(MISSING_GRACE_MS + 200));
      expect(screen.getByTestId("tour-bubble")).toHaveTextContent("Not on this platform");
      expect(screen.getByRole("button", { name: "finish" })).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("tour layers", () => {
  // jsdom applies no CSS: read the stylesheets and resolve their z-index tokens.
  const css = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");
  const tour = css("./tutorials.css");
  const kit = css("../../kit/kit.css");
  /** The z-index of `selector` (a rule naming it alone), resolved through the zIndex tokens. */
  function z(sheet: string, selector: string): number {
    const esc = selector.replace(/[.-]/g, (c) => `\\${c}`);
    const rules = [...sheet.matchAll(new RegExp(`^${esc}\\s*\\{([^}]*)\\}`, "gm"))].map((m) => m[1]!);
    const decl = rules.map((r) => /z-index:\s*var\(--eth-z-([a-z-]+)\);/.exec(r)).find(Boolean) ?? null;
    expect(decl, `${selector} has a z-index token`).not.toBeNull();
    const key = decl![1]!.replace(/-([a-z])/g, (_, c: string) => c.toUpperCase()) as keyof typeof zIndex;
    expect(zIndex[key], `--eth-z-${decl![1]} is a token`).toBeDefined();
    return Number(zIndex[key]);
  }

  it("draws the highlight above menus and popovers, under the bubble; the dim stays under them", () => {
    const menu = z(kit, ".eth-popover");
    expect(z(tour, ".eth-tour")).toBeGreaterThan(Number(zIndex.dialog));
    expect(z(tour, ".eth-tour")).toBeLessThan(menu);
    expect(z(tour, ".eth-tour__rings")).toBeGreaterThan(menu);
    expect(z(tour, ".eth-tour__bubble")).toBeGreaterThan(z(tour, ".eth-tour__rings"));
  });

  it("puts the ring of a target inside a menu in the ring layer, outside the dim's stacking context", async () => {
    render(
      <>
        <div className="eth-popover" role="menu" aria-label="Project">
          <button type="button" role="menuitem" data-tour="thing">
            Import
          </button>
        </div>
        <TourRoot />
      </>,
    );
    layOut(screen.getByRole("menuitem", { name: "Import" }), 40, 60, 120, 24);
    const tour: TourDef = { ...TOUR, steps: [{ id: "entry", targets: ["thing"], title: "tutorials.record.intro.title", body: "This one", interactive: true }] };
    act(() => startTour(tour, CTX));
    const ring = await screen.findByTestId("tour-ring");
    expect(ring.parentElement).toHaveClass("eth-tour__rings");
    expect(screen.getByTestId("tour")).not.toContainElement(ring);
    expect(ring.closest(".eth-popover")).toBeNull();
    // Around the item (plus the hole's padding).
    expect(parseFloat(ring.style.left)).toBeLessThanOrEqual(40);
    expect(parseFloat(ring.style.left) + parseFloat(ring.style.width)).toBeGreaterThanOrEqual(160);
  });
});

describe("GetStarted", () => {
  const commands = (): EngineCommands => ({
    transport: new MockTransport({ timers: "manual" }),
    send: async () => undefined,
    error: null,
    clearError: () => undefined,
  });

  afterEach(() => useProjectScreen.setState({ launching: false }));

  it("starts a tour, and asks before running a finished one again", async () => {
    // Launching (the engine loading, #646): shown, but waiting.
    useProjectScreen.setState({ launching: true });
    render(
      <>
        <GetStarted commands={commands()} onStarted={() => undefined} />
        <TourRoot />
      </>,
    );
    expect(screen.getByRole("button", { name: "record audio" })).toBeDisabled();
    act(() => useProjectScreen.setState({ launching: false }));
    // Not written yet.
    expect(screen.getByText("make your first track").closest("li")).toHaveAttribute("data-soon");
    fireEvent.click(screen.getByRole("button", { name: "record audio" }));
    await vi.waitFor(() => expect(startTutorial).toHaveBeenCalledTimes(1));
    expect(vi.mocked(startTutorial).mock.calls[0]![0]).toBe(RECORD_AUDIO_TOUR);

    act(() => markTourDone(RECORD_AUDIO_TOUR));
    const done = screen.getByRole("button", { name: "record audio (completed)" });
    fireEvent.click(done);
    expect(screen.getByTestId("tutorial-confirm-record-audio")).toHaveTextContent("you already completed this tutorial. do it again?");
    fireEvent.click(screen.getByRole("button", { name: "cancel" }));
    expect(startTutorial).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "record audio (completed)" }));
    fireEvent.click(screen.getByRole("button", { name: "do it again" }));
    await vi.waitFor(() => expect(startTutorial).toHaveBeenCalledTimes(2));
  });
});
