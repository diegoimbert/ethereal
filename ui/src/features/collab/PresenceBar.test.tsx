import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Command, ReplyValue } from "@/generated";
import { useProjectStore } from "@/state/projectStore";
import { useSelectionStore } from "@/state/selection";
import { MockTransport, TransportProvider, type SendOptions } from "@/transport";
import { MockCollab } from "@/transport/mock/roadmap/collab";
import { useContextMenuStore } from "@/kit/contextMenuStore";
import { FakePeerConnection } from "./listen/fakeRtc";
import { PresenceBar } from ".";
import { highlightCss, initials, peerColor, useCollabStore } from "./store";

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
    expect((await screen.findByRole("alert")).textContent).toMatch(/relay address/);

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
    // Remembered, without the token.
    expect(localStorage.getItem("eth-collab-join")).toBe(JSON.stringify({ server: "ws://relay:9003", session: "jam", name: "Ada" }));

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
    expect(screen.getByRole("button", { name: "Listen on Mock peer's computer" }).title).toMatch(/cannot host/);
    fireEvent.click(screen.getByRole("button", { name: "Listen on Zoe's computer" }));
    await waitFor(() => expect(mock.sent).toContainEqual({ domain: "Collab", command: { type: "Listen", host: "7" } }));
    expect((await screen.findByTestId("listen-status")).textContent).toContain("Connecting to Zoe…");
    expect(FakePeerConnection.last).not.toBeNull();

    // The chip menu offers to stop.
    const chip = screen.getByTestId("collab-peers").querySelector<HTMLElement>('[data-peer="Zoe"]')!;
    fireEvent.contextMenu(chip);
    const items = useContextMenuStore.getState().menu!.items;
    expect(items.map((i) => (i === "separator" ? i : i.label))).toEqual(["Stop listening"]);
    act(() => (items[0] as { onSelect(): void }).onSelect());
    await waitFor(() => expect(mock.sent).toContainEqual({ domain: "Collab", command: { type: "StopListening" } }));
    await waitFor(() => expect(screen.queryByTestId("listen-status")).toBeNull());
    expect(FakePeerConnection.last!.closed).toBe(true);

    fireEvent.contextMenu(chip);
    const again = useContextMenuStore.getState().menu!.items[0] as { label: string; disabled?: boolean };
    expect(again).toMatchObject({ label: "Listen on Zoe's computer", disabled: false });
    act(() => useContextMenuStore.getState().close());
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
