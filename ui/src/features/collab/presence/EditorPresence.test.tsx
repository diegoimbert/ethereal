import { act, render, screen, waitFor } from "@testing-library/react";
import { useRef } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Command, Presence, ReplyValue } from "@/generated";
import { MockTransport, TransportProvider, type SendOptions } from "@/transport";
import { useCollabStore } from "../store";
import { EditingPeers, EditorPresence, type EditorCursorMapping } from "./EditorPresence";
import { presenceV2Fields, useLocalPresence } from "./local";
import { usePointerStore } from "./pointers";
import { INTERP_DELAY_MS } from "./interp";

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

// A simple editor: 10 px per beat, 10 px per key with pitch 127 at the top.
const MAPPING: EditorCursorMapping = {
  fromScreen: (x, y) => ({ beats: x / 10, pitch: 128 - y / 10 }),
  toScreen: (beats, pitch) => ({ x: beats * 10, y: (128 - pitch) * 10 }),
};

function Harness({ clip }: { clip: string }) {
  const grid = useRef<HTMLDivElement>(null);
  const mapping = useRef(MAPPING);
  return (
    <div
      data-testid="grid"
      ref={(el) => {
        grid.current = el;
        if (el) el.getBoundingClientRect = () => ({ top: 0, left: 0, right: 800, bottom: 600, width: 800, height: 600, x: 0, y: 0 }) as DOMRect;
      }}
    >
      <EditorPresence clip={clip} gridRef={grid} mapping={mapping} />
      <EditingPeers clip={clip} />
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

async function setup(clip = "c1") {
  const mock = new Recorder({ timers: "manual", seed: 5 });
  const view = render(
    <TransportProvider transport={mock}>
      <Harness clip={clip} />
    </TransportProvider>,
  );
  return { mock, grid: view.getByTestId("grid"), unmount: view.unmount };
}

beforeEach(() => {
  useCollabStore.setState({ status: { type: "Online", session: "jam", site: "1" }, peers: [peer("2", "Ada")] });
});

afterEach(() => {
  useCollabStore.getState().reset();
  usePointerStore.getState().clear();
  useLocalPresence.setState({ editingClip: null });
});

describe("EditorPresence", () => {
  it("shares the edited clip while the piano roll shows it", async () => {
    const { unmount } = await setup("c1");
    expect(useLocalPresence.getState().editingClip).toBe("c1");
    expect(presenceV2Fields()).toMatchObject({ editing_clip: "c1" });
    unmount();
    expect(useLocalPresence.getState().editingClip).toBeNull();
    expect(presenceV2Fields()).not.toHaveProperty("editing_clip");
  });

  it("publishes the pointer in the clip's content coordinates and clears it on leave", async () => {
    const { mock, grid } = await setup("c1");
    act(() => {
      grid.dispatchEvent(new MouseEvent("pointermove", { clientX: 25, clientY: 675 }));
    });
    // x 25 → beat 2.5; y 675 → pitch 128 - 67.5 = 60.5 (the middle of C3's key).
    await waitFor(() =>
      expect(mock.pointers().at(-1)).toEqual({ beats: 0, track: null, y: 0, editor: { clip: "c1", beats: 2.5, pitch: 60.5 } }),
    );
    act(() => {
      grid.dispatchEvent(new MouseEvent("pointerleave"));
    });
    expect(mock.pointers().at(-1)).toBeNull();
  });

  it("draws peers' pointers on the same clip only, and says who is editing it", async () => {
    useCollabStore.setState({ peers: [peer("2", "Ada", { editing_clip: "c1" }), peer("3", "Bob", { editing_clip: "other" })] });
    await setup("c1");
    expect(screen.getByRole("status", { name: "Ada is editing this clip" })).toBeInTheDocument();
    const t0 = performance.now() - 10 * INTERP_DELAY_MS;
    act(() => {
      usePointerStore.getState().onPointer("2", { beats: 0, track: null, y: 0, editor: { clip: "c1", beats: 4, pitch: 64 } }, t0);
      usePointerStore.getState().onPointer("3", { beats: 0, track: null, y: 0, editor: { clip: "other", beats: 1, pitch: 60 } }, t0);
    });
    await waitFor(() => {
      const ada = document.querySelector<HTMLElement>('[data-testid="editor-peer-pointer"][data-peer="Ada"]')!;
      expect(ada.dataset.hidden).toBe("false");
      expect(ada.style.transform).toBe("translate(40px, 640px)");
    });
    expect(document.querySelector<HTMLElement>('[data-testid="editor-peer-pointer"][data-peer="Bob"]')!.dataset.hidden).toBe("true");
  });
});
