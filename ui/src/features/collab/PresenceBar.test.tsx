import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Command, PresenceState, ReplyValue } from "@/generated";
import { useProjectStore } from "@/state/projectStore";
import { useSelectionStore } from "@/state/selection";
import { MockTransport, TransportProvider, type SendOptions } from "@/transport";
import { MockCollab } from "@/transport/mock/roadmap/collab";
import { useContextMenuStore } from "@/kit/contextMenuStore";
import { FakePeerConnection } from "./listen/fakeRtc";
import { PresenceBar } from ".";
import { highlightCss, initials, peerColor, useCollabStore } from "./store";
import { setActivity, setFollowing, useLocalPresence } from "./presence/local";
import { usePointerStore } from "./presence/pointers";

/** A MockTransport recording what the UI sent (its `MockCollab` simulates the session). */
class CollabMock extends MockTransport {
  sent: Command[] = [];
  override async send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    this.sent.push(command);
    return super.send(command, opts);
  }
  /** The mock's session simulation (private in MockTransport). */
  get sim(): MockCollab {
    return (this as unknown as { collab: MockCollab }).collab;
  }
}

async function setup() {
  const mock = new CollabMock({ timers: "manual", seed: 3 });
  render(
    <TransportProvider transport={mock}>
      <PresenceBar />
    </TransportProvider>,
  );
  await waitFor(() => {
    if (!useProjectStore.getState().project) throw new Error("not connected");
  });
  return mock;
}

afterEach(() => {
  useCollabStore.getState().reset();
  setFollowing(null);
  setActivity(null);
  useSelectionStore.getState().selectTrack(null);
  localStorage.clear();
});

describe("PresenceBar", () => {
  it("asks for the current state on mount", async () => {
    const mock = await setup();
    expect(mock.sent).toContainEqual({ domain: "Collab", command: { type: "Get" } });
    expect(screen.getByTestId("collab-button").textContent).toBe("Collab");
  });

  it("joins, shows peers in their colors, publishes the selection and leaves", async () => {
    const mock = await setup();
    fireEvent.click(screen.getByTestId("collab-button"));
    fireEvent.change(screen.getByLabelText("Relay address"), { target: { value: "http://nope" } });
    fireEvent.click(screen.getByRole("button", { name: "Join" }));
    // Inline errors under the fields (base-114): nothing is sent.
    const alerts = (await screen.findAllByRole("alert")).map((a) => a.textContent);
    expect(alerts).toEqual([expect.stringMatching(/relay address/), "Enter a session name."]);
    expect(mock.sent.some((c) => c.domain === "Collab" && c.command.type === "Join")).toBe(false);

    fireEvent.change(screen.getByLabelText("Relay address"), { target: { value: "ws://relay:9003" } });
    fireEvent.change(screen.getByLabelText("Session"), { target: { value: "jam" } });
    fireEvent.change(screen.getByLabelText("Your name"), { target: { value: "Ada" } });
    fireEvent.change(screen.getByLabelText("Token"), { target: { value: "secret" } });
    fireEvent.click(screen.getByRole("button", { name: "Join" }));
    await waitFor(() => expect(screen.getByTestId("collab-button").textContent).toBe("● jam"));
    expect(mock.sent).toContainEqual({
      domain: "Collab",
      command: { type: "Join", server: "ws://relay:9003", session: "jam", token: "secret", name: "Ada" },
    });
    // Remembered; the token apart (base-114: this browser's storage on the web build).
    expect(localStorage.getItem("eth-collab-join")).toBe(JSON.stringify({ server: "ws://relay:9003", session: "jam", name: "Ada" }));
    expect(localStorage.getItem("eth-collab-token")).toBe("secret");

    const avatar = screen.getByTestId("collab-peers").querySelector<HTMLElement>(".eth-collab__avatar")!;
    expect(avatar.textContent).toBe("MP");
    expect(avatar.style.getPropertyValue("--eth-collab-peer")).toBe("#5cffe8");

    // The local selection goes out as presence.
    const track = Object.values(useProjectStore.getState().project!.tracks)[0]!.id;
    act(() => useSelectionStore.getState().selectTrack(track));
    await waitFor(() => expect(mock.sim.presence.selected_tracks).toEqual([track]));

    // A peer's selection is outlined.
    act(() =>
      mock.sim.simulatePeer("7", "Zoe", 0xff94a6, {
        cursor: null,
        selected_tracks: [track],
        selected_clips: [],
        selected_notes: [],
        selected_devices: [],
        view: null,
      }),
    );
    expect(screen.getByTestId("collab-highlights").textContent).toContain(`[data-track="${track}"]`);

    fireEvent.click(screen.getByTestId("collab-button"));
    expect(screen.getByTestId("collab-session").textContent).toContain("Zoe");
    fireEvent.click(screen.getByRole("button", { name: "Leave session" }));
    await waitFor(() => expect(screen.getByTestId("collab-button").textContent).toBe("Collab"));
    expect(screen.queryByTestId("collab-peers")).toBeNull();
  });
});

describe("Listen on a peer (stream-listen)", () => {
  const peerState = { cursor: null, selected_tracks: [], selected_clips: [], selected_notes: [], selected_devices: [], view: null };
  afterEach(() => {
    // @ts-expect-error: jsdom has no WebRTC; remove the stub
    delete globalThis.RTCPeerConnection;
  });

  it("is disabled with a reason, listens from the chip menu or the dialog, shows the status and stops", async () => {
    const mock = await setup();
    await act(() => mock.send({ domain: "Collab", command: { type: "Join", server: "ws://r:1", session: "jam", token: null, name: "Ada" } }));
    act(() => mock.sim.simulatePeer("7", "Zoe", 0xff94a6, { ...peerState, can_host: true }));

    fireEvent.click(screen.getByTestId("collab-button"));
    const zoe = screen.getByRole("button", { name: "Listen on Zoe's computer" });
    expect(zoe).toHaveProperty("disabled", true);
    expect(zoe.title).toMatch(/no WebRTC/);

    globalThis.RTCPeerConnection = FakePeerConnection as unknown as typeof RTCPeerConnection;
    act(() => mock.sim.simulatePeer("7", "Zoe", 0xff94a6, { ...peerState, can_host: true, view: "x" }));
    expect(screen.getByRole("button", { name: "Listen on Mock peer's computer" }).title).toMatch(/can't host/);
    fireEvent.click(screen.getByRole("button", { name: "Listen on Zoe's computer" }));
    await waitFor(() => expect(mock.sent).toContainEqual({ domain: "Collab", command: { type: "Listen", host: "7" } }));
    expect((await screen.findByTestId("listen-status")).getAttribute("aria-label")).toBe("Connecting to Zoe…");
    expect(FakePeerConnection.last).not.toBeNull();

    // The chip menu offers to stop.
    const chip = screen.getByTestId("collab-peers").querySelector<HTMLElement>('[data-peer="Zoe"]')!;
    fireEvent.contextMenu(chip);
    const labels = () => useContextMenuStore.getState().menu!.items.map((i) => (i === "separator" ? i : i.label));
    const item = (label: string) => useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === label) as { onSelect(): void; disabled?: boolean };
    // Next to presence-v2's follow entry.
    expect(labels()).toEqual(["Follow Zoe", "separator", "Stop listening"]);
    act(() => item("Stop listening").onSelect());
    await waitFor(() => expect(mock.sent).toContainEqual({ domain: "Collab", command: { type: "StopListening" } }));
    await waitFor(() => expect(screen.queryByTestId("listen-status")).toBeNull());
    expect(FakePeerConnection.last!.closed).toBe(true);

    fireEvent.contextMenu(chip);
    expect(item("Listen on Zoe's computer").disabled).toBe(false);
    act(() => useContextMenuStore.getState().close());
  });
});

describe("PresenceBar presence v2", () => {
  const state = (s: Partial<PresenceState> = {}): PresenceState => ({
    cursor: null,
    selected_tracks: [],
    selected_clips: [],
    selected_notes: [],
    selected_devices: [],
    view: null,
    ...s,
  });

  async function joined() {
    const mock = await setup();
    await act(() => mock.send({ domain: "Collab", command: { type: "Join", server: "ws://r:1", session: "jam", token: null, name: "Me" } }));
    await waitFor(() => expect(screen.getByTestId("collab-button").textContent).toBe("● jam"));
    return mock;
  }

  it("follows a peer on chip click (published as `following`) and stops on a second click", async () => {
    const mock = await joined();
    const chip = screen.getByRole("button", { name: "Follow Mock peer" });
    fireEvent.click(chip);
    expect(useLocalPresence.getState().following).toBe("2");
    await waitFor(() => expect(mock.sim.presence.following).toBe("2"));
    expect(chip.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: "Stop following Mock peer" }));
    expect(useLocalPresence.getState().following).toBeNull();
    await waitFor(() => expect(mock.sim.presence.following).toBeUndefined());
  });

  it("publishes the activity and shows peers' activity, listening and following", async () => {
    const mock = await joined();
    act(() => setActivity({ kind: "Dragging", target: { type: "Clip", clip: "c1" } }));
    await waitFor(() => expect(mock.sim.presence.activity).toEqual({ kind: "Dragging", target: { type: "Clip", clip: "c1" } }));
    act(() => setActivity(null));
    await waitFor(() => expect(mock.sim.presence.activity).toBeUndefined());

    act(() => mock.sim.simulatePeer("7", "Zoe", 0xff94a6, state({ listening_to: "1", following: "1", activity: { kind: "Resizing", target: { type: "Selection" } } })));
    const zoe = screen.getByRole("button", { name: "Follow Zoe" });
    expect(zoe.title).toContain("Zoe · resizing · listening to you · following you");
    expect(zoe.dataset.followed).toBe("you");
    expect(within(zoe).getByTestId("listening-badge").title).toBe("Zoe is listening to you");
    expect(screen.getAllByTestId("listening-badge")).toHaveLength(1);
  });

  it("keeps peers' pointers in their store and drops them when the session ends", async () => {
    const mock = await joined();
    act(() => mock.sim.simulatePointer("2", { beats: 3, track: null, y: 0 }));
    expect(usePointerStore.getState().sites).toEqual(["2"]);
    expect(usePointerStore.getState().trails.get("2")?.latest()).toEqual({ beats: 3, track: null, y: 0 });
    act(() => mock.sim.simulatePointer("2", null));
    expect(usePointerStore.getState().sites).toEqual([]);
    act(() => mock.sim.simulatePointer("2", { beats: 3, track: null, y: 0 }));
    await act(() => mock.send({ domain: "Collab", command: { type: "Leave" } }));
    expect(usePointerStore.getState().sites).toEqual([]);
  });
});

describe("collab helpers", () => {
  it("formats colors, initials and highlight rules", () => {
    expect(peerColor(0x00ff00)).toBe("#00ff00");
    expect(initials("ada lovelace")).toBe("AL");
    expect(initials("  ")).toBe("?");
    const css = highlightCss([
      {
        site: "2",
        actor: null,
        name: "B",
        color: 0x112233,
        state: { cursor: null, selected_tracks: ["T1", 'x"]{}'], selected_clips: ["C1"], selected_notes: [], selected_devices: [], view: null },
      },
    ]);
    expect(css).toContain('[data-clip-id="C1"]');
    expect(css).toContain("#112233");
    expect(css).not.toContain("x\"");
  });
});
