import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useRef } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Command, Presence, ReplyValue, Track } from "@/generated";
import type { Row } from "@/features/arrangement/layout";
import { arrangementView, useArrangementUi } from "@/features/arrangement/uiStore";
import { resetShell, useShellStore } from "@/app/shell/shellStore";
import { LeftPanel, LeftRail } from "@/app/shell/LeftRail";
import { buildCommands } from "@/app/shell/commands";
import { useProjectStore } from "@/state/projectStore";
import { MockTransport, TransportProvider, type SendOptions } from "@/transport";
import type { MockCollab } from "@/transport/mock/roadmap/collab";
import { PresenceBar } from "..";
import { useClipEditors } from "../presence/editors";
import { PresenceLayer } from "../presence/PresenceLayer";
import { useCollabStore } from "../store";
import { ArrangerSocialLayer, leaveNoteEntries } from ".";
import { useChatUi } from "./chatStore";
import { chatTextError, firstLine, relativeTime } from "./chatText";
import { rulerNoteEntries } from "./notes/notesStore";
import { useNotesUi } from "./notes/notesStore";

class SocialMock extends MockTransport {
  sent: Command[] = [];
  override async send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    this.sent.push(command);
    return super.send(command, opts);
  }
  get sim(): MockCollab {
    return (this as unknown as { collab: MockCollab }).collab;
  }
}

const track = (id: string) => ({ id, parent: null, kind: "Midi", name: id }) as unknown as Track;
const ROWS: Row[] = [
  { track: track("a"), depth: 0, y: 0, laneHeight: 56, height: 56 },
  { track: track("b"), depth: 0, y: 56, laneHeight: 100, height: 100 },
];
const RULER = 30;
const HEADER = 200;

function box(el: HTMLElement, top: number, height: number, width = 1000) {
  el.getBoundingClientRect = () => ({ top, bottom: top + height, left: 0, right: width, width, height, x: 0, y: top }) as DOMRect;
  Object.defineProperty(el, "clientWidth", { configurable: true, value: width });
  Object.defineProperty(el, "clientHeight", { configurable: true, value: height });
}

/** The arranger's geometry, like presence-v2's tests (jsdom has no layout). */
function Arranger() {
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
      />
      <PresenceLayer rootRef={root} scrollRef={scroll} rows={ROWS} masterRow={null} />
      <ArrangerSocialLayer rootRef={root} scrollRef={scroll} rows={ROWS} masterRow={null} />
    </div>
  );
}

async function setup(children: React.ReactNode = null) {
  const mock = new SocialMock({ timers: "manual", seed: 11 });
  render(
    <TransportProvider transport={mock}>
      <PresenceBar />
      <LeftRail />
      <Chat />
      {children}
    </TransportProvider>,
  );
  await waitFor(() => {
    if (!useProjectStore.getState().project) throw new Error("not connected");
  });
  return mock;
}

/** The left pane's content, as the app shell shows it. */
function Chat() {
  const left = useShellStore((s) => s.left);
  return left.open ? <LeftPanel tab={left.tab} /> : null;
}

async function join(mock: SocialMock) {
  await act(() => mock.send({ domain: "Collab", command: { type: "Join", server: "ws://relay:1", session: "jam", token: null, name: "Ada" } }));
  await waitFor(() => expect(useCollabStore.getState().status.type).toBe("Online"));
}

beforeEach(() => {
  resetShell();
  useArrangementUi.getState().setHeaderWidth(HEADER);
  arrangementView.getState().setWidth(800);
  arrangementView.getState().setViewport({ pxPerBeat: 20, scrollBeats: 0 });
});

afterEach(() => {
  useCollabStore.getState().reset();
  useCollabStore.getState().setHideOthers(false);
  useChatUi.setState({ toasts: [], focusRequest: 0, returnFocus: null });
  useNotesUi.setState({ draft: null, open: null, dragging: null });
  localStorage.clear();
});

describe("chat text", () => {
  it("follows the engine's rules and formats", () => {
    expect(chatTextError(" \n")).not.toBeNull();
    expect(chatTextError("x".repeat(2000))).toBeNull();
    expect(chatTextError("x".repeat(2001))).not.toBeNull();
    expect(chatTextError("😀".repeat(1100))).not.toBeNull();
    expect(firstLine("\n  \nhello\nworld")).toBe("hello");
    expect(relativeTime(0, 10_000)).toBe("just now");
    expect(relativeTime(0, 5 * 60_000)).toBe("5 min ago");
  });
});

describe("Chat section", () => {
  it("is a rail tab in a session only, sends with Enter, and toasts peers while closed", async () => {
    const mock = await setup();
    expect(screen.queryByRole("button", { name: "Chat" })).toBeNull();
    expect(buildCommands(mock, []).some((c) => c.id === "chat:focus")).toBe(false);
    await join(mock);
    expect(buildCommands(mock, []).find((c) => c.id === "chat:focus")?.label).toBe("Chat: Focus input");

    // A peer's live message while the chat is closed: a toast; a click opens the chat.
    act(() => void mock.sim.simulateChat("2", "hello there\nsecond line"));
    const toast = await screen.findByRole("status");
    expect(toast.textContent).toContain("Mock peer");
    expect(toast.textContent).toContain("hello there");
    fireEvent.click(toast.querySelector(".eth-toast__body")!);
    await waitFor(() => expect(useShellStore.getState().left).toMatchObject({ open: true, tab: "chat" }));
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.getAllByTestId("chat-message")).toHaveLength(1);

    // Open: no toast for the next one.
    act(() => void mock.sim.simulateChat("2", "again"));
    expect(screen.queryByRole("status")).toBeNull();

    // Enter sends (Shift+Enter doesn't).
    const input = screen.getByTestId("chat-input");
    fireEvent.change(input, { target: { value: "hi all" } });
    fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
    expect(mock.sent.some((c) => c.domain === "Chat")).toBe(false);
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(screen.getAllByTestId("chat-message")).toHaveLength(3));
    const mine = screen.getAllByTestId("chat-message").at(-1)!;
    expect(mine.textContent).toContain("You");
    expect(mine.textContent).toContain("hi all");
    expect((input as HTMLTextAreaElement).value).toBe("");
  });

  it("Mod+Shift+M opens the chat and focuses the input; Escape returns", async () => {
    const mock = await setup(<button type="button">elsewhere</button>);
    await join(mock);
    const elsewhere = screen.getByRole("button", { name: "elsewhere" });
    elsewhere.focus();
    fireEvent.keyDown(window, { key: "M", code: "KeyM", metaKey: true, shiftKey: true });
    await waitFor(() => expect(document.activeElement).toBe(screen.getByTestId("chat-input")));
    fireEvent.keyDown(screen.getByTestId("chat-input"), { key: "Escape" });
    expect(document.activeElement).toBe(elsewhere);
  });

  it("closes with the session", async () => {
    const mock = await setup();
    await join(mock);
    act(() => useShellStore.getState().toggleLeft("chat"));
    await act(() => mock.send({ domain: "Collab", command: { type: "Leave" } }));
    await waitFor(() => expect(useShellStore.getState().left.open).toBe(false));
  });
});

const peer = (state: Partial<Presence["state"]>): Presence => ({
  site: "2",
  actor: null,
  name: "Zoe Q",
  color: 0x5cffe8,
  state: { cursor: null, selected_tracks: ["a"], selected_clips: [], selected_notes: [], selected_devices: [], view: null, ...state },
});

function Editors({ clip }: { clip: string }) {
  const editors = useClipEditors(clip);
  return <span data-testid="editors">{editors.map((e) => e.name).join(",")}</span>;
}

describe("notes, playheads and the hide toggle", () => {
  it("leaves a note at the right-click point, shows it and hides it with 'Hide users and notes'", async () => {
    const mock = await setup(<Arranger />);
    // "Leave a note" on row "b" at beat 5 (100 px into the lanes, 3/4 down).
    const entries = leaveNoteEntries({ kind: "arranger" }, { clientX: HEADER + 100, clientY: RULER + 56 + 75 });
    expect(entries).toMatchObject([{ label: "Leave a note" }]);
    act(() => (entries[0] as { onSelect(): void }).onSelect());
    const input = await screen.findByTestId("note-input");
    fireEvent.change(input, { target: { value: "the bass is late" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(screen.getAllByTestId("pinned-note")).toHaveLength(1));
    const add = mock.sent.find((c) => c.domain === "PinnedNote");
    expect(add).toMatchObject({ command: { type: "Add", text: "the bass is late", position: { beats: 5, track: "b", y: 0.75 } } });
    const dot = screen.getByTestId("pinned-note");
    expect(dot.style.transform).toBe(`translate(${HEADER + 100}px, ${RULER + 56 + 75}px)`);
    // The ruler's menu places it on the ruler row.
    expect(rulerNoteEntries(3)).toHaveLength(1);

    // Resolve from the card.
    fireEvent.click(dot.querySelector("button")!);
    fireEvent.click(await screen.findByRole("button", { name: "Resolve" }));
    await waitFor(() => expect(screen.getByTestId("pinned-note").dataset.resolved).toBe("true"));

    // Hidden (and no "Leave a note") while hiding; our note is still in the project.
    act(() => useCollabStore.getState().setHideOthers(true));
    expect(screen.queryByTestId("pinned-note")).toBeNull();
    expect(leaveNoteEntries({ kind: "arranger" }, { clientX: HEADER + 100, clientY: RULER + 10 })).toEqual([]);
    expect(Object.keys(useProjectStore.getState().project!.pinned_notes)).toHaveLength(1);
    expect(localStorage.getItem("eth-collab-hide-others")).toBe("1");
  });

  it("draws peers' playheads and hides every peer overlay while hiding", async () => {
    const mock = await setup(
      <>
        <Arranger />
        <Editors clip="c1" />
      </>,
    );
    await join(mock);
    act(() => {
      mock.sim.simulatePeer("2", "Zoe Q", 0x5cffe8, peer({ editing_clip: "c1" }).state);
      mock.sim.simulatePeerTransport("2", { position: 8, playing: false, sent_at_ms: 1, loop_region: null });
      mock.sim.simulatePointer("2", { beats: 4, track: "a", y: 0.5 });
    });
    expect(await screen.findByTestId("peer-pointers")).toBeTruthy();
    const line = await screen.findByTestId("peer-playhead");
    await waitFor(() => expect(line.dataset.hidden).toBe("false"));
    expect(line.dataset.beats).toBe("8.000");
    expect(line.style.transform).toBe(`translateX(${HEADER + 160}px)`);
    expect(line.textContent).toBe("ZQ");
    expect(screen.getByTestId("editors").textContent).toBe("Zoe Q");
    expect(screen.getByTestId("collab-highlights")).toBeTruthy();

    // The toggle lives in the collab dialog.
    fireEvent.click(screen.getByTestId("collab-button"));
    fireEvent.click(screen.getByRole("switch", { name: "Hide users and notes" }));
    expect(useCollabStore.getState().hideOthers).toBe(true);
    expect(screen.getByTestId("collab-hide-others").textContent).toContain("Hide users and notes");
    expect(screen.queryByTestId("peer-playhead")).toBeNull();
    expect(screen.queryByTestId("peer-pointers")).toBeNull();
    expect(screen.getByTestId("editors").textContent).toBe("");
    expect(screen.queryByTestId("collab-highlights")).toBeNull();
    // The avatars stay (they're how to turn it back off).
    expect(screen.getByTestId("collab-peers")).toBeTruthy();
  });
});
