/** Share button, session pill, Share popover, toasts, banner (docs/SHARING.md §8) against MockShare. */
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Command, ReplyValue } from "@/generated";
import { pickOption } from "@/kit/testing";
import { useProjectStore } from "@/state/projectStore";
import { MockTransport, TransportProvider, type SendOptions } from "@/transport";
import { useCollabStore } from "@/features/collab/store";
import { useListenStore } from "@/features/collab/listen/store";
import { ShareControl } from ".";
import { shareCommands } from "./commands";
import { useConfirm } from "./confirmStore";
import { useJoinStore } from "./join/store";
import { DEFAULT_SHARE_SETTINGS, useShareSettings } from "./settings";
import { useShareStore } from "./store";
import { useShareToasts } from "./toasts";

const EDIT_INVITE = "https://app.ethereal.ws/join/AbCdEfGhIjKlMnOpQrStUv#1AbCdEfGhIjKlMnOpQrStUv";
const LISTEN_INVITE = "https://app.ethereal.ws/join/AbCdEfGhIjKlMnOpQrStUv#1LbCdEfGhIjKlMnOpQrStUv";

/** A MockTransport recording what the UI sent (its `share` is the MockShare simulation). */
class ShareMock extends MockTransport {
  sent: Command[] = [];
  override async send(command: Command, opts?: SendOptions): Promise<ReplyValue> {
    this.sent.push(command);
    return super.send(command, opts);
  }
  shareSent(type: string) {
    return this.sent.filter((c) => c.domain === "Share" && c.command.type === type).map((c) => c.command);
  }
}

async function setup() {
  const mock = new ShareMock({ timers: "manual", seed: 5 });
  render(
    <TransportProvider transport={mock}>
      <ShareControl />
    </TransportProvider>,
  );
  await waitFor(() => {
    if (!useProjectStore.getState().project) throw new Error("not connected");
  });
  await waitFor(() => expect(mock.shareSent("Get")).toHaveLength(1));
  return mock;
}

const popover = () => screen.getByTestId("share-popover");
const toastTitles = () => useShareToasts.getState().toasts.map((t) => t.title);

/** Share from the top bar: Start, then the popover opens on the pill. */
async function share(mock: ShareMock) {
  fireEvent.click(screen.getByTestId("share-button"));
  await waitFor(() => expect(screen.getByTestId("session-pill")).toBeInTheDocument());
  expect(mock.shareSent("Start")).toHaveLength(1);
  await screen.findByTestId("share-popover");
}

let clipboard: string[] = [];

beforeEach(() => {
  clipboard = [];
  vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText: async (t: string) => void clipboard.push(t) } });
});

afterEach(() => {
  vi.unstubAllGlobals();
  useShareStore.getState().reset();
  useShareToasts.getState().clear();
  useConfirm.getState().close();
  useCollabStore.getState().reset();
  useCollabStore.getState().setDialogOpen(false);
  useListenStore.setState({ listening: { type: "Off" } });
  localStorage.clear();
  useShareSettings.getState().reload();
  useProjectStore.getState().reset();
});

describe("ShareControl (host)", () => {
  it("shows Share; sharing opens the popover with the edit and listen links", async () => {
    const mock = await setup();
    expect(screen.getByTestId("share-button")).toHaveTextContent("Share");
    expect(screen.queryByTestId("session-pill")).toBeNull();

    await share(mock);
    const pill = screen.getByTestId("session-pill");
    expect(pill).toHaveTextContent("Live");
    expect(pill.dataset.tone).toBe("live");
    // Alone in the session: no avatars yet.
    expect(within(pill).queryByTestId("share-avatars")).toBeNull();

    const name = useProjectStore.getState().project!.settings.name;
    expect(within(popover()).getByText(`Share “${name}”`)).toBeInTheDocument();
    const edit = within(popover()).getByLabelText<HTMLInputElement>("Edit link");
    expect(edit.value).toMatch(/^https:\/\/app\.ethereal\.ws\/join\/[\w-]{22}#1/);
    fireEvent.click(within(popover()).getByTestId("share-copy"));
    await waitFor(() => expect(clipboard).toEqual([edit.value]));
    expect(toastTitles()).toContain("Link copied");
    expect(within(popover()).getByTestId("share-copy")).toHaveTextContent("Copied");

    fireEvent.click(within(popover()).getByRole("tab", { name: "Can listen" }));
    const listen = within(popover()).getByLabelText<HTMLInputElement>("Listen link");
    expect(listen.value).toMatch(/#1L/);
    expect(within(popover()).getByText(/can listen and chat, but not edit/)).toBeInTheDocument();
    // The host row: you, the Host badge.
    const me = within(popover()).getAllByTestId("share-person")[0]!;
    expect(me).toHaveTextContent("(you)");
    expect(within(me).getByText("Host")).toBeInTheDocument();
  });

  it("people join and leave: toasts, avatars, role and remove", async () => {
    const mock = await setup();
    await share(mock);
    let ada = "";
    act(() => {
      ada = mock.share.simulateJoin("Ada Lovelace", "Edit", 0xff94a6);
    });
    expect(toastTitles()).toContain("Ada Lovelace joined");
    expect(useShareToasts.getState().toasts.at(-1)?.accent).toBe("#ff94a6");
    const avatars = within(screen.getByTestId("session-pill")).getByTestId("share-avatars");
    expect(avatars).toHaveTextContent("AL");

    const row = () => within(popover()).getAllByTestId("share-person").find((r) => r.dataset.member === ada)!;
    expect(row()).toHaveTextContent("Ada Lovelace");
    pickOption(within(row()).getByRole("combobox", { name: "Ada Lovelace's role" }), "Can listen");
    await waitFor(() => expect(mock.shareSent("SetParticipantRole")).toEqual([{ type: "SetParticipantRole", member: ada, role: "Listen" }]));
    await waitFor(() => expect(within(row()).getByRole("combobox")).toHaveTextContent("Can listen"));
    // The popover stayed open through the role list (portaled outside it).
    expect(screen.getByTestId("share-popover")).toBeInTheDocument();

    act(() => mock.share.simulateLeave(ada));
    expect(toastTitles()).toContain("Ada Lovelace left");
    expect(row().className).toMatch(/offline/);
    expect(row()).toHaveTextContent("last seen just now");
    expect(within(screen.getByTestId("session-pill")).queryByTestId("share-avatars")).toBeNull();

    fireEvent.click(within(row()).getByRole("button", { name: "Actions for Ada Lovelace" }));
    fireEvent.pointerDown(screen.getByRole("menuitem", { name: "Remove from project" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Remove from project" }));
    await waitFor(() => expect(mock.shareSent("RemoveParticipant")).toEqual([{ type: "RemoveParticipant", member: ada }]));
    await waitFor(() => expect(within(popover()).getAllByTestId("share-person")).toHaveLength(1));
  });

  it("shows at most three avatars, then +N", async () => {
    const mock = await setup();
    await share(mock);
    act(() => {
      for (const n of ["Ada", "Bob", "Cy", "Dee", "Eve"]) mock.share.simulateJoin(n, "Edit");
    });
    const avatars = within(screen.getByTestId("session-pill")).getByTestId("share-avatars");
    expect(avatars.querySelectorAll(".eth-share-avatar")).toHaveLength(3);
    expect(avatars).toHaveTextContent("+2");
    // The toasts keep the last three.
    expect(toastTitles()).toEqual(["Cy joined", "Dee joined", "Eve joined"]);
  });

  it("resets a link and stops sharing after a confirmation", async () => {
    const mock = await setup();
    await share(mock);
    act(() => void mock.share.simulateJoin("Ada", "Edit"));
    const before = within(popover()).getByLabelText<HTMLInputElement>("Edit link").value;

    fireEvent.click(within(popover()).getByRole("button", { name: "More sharing actions" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Reset edit link" }));
    const confirm = await screen.findByRole("dialog", { name: "Reset the edit link?" });
    expect(confirm).toHaveTextContent("People already in keep access.");
    fireEvent.click(within(confirm).getByRole("button", { name: "Reset link" }));
    await waitFor(() => expect(mock.shareSent("ResetLink")).toEqual([{ type: "ResetLink", role: "Edit" }]));
    await waitFor(() => expect(toastTitles()).toContain("Link reset"));
    await waitFor(() => expect(within(popover()).getByLabelText<HTMLInputElement>("Edit link").value).not.toBe(before));

    fireEvent.click(within(popover()).getByRole("button", { name: "Stop sharing" }));
    const stop = await screen.findByRole("dialog", { name: /^Stop sharing “/ });
    expect(stop).toHaveTextContent("Ada keeps an offline copy. Links stop working.");
    fireEvent.click(within(stop).getByRole("button", { name: "Stop sharing" }));
    await waitFor(() => expect(mock.shareSent("Stop")).toHaveLength(1));
    await waitFor(() => expect(screen.getByTestId("share-button")).toBeInTheDocument());
  });

  it("offers Join with a link… from the popover menu", async () => {
    const mock = await setup();
    await share(mock);
    fireEvent.click(within(popover()).getByRole("button", { name: "More sharing actions" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Join with a link…" }));
    await waitFor(() => expect(useJoinStore.getState().pasteOpen).toBe(true));
    act(() => useJoinStore.getState().setPasteOpen(false));
  });

  it("shows the signaling service states on the pill and in the popover", async () => {
    const mock = await setup();
    await share(mock);
    const s = useShareStore.getState().state;
    if (s.type !== "Hosting") throw new Error("not hosting");
    act(() => useShareStore.setState({ state: { ...s, signal: { type: "Offline", reason: "network" }, edit_link: null } }));
    expect(screen.getByTestId("session-pill")).toHaveTextContent("Not joinable");
    expect(screen.getByTestId("session-pill").title).toMatch(/network/);
    // Offline: no "Getting a link…" spinner, the reason instead.
    expect(within(popover()).queryByText("Getting a link…")).toBeNull();
    expect(within(popover()).getByRole("alert")).toHaveTextContent("Can't reach the sharing service");
    act(() => useShareStore.setState({ state: { ...s, signal: { type: "Connecting" }, edit_link: null } }));
    expect(within(popover()).getByText("Getting a link…")).toBeInTheDocument();
    expect(screen.getByTestId("session-pill")).toHaveTextContent("Connecting…");
    expect(screen.getByTestId("session-pill").dataset.tone).toBe("busy");
  });

  it("edits the identity: SetIdentity with the name and colour", async () => {
    const mock = await setup();
    await share(mock);
    // No identity yet: the editor is open.
    const field = within(popover()).getByLabelText("Your name");
    fireEvent.change(field, { target: { value: "  " } });
    expect(within(popover()).getByRole("alert")).toHaveTextContent("Enter a name.");
    fireEvent.change(field, { target: { value: "Diego" } });
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() => expect(mock.shareSent("SetIdentity")).toEqual([{ type: "SetIdentity", name: "Diego", color: null }]));
    // Collapsed once named; "Change" opens it again.
    expect(within(popover()).queryByLabelText("Your name")).toBeNull();
    fireEvent.click(within(popover()).getByRole("button", { name: "Change" }));
    fireEvent.click(within(popover()).getByRole("button", { name: "Colour 3" }));
    await waitFor(() => expect(mock.shareSent("SetIdentity").at(-1)).toEqual({ type: "SetIdentity", name: "Diego", color: 0xffa529 }));
    expect(within(popover()).getByRole("button", { name: "Colour 3" })).toHaveAttribute("aria-pressed", "true");
    // join-flow's identity key (its "Join as" field reads and writes the same).
    expect(JSON.parse(localStorage.getItem("eth.share.identity")!)).toEqual({ name: "Diego", color: 0xffa529 });
  });

  it("pushes a saved identity, the preferences and servers at startup", async () => {
    localStorage.setItem("eth.share.identity", JSON.stringify({ name: "Ada", color: 0x92a7ff }));
    localStorage.setItem("eth-share-settings", JSON.stringify({ ...DEFAULT_SHARE_SETTINGS, autoListen: false, signalUrl: "https://signal.example.com" }));
    useShareSettings.getState().reload();
    const mock = await setup();
    expect(mock.shareSent("SetIdentity")).toEqual([{ type: "SetIdentity", name: "Ada", color: 0x92a7ff }]);
    expect(mock.shareSent("SetPreferences")).toEqual([{ type: "SetPreferences", resume_on_open: true, auto_listen: false, relay_only: false }]);
    expect(mock.share.preferences).toEqual({ resumeOnOpen: true, autoListen: false, relayOnly: false });
    expect(mock.shareSent("SetServers")).toEqual([{ type: "SetServers", signal_url: "https://signal.example.com", invite_origin: null }]);
    // No ICE servers configured: the engine keeps its own.
    expect(mock.sent.some((c) => c.domain === "Collab" && c.command.type === "SetIceServers")).toBe(false);
  });
});

describe("ShareControl (joiner)", () => {
  async function join(mock: ShareMock, link: string) {
    await act(async () => {
      await mock.send({ domain: "Share", command: { type: "OpenInvite", link } });
      await mock.send({ domain: "Share", command: { type: "AcceptInvite" } });
    });
    await waitFor(() => expect(screen.getByTestId("session-pill")).toHaveTextContent("Live"));
  }

  it("joined: pill with the host, Leave from the popover", async () => {
    const mock = await setup();
    await join(mock, EDIT_INVITE);
    expect(screen.queryByTestId("view-only")).toBeNull();
    expect(within(screen.getByTestId("session-pill")).getByTestId("share-avatars")).toHaveTextContent("MH");
    fireEvent.click(screen.getByTestId("session-pill"));
    expect(within(popover()).getByText(/Shared by Mock host · you can edit/)).toBeInTheDocument();
    expect(within(popover()).queryByRole("combobox")).toBeNull();
    fireEvent.click(within(popover()).getByRole("button", { name: "Leave" }));
    await waitFor(() => expect(mock.shareSent("Leave")).toHaveLength(1));
    await waitFor(() => expect(screen.getByTestId("share-button")).toBeInTheDocument());
    expect(toastTitles().at(-1)).toMatch(/^You left /);
  });

  it("listen link: View only, Listening to the host; host offline banner and toasts", async () => {
    const mock = await setup();
    await join(mock, LISTEN_INVITE);
    expect(screen.getByTestId("view-only")).toHaveTextContent("View only");
    fireEvent.click(screen.getByTestId("session-pill"));
    expect(within(popover()).getByText(/you can listen and chat/)).toBeInTheDocument();
    expect(within(popover()).getByText("Not listening to Mock host")).toBeInTheDocument();
    act(() => useListenStore.setState({ listening: { type: "Listening", host: "2", stream: 1 } as never }));
    expect(within(popover()).getByText("Listening to Mock host")).toBeInTheDocument();
    expect(within(popover()).getByRole("button", { name: "Stop listening" })).toBeInTheDocument();

    act(() => mock.share.simulateHostOnline(false));
    expect(screen.getByTestId("session-pill")).toHaveTextContent("Mock host offline");
    expect(screen.getByTestId("share-offline-banner")).toHaveTextContent("Mock host is offline. You're working on an offline copy.");
    expect(toastTitles()).toContain("Mock host went offline");
    act(() => mock.share.simulateHostOnline(true));
    expect(screen.queryByTestId("share-offline-banner")).toBeNull();
    expect(toastTitles()).toContain("Mock host is back");
  });

});

describe("palette commands", () => {
  it("share, copy, stop; leave when joined", async () => {
    const mock = await setup();
    const ids = () => shareCommands(mock, true).map((c) => c.id);
    expect(ids()).toEqual(["share:start", "settings:sharing"]);
    act(() => shareCommands(mock, true)[0]!.run());
    await waitFor(() => expect(screen.getByTestId("share-popover")).toBeInTheDocument());
    expect(ids()).toEqual(["share:open", "share:copy", "share:stop", "settings:sharing"]);
    const run = (id: string) => act(() => shareCommands(mock, true).find((c) => c.id === id)!.run());
    run("share:copy");
    await waitFor(() => expect(clipboard).toHaveLength(1));
    run("share:stop");
    fireEvent.click(within(await screen.findByRole("dialog", { name: "Stop sharing?" })).getByRole("button", { name: "Stop sharing" }));
    await waitFor(() => expect(mock.shareSent("Stop")).toHaveLength(1));
    await act(async () => {
      await mock.send({ domain: "Share", command: { type: "OpenInvite", link: EDIT_INVITE } });
      await mock.send({ domain: "Share", command: { type: "AcceptInvite" } });
    });
    expect(ids()).toEqual(["share:leave", "settings:sharing"]);
  });
});

describe("ShareControl (relay session)", () => {
  it("a relay session shows its own bar instead of the Share button", async () => {
    const mock = await setup();
    await act(async () => {
      await mock.send({ domain: "Collab", command: { type: "Join", server: "ws://relay:9003", session: "jam", token: null, name: "Ada" } });
    });
    await waitFor(() => expect(screen.getByTestId("collab-button")).toHaveTextContent("● jam"));
    expect(screen.queryByTestId("share-button")).toBeNull();
    expect(screen.queryByTestId("session-pill")).toBeNull();
    await act(async () => {
      await mock.send({ domain: "Collab", command: { type: "Leave" } });
    });
    await waitFor(() => expect(screen.getByTestId("share-button")).toBeInTheDocument());
    expect(screen.queryByTestId("collab-button")).toBeNull();
  });

  it("a shared project's own collab session (the hub's \"share\") shows only the pill", async () => {
    const mock = await setup();
    await share(mock);
    await act(async () => {
      await mock.send({ domain: "Collab", command: { type: "Join", server: "ws://hub", session: "share", token: null, name: "Diego" } });
    });
    await waitFor(() => expect(useCollabStore.getState().status.type).toBe("Online"));
    expect(screen.getByTestId("session-pill")).toBeInTheDocument();
    expect(screen.queryByTestId("collab-button")).toBeNull();
  });
});
