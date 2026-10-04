// Recents for shared projects (recents-shared, docs/SHARING.md §8.5): badges, avatars,
// "Live", and the sharing menu entries, against the mock with fixture `ProjectSummary.share`s
// (what the engine stores read from `share.json`).
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Command, ProjectShareInfo, ProjectSummary, ShareCommand } from "@/generated";
import { useNotices } from "@/features/notifications";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useContextMenuStore } from "@/kit";
import { useProjectStore } from "@/state";
import type { MockTransport } from "@/transport/mock/MockTransport";
import { ProjectMenu } from "./index";
import { useProjectScreen } from "./screenStore";
import { avatarStack, isLive, shareBadge, useShareSession } from "./shareState";

const person = (name: string, color = 0x5cffe8) => ({ name, color });

const HOST: ProjectShareInfo = {
  role: "Host",
  host_name: "",
  participants: ["Ada", "Tom", "Kim", "Lee", "Max"].map((n) => person(n)),
  active: true,
  last_synced_ms: null,
};
const COPY: ProjectShareInfo = {
  role: "Edit",
  host_name: "Diego",
  participants: [person("Diego", 0xff94a6), person("Ada")],
  active: true,
  last_synced_ms: 1_700_000_000_000,
};
const ENDED: ProjectShareInfo = { ...COPY, role: "Listen", active: false };

/** Give the mock's stored projects these `share`s (by name), as the engine stores would. */
function withShares(mock: MockTransport, shares: Record<string, ProjectShareInfo>) {
  const m = mock as unknown as { summaries(): ProjectSummary[] };
  const orig = m.summaries.bind(mock);
  m.summaries = () => orig().map((p) => (shares[p.name] ? { ...p, share: shares[p.name] } : p));
}

/** Every `Share` command the UI sent. */
function spyShare(mock: MockTransport): ShareCommand[] {
  const sent: ShareCommand[] = [];
  const send = mock.send.bind(mock);
  mock.send = (c: Command, opts) => {
    if (c.domain === "Share") sent.push(c.command);
    return send(c, opts);
  };
  return sent;
}

async function setup(shares: Record<string, ProjectShareInfo>) {
  const { mock } = await renderWithMock(<ProjectMenu />);
  withShares(mock, shares);
  const sent = spyShare(mock);
  fireEvent.click(screen.getByRole("button", { name: "Projects" }));
  const dialog = await screen.findByRole("dialog", { name: "Projects" });
  // The screen lists on open; wait for the fixtures to land.
  await waitFor(() => expect(useProjectStore.getState().projects.some((p) => p.share)).toBe(true));
  return { mock, sent, dialog };
}

function menuLabels(dialog: HTMLElement, name: string): string[] {
  fireEvent.click(within(dialog).getByRole("button", { name: `More actions for ${name}` }));
  const items = useContextMenuStore.getState().menu!.items;
  return items.map((i) => (i === "separator" ? "—" : i.label));
}

function pick(label: string) {
  const item = useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === label);
  if (!item || item === "separator") throw new Error(`no ${label} item`);
  act(() => item.onSelect());
}

const row = (dialog: HTMLElement, name: string) => within(dialog).getByRole("button", { name: new RegExp(`^Open ${name}`) }).closest("li")!;

let clipboard: string[];
beforeEach(() => {
  clipboard = [];
  Object.defineProperty(navigator, "clipboard", {
    configurable: true,
    value: { writeText: vi.fn((t: string) => (clipboard.push(t), Promise.resolve())) },
  });
});

afterEach(() => {
  resetStores();
  useShareSession.getState().set({ type: "Off" });
  useNotices.getState().clear();
  useProjectScreen.setState({ open: false, launchPending: false });
});

describe("share helpers", () => {
  it("labels hosts, copies and ended shares", () => {
    const p = (share?: ProjectShareInfo): ProjectSummary => ({ id: "p", name: "Song", modified_ms: 0, share });
    expect(shareBadge(p())).toBeNull();
    expect(shareBadge(p(HOST))).toMatchObject({ label: "Shared", tone: "accent" });
    expect(shareBadge(p(COPY))).toMatchObject({ label: "From Diego" });
    expect(shareBadge(p(COPY))!.title).toContain("You can edit");
    expect(shareBadge(p({ ...COPY, role: "Listen" }))!.title).toContain("You can listen");
    expect(shareBadge(p(ENDED))).toMatchObject({ label: "Sharing ended", tone: "default" });
    expect(shareBadge(p(ENDED))!.title).toBe("Diego stopped sharing. This copy stays on this computer.");
  });

  it("stacks at most 3 avatars, then +N", () => {
    expect(avatarStack(HOST.participants)).toMatchObject({ more: 2 });
    expect(avatarStack(HOST.participants).shown.map((x) => x.name)).toEqual(["Ada", "Tom", "Kim"]);
    expect(avatarStack(COPY.participants)).toMatchObject({ more: 0 });
  });

  it("is live only for the open, connected project", () => {
    const hosting = (signal: "Online" | "Connecting") => ({
      type: "Hosting" as const,
      project: "a",
      edit_link: null,
      listen_link: null,
      participants: [],
      signal: { type: signal },
    });
    expect(isLive(hosting("Online"), "a")).toBe(true);
    expect(isLive(hosting("Online"), "b")).toBe(false);
    expect(isLive(hosting("Connecting"), "a")).toBe(false);
    const joined = { type: "Joined" as const, project: "a", role: "Edit" as const, participants: [] };
    expect(isLive({ ...joined, link: { type: "Online" } }, "a")).toBe(true);
    expect(isLive({ ...joined, link: { type: "HostOffline", since_ms: 0 } }, "a")).toBe(false);
    expect(isLive({ type: "Off" }, "a")).toBe(false);
  });
});

describe("Recents: shared projects", () => {
  it("shows badges and avatars per row, and nothing on private projects", async () => {
    const { dialog } = await setup({ "Beat sketch": HOST, "Ambient idea": COPY });
    const host = row(dialog, "Beat sketch");
    expect(within(host).getByText("Shared")).toBeInTheDocument();
    expect(within(host).getByRole("img", { name: "People: Ada, Tom, Kim, Lee, Max" })).toBeInTheDocument();
    expect(within(host).getByText("T")).toBeInTheDocument(); // Tom's initial
    expect(within(host).getByText("+2")).toBeInTheDocument();
    expect(within(host).queryByText("Live")).toBeNull();

    const copy = row(dialog, "Ambient idea");
    expect(within(copy).getByText("From Diego")).toBeInTheDocument();
    expect(within(copy).getByRole("img", { name: "People: Diego, Ada" })).toBeInTheDocument();

    // The open project ("Demo") is private: no badge.
    const current = within(dialog).getByRole("region", { name: "Open project" });
    expect(within(current).queryByText("Shared")).toBeNull();
    expect(within(current).queryByRole("group", { name: "Sharing" })).toBeNull();
  });

  it("marks the open shared project Live while its room is online", async () => {
    const { mock, dialog } = await setup({ Demo: HOST });
    const current = within(dialog).getByRole("region", { name: "Open project" });
    expect(within(current).getByText("Shared")).toBeInTheDocument();
    expect(within(current).queryByText("Live")).toBeNull();
    await act(() => mock.send({ domain: "Share", command: { type: "Start" } }));
    expect(await within(current).findByText("Live")).toBeInTheDocument();
    await act(() => mock.send({ domain: "Share", command: { type: "Stop" } }));
    await waitFor(() => expect(within(current).queryByText("Live")).toBeNull());
  });

  it("shows Sharing ended, with only Make a private copy", async () => {
    const { dialog } = await setup({ "Ambient idea": ENDED });
    expect(within(row(dialog, "Ambient idea")).getByText("Sharing ended")).toBeInTheDocument();
    const labels = menuLabels(dialog, "Ambient idea");
    expect(labels).toContain("Make a private copy…");
    expect(labels).not.toContain("Reconnect");
  });

  it("offers the right menu entries per role", async () => {
    const { dialog } = await setup({ "Beat sketch": HOST, "Ambient idea": COPY });
    expect(menuLabels(dialog, "Beat sketch")).toEqual(["Open", "Open without plugins", "Rename", "Duplicate", "Export…", "—", "Copy invite link", "Stop sharing…", "—", "Delete…"]);
    expect(menuLabels(dialog, "Ambient idea")).toEqual(["Open", "Open without plugins", "Rename", "Duplicate", "Export…", "—", "Reconnect", "Make a private copy…", "—", "Delete…"]);
  });

  it("copies the invite link of a hosted project, opening and resuming it first", async () => {
    const { dialog, sent } = await setup({ "Beat sketch": HOST });
    menuLabels(dialog, "Beat sketch");
    pick("Copy invite link");
    await waitFor(() => expect(useNotices.getState().notices.map((n) => n.message)).toContain("Link copied"));
    expect(useProjectStore.getState().project!.settings.name).toBe("Beat sketch");
    expect(sent.map((c) => c.type)).toContain("Start");
    expect(clipboard).toHaveLength(1);
    expect(clipboard[0]).toMatch(/^https:\/\/etherealws\.pages\.dev\/join\/.+#1/);
  });

  it("copies the link of the open hosted project without restarting it", async () => {
    const { mock, dialog, sent } = await setup({ Demo: HOST });
    await act(() => mock.send({ domain: "Share", command: { type: "Start" } }));
    const current = within(dialog).getByRole("region", { name: "Open project" });
    fireEvent.click(within(current).getByRole("button", { name: "Copy invite link" }));
    await waitFor(() => expect(clipboard).toHaveLength(1));
    const s = useShareSession.getState().state;
    expect(s.type === "Hosting" && clipboard[0] === s.edit_link).toBe(true);
    expect(sent.filter((c) => c.type === "Start")).toHaveLength(1);
  });

  it("stops sharing after confirming", async () => {
    const { mock, dialog, sent } = await setup({ Demo: HOST });
    await act(() => mock.send({ domain: "Share", command: { type: "Start" } }));
    const current = within(dialog).getByRole("region", { name: "Open project" });
    fireEvent.click(within(current).getByRole("button", { name: "Stop sharing…" }));
    expect(within(current).getByText(/Ada, Tom, Kim, Lee and Max keep an offline copy\. Links stop working\./)).toBeInTheDocument();
    fireEvent.click(within(current).getByRole("button", { name: "Cancel" }));
    expect(sent.map((c) => c.type)).not.toContain("Stop");

    fireEvent.click(within(current).getByRole("button", { name: "Stop sharing…" }));
    fireEvent.click(within(current).getByRole("button", { name: "Confirm stop sharing Demo" }));
    await waitFor(() => expect(sent.map((c) => c.type)).toContain("Stop"));
    await waitFor(() => expect(useShareSession.getState().state.type).toBe("Off"));
  });

  it("reconnects a copy: opens it and joins the host", async () => {
    const { dialog, sent } = await setup({ "Ambient idea": COPY });
    menuLabels(dialog, "Ambient idea");
    pick("Reconnect");
    await waitFor(() => expect(useShareSession.getState().state.type).toBe("Joined"));
    expect(useProjectStore.getState().project!.settings.name).toBe("Ambient idea");
    expect(sent).toContainEqual({ type: "Reconnect", project: useProjectStore.getState().project!.id });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("makes a copy private after confirming (Detach)", async () => {
    const { dialog, sent } = await setup({ "Ambient idea": COPY });
    const id = useProjectStore.getState().projects.find((p) => p.name === "Ambient idea")!.id;
    menuLabels(dialog, "Ambient idea");
    pick("Make a private copy…");
    expect(within(dialog).getByText("Make “Ambient idea” private? It stops syncing with Diego.")).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "Confirm make private Ambient idea" }));
    await waitFor(() => expect(sent).toContainEqual({ type: "Detach", project: id }));
    // Not opened.
    expect(useProjectStore.getState().project!.settings.name).toBe("Demo");
  });

  it("reports a failed action", async () => {
    const { mock, dialog } = await setup({ "Ambient idea": COPY });
    const send = mock.send.bind(mock);
    mock.send = (c, opts) =>
      c.domain === "Share" && c.command.type === "Detach" ? Promise.reject(new Error("Detach failed: disk full")) : send(c, opts);
    menuLabels(dialog, "Ambient idea");
    pick("Make a private copy…");
    fireEvent.click(within(dialog).getByRole("button", { name: "Confirm make private Ambient idea" }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("disk full");
  });
});
