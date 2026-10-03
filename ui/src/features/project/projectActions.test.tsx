// base-114: the open project's actions (save as, duplicate, export/import, close and delete,
// rename), the leave-session warning and the recents badges, against the mock engine.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useCollabStore } from "@/features/collab/store";
import { useNotices } from "@/features/notifications";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
import type { Command } from "@/generated";
import { cmd, type EngineTransport } from "@/transport";
import { exportProject, importProject } from "./actions";
import { ProjectMenu } from "./index";
import { guardLeave, useLeaveGuard } from "./leaveGuard";
import { useProjectScreen } from "./screenStore";
import { markSessionProject, resetSessionMarks, splitLocalCopy } from "./sessionMarks";

const store = () => useProjectStore.getState();
const names = () => store().projects.map((p) => p.name);
const current = () => store().project!;

/** Blobs handed to the browser as downloads. */
let downloads: Blob[] = [];

beforeEach(() => {
  downloads = [];
  URL.createObjectURL = vi.fn((b: Blob | MediaSource) => {
    downloads.push(b as Blob);
    return "blob:test";
  });
  URL.revokeObjectURL = vi.fn();
});

afterEach(() => {
  resetStores();
  useProjectScreen.setState({ open: false, launchPending: false, mode: "home" });
  useCollabStore.getState().reset();
  useLeaveGuard.setState({ pending: null });
  useNotices.getState().clear();
  localStorage.clear();
  resetSessionMarks();
});

/** Record the commands sent through `t` (the app holds the same object). */
function recordSent(t: EngineTransport): Command[] {
  const sent: Command[] = [];
  const send = t.send.bind(t);
  t.send = (c, o) => {
    sent.push(c);
    return send(c, o);
  };
  return sent;
}

async function openScreen() {
  fireEvent.click(screen.getByRole("button", { name: "Projects" }));
  return screen.findByRole("dialog", { name: "Projects" });
}

const actions = (dialog: HTMLElement) => within(dialog).getByRole("group", { name: "Project actions" });

describe("open project actions", () => {
  it("saves as a new project and switches to it", async () => {
    await renderWithMock(<ProjectMenu />);
    const before = current().id;
    const dialog = await openScreen();
    fireEvent.click(within(actions(dialog)).getByRole("button", { name: "Save as…" }));
    const input = await screen.findByLabelText("Save as name");
    expect(input).toHaveAttribute("placeholder", "Demo copy");
    fireEvent.change(input, { target: { value: "Demo v2" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(current().settings.name).toBe("Demo v2"));
    expect(current().id).not.toBe(before);
    expect(names()).toEqual(expect.arrayContaining(["Demo", "Demo v2"]));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("duplicates the open project without leaving it, and says so", async () => {
    await renderWithMock(<ProjectMenu />);
    const id = current().id;
    const dialog = await openScreen();
    fireEvent.click(within(actions(dialog)).getByRole("button", { name: "Duplicate" }));
    await waitFor(() => expect(names()).toContain("Demo copy"));
    expect(current().id).toBe(id);
    expect(useNotices.getState().notices.map((n) => n.message)).toEqual(["Duplicated as “Demo copy”"]);
  });

  it("exports a bundle download and imports it back as a new project (web)", async () => {
    const { mock } = await renderWithMock(<ProjectMenu />);
    const transport = mock;
    const sent = recordSent(mock);
    expect(await act(() => exportProject(transport, current().id, "Demo"))).toBe(true);
    expect(downloads).toHaveLength(1);
    const bytes = new Uint8Array(await downloads[0]!.arrayBuffer());
    // The download is released engine-side once saved.
    await waitFor(() => expect(sent.some((c) => c.domain === "Export" && c.command.type === "Release")).toBe(true));

    const file = { name: "Demo.ether", size: bytes.length, slice: (a: number, b: number) => new Blob([bytes.slice(a, b)]) };
    const summary = await act(() => importProject(transport, file));
    // "Demo" exists: the import gets a number.
    expect(summary?.name).toBe("Demo 2");
    await waitFor(() => expect(names()).toContain("Demo 2"));
    expect(current().settings.name).toBe("Demo");
  });

  it("deletes the open project by closing it first", async () => {
    await renderWithMock(<ProjectMenu />);
    const id = current().id;
    const dialog = await openScreen();
    fireEvent.click(within(actions(dialog)).getByRole("button", { name: "Delete…" }));
    const confirm = within(dialog).getByRole("group", { name: "Confirm delete" });
    expect(confirm.textContent).toMatch(/Delete “Demo”\? It closes first/);
    fireEvent.click(within(confirm).getByRole("button", { name: "Close and delete" }));
    await waitFor(() => expect(store().projects.some((p) => p.id === id)).toBe(false));
    expect(current().id).not.toBe(id);
  });

  it("Rename focuses the name field", async () => {
    await renderWithMock(<ProjectMenu />);
    act(() => useProjectScreen.getState().rename());
    const input = await screen.findByLabelText("Project name");
    await waitFor(() => expect(input).toHaveFocus());
  });
});

describe("leave-session warning", () => {
  const goOnline = () => act(() => useCollabStore.setState({ status: { type: "Online", session: "jam", site: "1" } }));

  it("asks before opening another project while in a session", async () => {
    const { mock } = await renderWithMock(<ProjectMenu />);
    const sent = recordSent(mock);
    goOnline();
    let dialog = await openScreen();
    fireEvent.click(await within(dialog).findByRole("button", { name: "Open Beat sketch" }));
    const ask = await screen.findByRole("dialog", { name: "Leave the “jam” session?" });
    fireEvent.click(within(ask).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: /Leave the/ })).toBeNull());
    expect(current().settings.name).toBe("Demo");
    expect(sent.some((c) => c.domain === "Collab" && c.command.type === "Leave")).toBe(false);

    dialog = screen.getByRole("dialog", { name: "Projects" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Open Beat sketch" }));
    fireEvent.click(within(await screen.findByRole("dialog", { name: /Leave the/ })).getByRole("button", { name: "Leave & open" }));
    await waitFor(() => expect(current().settings.name).toBe("Beat sketch"));
    expect(sent).toContainEqual({ domain: "Collab", command: { type: "Leave" } });
  });

  it("asks before creating a project, with its own confirm label", async () => {
    await renderWithMock(<ProjectMenu />);
    goOnline();
    const dialog = await openScreen();
    fireEvent.click(within(dialog).getByRole("button", { name: "New project" }));
    fireEvent.change(await screen.findByLabelText("New project name"), { target: { value: "Fresh" } });
    fireEvent.click(screen.getByRole("button", { name: "Create" }));
    const ask = await screen.findByRole("dialog", { name: "Leave the “jam” session?" });
    fireEvent.click(within(ask).getByRole("button", { name: "Leave & create" }));
    await waitFor(() => expect(current().settings.name).toBe("Fresh"));
  });

  it("runs at once when not in a session", async () => {
    const run = vi.fn();
    await expect(guardLeave(run)).resolves.toBe(true);
    expect(run).toHaveBeenCalledOnce();
    expect(useLeaveGuard.getState().pending).toBeNull();
  });
});

describe("recents badges", () => {
  it("marks local copies and projects used in a session", async () => {
    const { mock } = await renderWithMock(<ProjectMenu />);
    const beat = store().projects.find((p) => p.name === "Beat sketch")!;
    await mock.send(cmd("Project", { type: "Rename", id: beat.id, name: "Beat sketch (local copy)" }));
    const other = store().projects.find((p) => p.id !== beat.id && p.id !== current().id)!;
    markSessionProject(other.id, "jam");

    const dialog = await openScreen();
    const copyRow = await within(dialog).findByRole("button", { name: "Open Beat sketch (local copy)" });
    expect(within(copyRow).getByText("Local copy")).toBeInTheDocument();
    // The suffix shows as the badge, not in the name.
    expect(within(copyRow).getByTitle("Beat sketch (local copy)").textContent).toBe("Beat sketch");
    const sessionRow = within(dialog).getByRole("button", { name: `Open ${other.name}` });
    expect(within(sessionRow).getByText("Collab")).toBeInTheDocument();
    expect(within(sessionRow).getByTitle("Last used in the collaboration session “jam”")).toBeInTheDocument();
  });

  it("remembers the open project while online", async () => {
    await renderWithMock(<ProjectMenu />);
    act(() => useCollabStore.setState({ status: { type: "Online", session: "late-night", site: "1" } }));
    await waitFor(() => expect(JSON.parse(localStorage.getItem("eth-collab-projects") ?? "{}")).toEqual({ [current().id]: "late-night" }));
  });

  it("splits the local copy suffix", () => {
    expect(splitLocalCopy("Song (local copy)")).toEqual({ base: "Song", localCopy: true });
    expect(splitLocalCopy("Song")).toEqual({ base: "Song", localCopy: false });
    expect(splitLocalCopy(" (local copy)")).toEqual({ base: " (local copy)", localCopy: false });
  });
});
