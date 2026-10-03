import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Command, ReplyValue } from "@/generated";
import { useProjectStore } from "@/state/projectStore";
import { MockTransport, TransportProvider, type SendOptions } from "@/transport";
import type { Unsubscribe } from "@/transport/EngineTransport";
import { IDENTITY_KEY, saveIdentity } from "./identity";
import { JoinRoot } from "./JoinRoot";
import { openInvite, openJoinWithLink, useJoinStore } from "./store";
import { joinPaletteCommands } from ".";

const ROOM = "AbCdEfGhIjKlMnOpQrStUv";
const KEY = "10123456789_-abcdefghij";
const LINK = `https://etherealws.pages.dev/join/${ROOM}#${KEY}`;

/** A MockTransport (with `MockShare`) recording Share commands; optionally a deep-link source. */
class ShareMock extends MockTransport {
  sent: Command[] = [];
  private deepLink: ((url: string) => void) | null = null;
  override async send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    if (command.domain === "Share") this.sent.push(command);
    return super.send(command, opts);
  }
  /** As `TauriTransport.onDeepLink`. */
  onDeepLink(listener: (url: string) => void): Unsubscribe {
    this.deepLink = listener;
    return () => {
      this.deepLink = null;
    };
  }
  clickDeepLink(url: string) {
    this.deepLink?.(url);
  }
  shareTypes() {
    return this.sent.map((c) => (c.domain === "Share" ? c.command.type : ""));
  }
}

async function setup() {
  const mock = new ShareMock({ timers: "manual", seed: 1 });
  render(
    <TransportProvider transport={mock}>
      <JoinRoot />
    </TransportProvider>,
  );
  await waitFor(() => {
    if (!useProjectStore.getState().project) throw new Error("not connected");
  });
  return mock;
}

afterEach(() => {
  localStorage.clear();
  useJoinStore.setState({ queue: [], pasteOpen: false });
});

describe("JoinRoot", () => {
  it("opens a queued invite once connected, joins, and strips the link", async () => {
    saveIdentity({ name: "Ada", color: null });
    const delivered = vi.fn();
    openInvite(LINK, delivered);
    const mock = await setup();
    await screen.findByText("Mock host");
    expect(screen.getByTestId("join-ready")).toHaveTextContent("Mock host invites you to Shared song");
    expect(delivered).toHaveBeenCalledTimes(1);
    expect(mock.sent).toContainEqual({ domain: "Share", command: { type: "OpenInvite", link: LINK } });

    fireEvent.click(screen.getByRole("button", { name: "Join" }));
    await screen.findByText("You're in Shared song with Mock host");
    expect(mock.shareTypes()).toEqual(expect.arrayContaining(["SetIdentity", "AcceptInvite"]));
    expect(mock.sent).toContainEqual({ domain: "Share", command: { type: "SetIdentity", name: "Ada", color: null } });
    expect(mock.share.state.type).toBe("Joined");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("asks for a name the first time and remembers it", async () => {
    const mock = await setup();
    act(() => openInvite(LINK));
    fireEvent.change(await screen.findByLabelText("Your name"), { target: { value: "Ada" } });
    fireEvent.click(screen.getByRole("button", { name: "Join" }));
    await waitFor(() => expect(mock.share.state.type).toBe("Joined"));
    expect(mock.share.name).toBe("Ada");
    expect(JSON.parse(localStorage.getItem(IDENTITY_KEY)!)).toEqual({ name: "Ada", color: null });
  });

  it("Not now declines the invite", async () => {
    const mock = await setup();
    act(() => openInvite(LINK));
    fireEvent.click(await screen.findByRole("button", { name: "Not now" }));
    await waitFor(() => expect(mock.share.state.type).toBe("Off"));
    expect(mock.shareTypes()).toContain("Leave");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("shows why an invalid link can't be joined", async () => {
    const mock = await setup();
    act(() => openInvite(`https://etherealws.pages.dev/join/${ROOM}#1short`));
    expect(await screen.findByRole("alert")).toHaveTextContent("The invite link is damaged.");
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(mock.share.state.type).toBe("Off"));
  });

  it("waits for a host whose app is closed", async () => {
    await setup();
    act(() => openInvite(`https://etherealws.pages.dev/join/Off${ROOM.slice(3)}#${KEY}`));
    expect(await screen.findByText(/Ethereal is closed/)).toBeInTheDocument();
  });

  it("routes desktop deep links (and ignores other ethereal:// links)", async () => {
    saveIdentity({ name: "Ada", color: null });
    const mock = await setup();
    act(() => mock.clickDeepLink("ethereal://open/something"));
    act(() => mock.clickDeepLink(`ethereal://join/${ROOM}#${KEY}`));
    await screen.findByTestId("join-ready");
    expect(mock.sent).toContainEqual({ domain: "Share", command: { type: "OpenInvite", link: `ethereal://join/${ROOM}#${KEY}` } });
    expect(mock.sent.filter((c) => c.domain === "Share" && c.command.type === "OpenInvite")).toHaveLength(1);
  });

  it("explains that the host must stop sharing before joining", async () => {
    const mock = await setup();
    await act(async () => {
      await mock.send({ domain: "Share", command: { type: "Start" } });
    });
    act(() => openInvite(LINK));
    expect(await screen.findByRole("alert")).toHaveTextContent("Stop sharing your project before joining another one.");
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(mock.share.state.type).toBe("Hosting");
  });

  it("Join with a link…: validates, then opens the invite (palette entry included)", async () => {
    saveIdentity({ name: "Ada", color: null });
    const mock = await setup();
    const [entry] = joinPaletteCommands();
    expect(entry).toMatchObject({ id: "share:join-link", label: "Join shared project…" });
    act(() => entry!.run());
    const input = await screen.findByLabelText("Invite link");
    fireEvent.change(input, { target: { value: "https://example.org/song" } });
    fireEvent.click(screen.getByRole("button", { name: "Join" }));
    expect(screen.getByRole("alert")).toHaveTextContent("This is not an Ethereal invite link.");
    fireEvent.change(input, { target: { value: ` ethereal://join/${ROOM}#${KEY} ` } });
    fireEvent.click(screen.getByRole("button", { name: "Join" }));
    await screen.findByTestId("join-ready");
    expect(mock.sent).toContainEqual({ domain: "Share", command: { type: "OpenInvite", link: `ethereal://join/${ROOM}#${KEY}` } });
    expect(useJoinStore.getState().pasteOpen).toBe(false);
  });

  it("opens the paste dialog from anywhere", async () => {
    await setup();
    act(() => openJoinWithLink());
    expect(await screen.findByRole("dialog", { name: "Join with a link" })).toBeInTheDocument();
  });
});
