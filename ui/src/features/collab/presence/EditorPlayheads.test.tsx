import { act, screen, waitFor } from "@testing-library/react";
import { useRef } from "react";
import { afterEach, describe, expect, it } from "vitest";
import type { Clip, PeerTransport, Presence } from "@/generated";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
import { useCollabStore } from "../store";
import { editorPlayheadX } from "./EditorPlayheads";
import { EditorPresence, type EditorCursorMapping } from "./EditorPresence";
import { useLocalPresence } from "./local";

// 10 px per content beat.
const MAPPING: EditorCursorMapping = {
  fromScreen: (x, y) => ({ beats: x / 10, pitch: 128 - y / 10 }),
  toScreen: (beats, pitch) => ({ x: beats * 10, y: (128 - pitch) * 10 }),
};

function Harness({ clip }: { clip: string }) {
  const grid = useRef<HTMLDivElement>(null);
  const mapping = useRef(MAPPING);
  return (
    <div ref={grid}>
      <EditorPresence clip={clip} gridRef={grid} mapping={mapping} />
    </div>
  );
}

/** The harness on the mock project's first MIDI clip, once loaded. */
function OnMidiClip() {
  const id = useProjectStore((s) => (s.project ? Object.values(s.project.clips).find((c) => c.content.type === "Midi")?.id : undefined));
  return id ? <Harness clip={id} /> : null;
}

const peer = (site: string, name: string, transport: PeerTransport | null): Presence => ({
  site,
  actor: null,
  name,
  color: 0x5cffe8,
  state: { cursor: null, selected_tracks: [], selected_clips: [], selected_notes: [], selected_devices: [], view: null, transport },
});

const stopped = (position: number): PeerTransport => ({ position, playing: false, sent_at_ms: 1, loop_region: null });

const midiClip = (): Clip => Object.values(useProjectStore.getState().project!.clips).find((c) => c.content.type === "Midi")!;

afterEach(() => {
  useCollabStore.getState().reset();
  useLocalPresence.setState({ editingClip: null });
  resetStores();
});

describe("editorPlayheadX", () => {
  const clip = {
    start: 8,
    length: 8,
    offset: 1,
    looping: { enabled: false, start: 0, end: 4 },
  } as unknown as Clip;

  it("maps song time onto the clip's content axis", () => {
    // Song 10 = 2 beats into the clip = content 3 (offset 1) → 30 px.
    expect(editorPlayheadX(clip, 10, MAPPING)).toBe(30);
  });

  it("has no line where the clip doesn't play", () => {
    expect(editorPlayheadX(clip, 7.99, MAPPING)).toBeNull();
    expect(editorPlayheadX(clip, 16, MAPPING)).toBeNull();
    expect(editorPlayheadX(undefined, 10, MAPPING)).toBeNull();
  });

  it("wraps into the clip loop", () => {
    const looped = { ...clip, offset: 0, looping: { enabled: true, start: 0, end: 4 } } as Clip;
    // Song 13 = 5 beats into the clip → content 1 after one 4-beat loop.
    expect(editorPlayheadX(looped, 13, MAPPING)).toBe(10);
  });
});

describe("EditorPlayheads", () => {
  it("draws a peer's playhead over the clip, dimmed while stopped, and hides it outside the clip", async () => {
    const { unmount } = await renderWithMock(<OnMidiClip />);
    const clip = midiClip();
    const inside = clip.start + Math.min(1, clip.length / 2);
    act(() =>
      useCollabStore.setState({
        status: { type: "Online", session: "jam", site: "1" },
        peers: [peer("2", "Zoe Q", stopped(inside))],
      }),
    );
    const line = await screen.findByTestId("editor-peer-playhead");
    await waitFor(() => expect(line.dataset.hidden).toBe("false"));
    const content = inside - clip.start + clip.offset;
    expect(line.style.transform).toBe(`translateX(${content * 10}px)`);
    expect(line.dataset.playing).toBe("false");
    expect(line.textContent).toBe("ZQ");
    expect(line.style.getPropertyValue("--eth-collab-peer")).toBe("#5cffe8");

    // Before the clip: no line.
    act(() => useCollabStore.setState({ peers: [peer("2", "Zoe Q", stopped(clip.start - 1))] }));
    await waitFor(() => expect(screen.getByTestId("editor-peer-playhead").dataset.hidden).toBe("true"));

    // Hidden with "Hide users and notes"; gone when the peer has no transport.
    act(() => useCollabStore.getState().setHideOthers(true));
    expect(screen.queryByTestId("editor-peer-playheads")).toBeNull();
    act(() => useCollabStore.getState().setHideOthers(false));
    act(() => useCollabStore.setState({ peers: [peer("2", "Zoe Q", null)] }));
    expect(screen.queryByTestId("editor-peer-playheads")).toBeNull();
    unmount();
  });
});
