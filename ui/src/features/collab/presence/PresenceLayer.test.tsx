import { act, render, waitFor } from "@testing-library/react";
import { useRef } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Command, Presence, ReplyValue, Track } from "@/generated";
import type { Row } from "@/features/arrangement/layout";
import { arrangementView, useArrangementUi } from "@/features/arrangement/uiStore";
import { MockTransport, TransportProvider, type SendOptions } from "@/transport";
import { useCollabStore } from "../store";
import { setFollowing, useLocalPresence } from "./local";
import { usePointerStore } from "./pointers";
import { PresenceLayer } from "./PresenceLayer";

class Recorder extends MockTransport {
  sent: Command[] = [];
  override async send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    this.sent.push(command);
    return super.send(command, opts);
  }
  pointers() {
    return this.sent.flatMap((c) => (c.domain === "Collab" && c.command.type === "SetPointer" ? [c.command.pointer] : []));
  }
}

const track = (id: string, parent: string | null = null) => ({ id, parent, kind: "Midi", name: id }) as unknown as Track;
// Local layout: rows "a" (56 px) and "b" (100 px) in content px.
const ROWS: Row[] = [
  { track: track("a"), depth: 0, y: 0, laneHeight: 56, height: 56 },
  { track: track("b"), depth: 0, y: 56, laneHeight: 100, height: 100 },
];
const RULER = 30;
const HEADER = 200;

/** Give an element a fixed box (jsdom has no layout). */
function box(el: HTMLElement, top: number, height: number, width = 1000) {
  el.getBoundingClientRect = () => ({ top, bottom: top + height, left: 0, right: width, width, height, x: 0, y: top }) as DOMRect;
  Object.defineProperty(el, "clientWidth", { configurable: true, value: width });
  Object.defineProperty(el, "clientHeight", { configurable: true, value: height });
}

function Harness() {
  const root = useRef<HTMLDivElement>(null);
  const scroll = useRef<HTMLDivElement>(null);
  return (
    <div
      ref={(el) => {
        root.current = el;
        if (el) box(el, 0, RULER + 400);
      }}
    >
      <div
        className="eth-arr__top"
        ref={(el) => {
          if (el) box(el, 0, RULER);
        }}
      />
      <div
        ref={(el) => {
          scroll.current = el;
          if (el) box(el, RULER, 400);
        }}
        data-testid="scroll"
      />
      <PresenceLayer rootRef={root} scrollRef={scroll} rows={ROWS} masterRow={null} />
    </div>
  );
}

const peer = (site: string, name: string, state: Partial<Presence["state"]> = {}): Presence => ({
  site,
  actor: null,
  name,
  color: 0x5cffe8,
  state: { cursor: null, selected_tracks: [], selected_clips: [], selected_notes: [], selected_devices: [], view: null, ...state },
});

async function setup() {
  const mock = new Recorder({ timers: "manual", seed: 5 });
  const view = render(
    <TransportProvider transport={mock}>
      <Harness />
    </TransportProvider>,
  );
  return { mock, root: view.container.firstElementChild as HTMLElement, scroll: view.getByTestId("scroll"), unmount: view.unmount };
}

const move = (el: HTMLElement, clientX: number, clientY: number) =>
  act(() => {
    el.dispatchEvent(new MouseEvent("pointermove", { clientX, clientY, bubbles: true }));
  });

beforeEach(() => {
  useArrangementUi.getState().setHeaderWidth(HEADER);
  arrangementView.getState().setWidth(800);
  arrangementView.getState().setViewport({ pxPerBeat: 20, scrollBeats: 0 });
  useCollabStore.setState({ status: { type: "Online", session: "jam", site: "1" }, peers: [peer("2", "Ada")] });
});

afterEach(() => {
  useCollabStore.getState().reset();
  setFollowing(null);
  useLocalPresence.setState({ activity: null, viewport: null });
});

describe("PresenceLayer", () => {
  it("publishes the pointer in song coordinates and clears it on leave and blur", async () => {
    const { mock, root } = await setup();
    // 100 px into the lanes = beat 5; 3/4 down row "b" (top 30 + 56).
    move(root, HEADER + 100, RULER + 56 + 75);
    await waitFor(() => expect(mock.pointers().at(-1)).toEqual({ beats: 5, track: "b", y: 0.75 }));
    // Over the ruler: no track.
    move(root, HEADER + 40, 10);
    await waitFor(() => expect(mock.pointers().at(-1)).toEqual({ beats: 2, track: null, y: 0 }));
    act(() => {
      root.dispatchEvent(new MouseEvent("pointerleave"));
    });
    expect(mock.pointers().at(-1)).toBeNull();
    move(root, HEADER + 20, RULER + 10);
    await waitFor(() => expect(mock.pointers().at(-1)).toMatchObject({ track: "a" }));
    act(() => {
      window.dispatchEvent(new Event("blur"));
    });
    expect(mock.pointers().at(-1)).toBeNull();
  });

  it("coalesces moves into one update per frame", async () => {
    const { mock, root } = await setup();
    for (let i = 0; i < 10; i++) move(root, HEADER + i * 20, RULER + 10);
    await waitFor(() => expect(mock.pointers().length).toBeGreaterThan(0));
    expect(mock.pointers()).toEqual([{ beats: 9, track: "a", y: 10 / 56 }]);
  });

  it("clears the pointer on unmount", async () => {
    const { mock, root, unmount } = await setup();
    move(root, HEADER + 20, RULER + 10);
    await waitFor(() => expect(mock.pointers()).toHaveLength(1));
    unmount();
    expect(mock.pointers().at(-1)).toBeNull();
  });

  it("publishes the viewport", async () => {
    const { scroll } = await setup();
    await waitFor(() => expect(useLocalPresence.getState().viewport).toEqual({ start: 0, end: 40, top_track: "a", top_offset: 0 }));
    act(() => {
      scroll.scrollTop = 56 + 25;
      scroll.dispatchEvent(new Event("scroll"));
    });
    await waitFor(() => expect(useLocalPresence.getState().viewport).toMatchObject({ top_track: "b", top_offset: 0.25 }));
  });

  it("follows a peer's viewport and stops on a local scroll, zoom or Escape", async () => {
    const { root, scroll } = await setup();
    const viewport = { start: 8, end: 16, top_track: "b", top_offset: 0.5 };
    act(() => useCollabStore.setState({ peers: [peer("2", "Ada", { viewport })] }));
    act(() => setFollowing("2"));
    // 8 beats fill the 800 px lanes; row "b" half scrolled past.
    expect(arrangementView.getState()).toMatchObject({ pxPerBeat: 100, scrollBeats: 8 });
    expect(scroll.scrollTop).toBe(56 + 50);
    // The leader moves: the view follows.
    act(() => useCollabStore.setState({ peers: [peer("2", "Ada", { viewport: { ...viewport, start: 0, top_track: "a", top_offset: 0 } })] }));
    expect(arrangementView.getState()).toMatchObject({ pxPerBeat: 50, scrollBeats: 0 });
    expect(scroll.scrollTop).toBe(0);

    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(useLocalPresence.getState().following).toBeNull();

    act(() => setFollowing("2"));
    act(() => {
      root.dispatchEvent(new WheelEvent("wheel", { deltaY: 10, bubbles: true }));
    });
    expect(useLocalPresence.getState().following).toBeNull();

    act(() => setFollowing("2"));
    act(() => arrangementView.getState().zoomBy(2));
    expect(useLocalPresence.getState().following).toBeNull();

    // The leader leaves: following stops.
    act(() => setFollowing("2"));
    act(() => useCollabStore.setState({ peers: [] }));
    expect(useLocalPresence.getState().following).toBeNull();
  });

  it("draws peers' pointers at their song position in this layout", async () => {
    const { root } = await setup();
    act(() => usePointerStore.getState().onPointer("2", { beats: 10, track: "b", y: 0.5 }, performance.now() - 1000));
    const el = await waitFor(() => {
      const e = root.querySelector<HTMLElement>('[data-testid="peer-pointer"]');
      expect(e?.dataset.hidden).toBe("false");
      return e!;
    });
    expect(el.dataset.peer).toBe("Ada");
    // x = header + 10 beats * 20 px; y = ruler + row "b" top + half its height.
    expect(el.style.transform).toBe(`translate(${HEADER + 200}px, ${RULER + 56 + 50}px)`);
    // With an activity, the label says so.
    act(() => useCollabStore.setState({ peers: [peer("2", "Ada", { activity: { kind: "Dragging", target: { type: "Selection" } } })] }));
    expect(el.textContent).toBe("Ada · dragging");
    // Scrolled out to the right: pinned to the edge.
    act(() => usePointerStore.getState().onPointer("2", { beats: 500, track: "b", y: 0.5 }, performance.now() - 1000));
    await waitFor(() => expect(el.dataset.edge).toBe("right"));
    // A deleted track: hidden. A clear removes it.
    act(() => usePointerStore.getState().onPointer("2", { beats: 1, track: "gone", y: 0 }, performance.now() - 1000));
    await waitFor(() => expect(el.dataset.hidden).toBe("true"));
    act(() => usePointerStore.getState().onPointer("2", null));
    expect(root.querySelector('[data-testid="peer-pointer"]')).toBeNull();
  });
});
