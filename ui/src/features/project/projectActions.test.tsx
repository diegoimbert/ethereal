// The open project's actions (save as, duplicate, export/import, close and delete,
// rename) and the recents badges, against the mock engine.
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useNotices } from "@/features/notifications";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
import type { Command } from "@/generated";
import { cmd, type EngineTransport } from "@/transport";
import { exportProject, importAndOpenBundle, importProject, isEtherFileName } from "./actions";
import { ProjectMenu } from "./index";
import { guardLeave } from "./leaveGuard";
import { useProjectScreen } from "./screenStore";
import { splitLocalCopy } from "./sessionMarks";

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
  useNotices.getState().clear();
  localStorage.clear();
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

/** The project picker (as the palette's "open project…" shows it). */
async function openScreen() {
  act(() => useProjectScreen.getState().show());
  return screen.findByRole("dialog", { name: "open project" });
}

/** Open the top bar's project menu. */
async function openMenu() {
  fireEvent.click(screen.getByRole("button", { name: "projects" }));
  return screen.findByRole("menu", { name: "project" });
}

describe("open project actions", () => {
  it("saves as a new project and switches to it", async () => {
    await renderWithMock(<ProjectMenu />);
    const before = current().id;
    // "save as…" is in the command palette (it opens the picker's save-as form).
    act(() => useProjectScreen.getState().show("saveAs"));
    const input = await screen.findByLabelText("save as name");
    expect(input).toHaveAttribute("placeholder", "Demo copy");
    fireEvent.change(input, { target: { value: "Demo v2" } });
    fireEvent.click(screen.getByRole("button", { name: "save" }));
    await waitFor(() => expect(current().settings.name).toBe("Demo v2"));
    expect(current().id).not.toBe(before);
    expect(names()).toEqual(expect.arrayContaining(["Demo", "Demo v2"]));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
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

    // A dropped .ether (importAndOpenBundle): imported (numbered again), then opened.
    expect(isEtherFileName("/Users/me/Songs/Demo.ETHER")).toBe(true);
    expect(isEtherFileName("Kit.als")).toBe(false);
    await act(() => importAndOpenBundle(transport, { file }));
    await waitFor(() => expect(current().settings.name).toBe("Demo 3"));
  });

  it("moves the open project to the bin from the menu by closing it first; the toast restores it", async () => {
    const { mock } = await renderWithMock(<ProjectMenu />);
    const id = current().id;
    const menu = await openMenu();
    fireEvent.click(within(menu).getByRole("menuitem", { name: "move project to the bin" }));
    await waitFor(() => expect(store().projects.some((p) => p.id === id)).toBe(false));
    expect(current().id).not.toBe(id);
    const binned = await mock.send(cmd("Project", { type: "ListBin" }));
    expect(binned.type === "Projects" && binned.projects.map((p) => p.id)).toEqual([id]);
    const notice = useNotices.getState().notices.find((n) => n.message.toLowerCase() === "moved “demo” to the bin");
    act(() => notice!.action!.onSelect());
    await waitFor(() => expect(store().projects.some((p) => p.id === id)).toBe(true));
  });

  it("Rename focuses the name field", async () => {
    await renderWithMock(<ProjectMenu />);
    act(() => useProjectScreen.getState().rename());
    const input = await screen.findByLabelText("project name");
    await waitFor(() => expect(input).toHaveFocus());
  });
});

describe("guardLeave", () => {
  it("runs at once (no collaboration sessions in this build)", async () => {
    const run = vi.fn();
    await expect(guardLeave(run)).resolves.toBe(true);
    expect(run).toHaveBeenCalledOnce();
  });
});

describe("recents badges", () => {
  // Sharing badges ("shared", "From …") come from the engine: recentsShared.test.tsx.
  it("marks local copies", async () => {
    const { mock } = await renderWithMock(<ProjectMenu />);
    const beat = store().projects.find((p) => p.name === "Beat sketch")!;
    await mock.send(cmd("Project", { type: "Rename", id: beat.id, name: "Beat sketch (local copy)" }));

    const dialog = await openScreen();
    const copyRow = await within(dialog).findByRole("button", { name: "open Beat sketch (local copy)" });
    expect(within(copyRow).getByText("local copy")).toBeInTheDocument();
    // The suffix shows as the badge, not in the name.
    expect(within(copyRow).getByTitle("Beat sketch (local copy)").textContent).toBe("Beat sketch");
  });

  it("shows where a fork came from", async () => {
    const { mock } = await renderWithMock(<ProjectMenu />);
    const beat = store().projects.find((p) => p.name === "Beat sketch")!;
    await mock.send(cmd("Project", { type: "Duplicate", id: beat.id, new_id: "0190a000-0000-7000-8000-00000000f0f0", name: "Beat remix" }));

    const dialog = await openScreen();
    const fork = await within(dialog).findByRole("button", { name: "open Beat remix" });
    expect(within(fork).getByTitle("forked from Beat sketch")).toBeInTheDocument();
    const original = within(dialog).getByRole("button", { name: "open Beat sketch" });
    expect(within(original).queryByTitle(/forked from/)).toBeNull();
  });

  it("splits the local copy suffix", () => {
    expect(splitLocalCopy("Song (local copy)")).toEqual({ base: "Song", localCopy: true });
    expect(splitLocalCopy("Song")).toEqual({ base: "Song", localCopy: false });
    expect(splitLocalCopy(" (local copy)")).toEqual({ base: " (local copy)", localCopy: false });
  });
});
