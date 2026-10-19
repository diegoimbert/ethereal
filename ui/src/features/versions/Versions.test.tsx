import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectScreen } from "@/features/project/screenStore";
import { useProjectStore } from "@/state";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { VERSION_INTERVAL_MS } from "@/transport/mock/roadmap/versions";
import { buildCommands } from "@/app/shell/commands";
import { chordsFor } from "@/features/keymap";
import { useNotices } from "@/features/notifications";
import { openVersions, useSnapshotPrompt, useVersionsDialog, VersionsRoot } from "./index";
import { useRecoveryShown } from "./store";

const trackCount = () => Object.keys(useProjectStore.getState().project!.tracks).length;
let seq = 0;
const newId = () => `01KV${String(++seq).padStart(22, "0")}`;
const addTrack = (mock: MockTransport, name: string) =>
  mock.send(cmd("Track", { type: "Create", id: newId(), kind: "Audio", name, color: null, parent: null, before: null }));

afterEach(() => {
  resetStores();
  useVersionsDialog.setState({ open: false });
  useProjectScreen.setState({ open: false, launchPending: false });
});

async function enabled(container: HTMLElement, name: string | RegExp) {
  const b = await within(container).findByRole("button", { name });
  await waitFor(() => expect(b).toBeEnabled());
  return b;
}

describe("VersionsDialog", () => {
  it("saves, compares, restores, renames and deletes versions", async () => {
    const { mock } = await renderWithMock(<VersionsRoot />);
    act(() => openVersions());
    const dialog = await screen.findByRole("dialog", { name: /versions of/ });
    expect(await within(dialog).findByText(/no versions yet/)).toBeInTheDocument();

    const before = trackCount();
    fireEvent.change(within(dialog).getByLabelText("version name"), { target: { value: "Before bass" } });
    fireEvent.click(await enabled(dialog, "save version"));
    expect(await within(dialog).findByText("Before bass")).toBeInTheDocument();

    await addTrack(mock, "Bass");
    expect(trackCount()).toBe(before + 1);

    fireEvent.click(await enabled(dialog, "compare Before bass with now"));
    const diff = await within(dialog).findByLabelText("changes since this version");
    expect(within(diff).getByText("tracks")).toBeInTheDocument();
    expect(within(diff).getByText("+1")).toBeInTheDocument();
    expect(within(diff).getByText("Bass")).toBeInTheDocument();

    fireEvent.click(await enabled(dialog, "restore Before bass"));
    await waitFor(() => expect(trackCount()).toBe(before));
    expect(await within(dialog).findByRole("status")).toHaveTextContent(/restored “Before bass”/);
    // The state before the restore is a version too: restoring it brings the track back.
    fireEvent.click(await enabled(dialog, "restore before restore"));
    await waitFor(() => expect(trackCount()).toBe(before + 1));

    // Rename (via the row menu) and delete with confirmation.
    const { useContextMenuStore } = await import("@/kit");
    fireEvent.click(await enabled(dialog, "more actions for Before bass"));
    const rename = useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === "rename");
    act(() => (rename as { onSelect(): void }).onSelect());
    const input = await within(dialog).findByLabelText("new name for Before bass");
    fireEvent.change(input, { target: { value: "Bassless" } });
    fireEvent.submit(input.closest("form")!);
    expect(await within(dialog).findByText("Bassless")).toBeInTheDocument();

    fireEvent.click(await enabled(dialog, "more actions for Bassless"));
    const del = useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === "delete…");
    act(() => (del as { onSelect(): void }).onSelect());
    fireEvent.click(await enabled(dialog, "confirm delete Bassless"));
    await waitFor(() => expect(within(dialog).queryByText("Bassless")).not.toBeInTheDocument());
  });

  it("has a visible close (#578)", async () => {
    await renderWithMock(<VersionsRoot />);
    act(() => openVersions());
    const dialog = await screen.findByRole("dialog", { name: /versions of/ });
    fireEvent.click(within(dialog).getByRole("button", { name: "close" }));
    expect(useVersionsDialog.getState().open).toBe(false);
  });

  it("lists autosave versions as they roll", async () => {
    const { mock } = await renderWithMock(<VersionsRoot />);
    act(() => openVersions());
    const dialog = await screen.findByRole("dialog", { name: /versions of/ });
    await within(dialog).findByText(/no versions yet/);
    await addTrack(mock, "Keys");
    act(() => mock.tick(VERSION_INTERVAL_MS));
    expect(await within(dialog).findByText("autosave")).toBeInTheDocument();
  });
});

describe("save snapshot", () => {
  afterEach(() => {
    useSnapshotPrompt.setState({ open: false });
    useNotices.getState().clear();
  });

  it("the palette command asks for a name and saves a named version, with a toast", async () => {
    const { mock } = await renderWithMock(<VersionsRoot />);
    const command = buildCommands(mock, []).find((c) => c.id === "project:snapshot");
    expect(command).toMatchObject({ label: "save snapshot…", action: "project.snapshot" });
    act(() => command!.run());
    const dialog = await screen.findByRole("dialog", { name: "save snapshot" });
    const save = within(dialog).getByRole("button", { name: "save snapshot" });
    expect(save).toBeDisabled();
    fireEvent.change(within(dialog).getByLabelText("snapshot name"), { target: { value: "  Before mixdown " } });
    fireEvent.click(await enabled(dialog, "save snapshot"));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "save snapshot" })).not.toBeInTheDocument());
    const reply = await mock.send(cmd("Version", { type: "List" }));
    expect(reply?.type === "Versions" && reply.versions).toEqual([expect.objectContaining({ kind: "Manual", name: "Before mixdown" })]);
    expect(useNotices.getState().notices.map((n) => n.message)).toContainEqual(expect.stringContaining("saved snapshot “Before mixdown”"));
  });

  it("opens on Mod+Alt+S (not from text fields)", async () => {
    await renderWithMock(<VersionsRoot />);
    expect(chordsFor("project.snapshot")).toEqual(["Mod+Alt+S"]);
    const input = document.body.appendChild(document.createElement("input"));
    fireEvent.keyDown(input, { key: "s", code: "KeyS", ctrlKey: true, altKey: true });
    expect(useSnapshotPrompt.getState().open).toBe(false);
    input.remove();
    fireEvent.keyDown(window, { key: "ß", code: "KeyS", ctrlKey: true, altKey: true });
    expect(useSnapshotPrompt.getState().open).toBe(true);
    const dialog = await screen.findByRole("dialog", { name: "save snapshot" });
    fireEvent.click(within(dialog).getByRole("button", { name: "cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "save snapshot" })).not.toBeInTheDocument());
  });
});

describe("RecoveryDialog", () => {
  async function crashed() {
    const mock = new MockTransport({ timers: "manual", seed: 7 });
    await mock.connect();
    await mock.send(cmd("Project", { type: "Save" }));
    mock.tick(10);
    await addTrack(mock, "Unsaved idea");
    mock.tick(VERSION_INTERVAL_MS);
    // The mock can't restart: its session marker is left behind as a killed app's would be.
    const id = mock.snapshot().id;
    mock.versions.simulateCrash(id);
    return { mock, id };
  }

  it("offers unsaved work after a crash and recovers it", async () => {
    const { mock, id } = await crashed();
    useProjectScreen.setState({ launchPending: true });
    render(
      <TransportProvider transport={mock}>
        <VersionsRoot />
      </TransportProvider>,
    );
    const dialog = await screen.findByRole("dialog", { name: "recover unsaved work?" });
    // The project screen waits behind it.
    // (Set by an effect once the dialog is open: it can land just after the dialog renders.)
    await waitFor(() => expect(useRecoveryShown.getState().shown).toBe(true));
    const name = mock.snapshot().settings.name;
    expect(within(dialog).getByText(name)).toBeInTheDocument();
    fireEvent.click(await enabled(dialog, `recover ${name}`));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "recover unsaved work?" })).not.toBeInTheDocument());
    await waitFor(() => expect(Object.values(useProjectStore.getState().project!.tracks).some((t) => t.name === "Unsaved idea")).toBe(true));
    expect(useProjectStore.getState().project!.id).toBe(id);
    expect(useProjectStore.getState().dirty).toBe(true);
  });

  it("keeps the saved project when asked", async () => {
    const { mock } = await crashed();
    render(
      <TransportProvider transport={mock}>
        <VersionsRoot />
      </TransportProvider>,
    );
    const dialog = await screen.findByRole("dialog", { name: "recover unsaved work?" });
    const name = mock.snapshot().settings.name;
    fireEvent.click(await enabled(dialog, `keep the saved ${name}`));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "recover unsaved work?" })).not.toBeInTheDocument());
    const again = await mock.send(cmd("Version", { type: "ListRecoverable" }));
    expect(again).toEqual({ type: "Recoverable", projects: [] });
  });

  it("offers a crashed project without unsaved work, and opens it without plugins (base-131)", async () => {
    const mock = new MockTransport({ timers: "manual", seed: 7 });
    await mock.connect();
    const id = mock.snapshot().id;
    const name = mock.snapshot().settings.name;
    mock.versions.simulateCrash(id);
    render(
      <TransportProvider transport={mock}>
        <VersionsRoot />
      </TransportProvider>,
    );
    const dialog = await screen.findByRole("dialog", { name: "ethereal didn’t close properly" });
    expect(within(dialog).getByText("no unsaved work")).toBeInTheDocument();
    fireEvent.click(await enabled(dialog, `open ${name} without plugins`));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await waitFor(() => expect(useProjectStore.getState().safe).toBe(true));
    expect(useProjectStore.getState().project!.id).toBe(id);
    expect(useRecoveryShown.getState().shown).toBe(false);
  });

  it("shows nothing when there is nothing to recover", async () => {
    await renderWithMock(<VersionsRoot />);
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});
