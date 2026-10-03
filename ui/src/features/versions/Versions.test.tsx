import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectScreen } from "@/features/project/screenStore";
import { useProjectStore } from "@/state";
import { cmd, MockTransport, TransportProvider } from "@/transport";
import { VERSION_INTERVAL_MS } from "@/transport/mock/roadmap/versions";
import { openVersions, useVersionsDialog, VersionsRoot } from "./index";
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
    const dialog = await screen.findByRole("dialog", { name: /Versions of/ });
    expect(await within(dialog).findByText(/No versions yet/)).toBeInTheDocument();

    const before = trackCount();
    fireEvent.change(within(dialog).getByLabelText("Version name"), { target: { value: "Before bass" } });
    fireEvent.click(await enabled(dialog, "Save version"));
    expect(await within(dialog).findByText("Before bass")).toBeInTheDocument();

    await addTrack(mock, "Bass");
    expect(trackCount()).toBe(before + 1);

    fireEvent.click(await enabled(dialog, "Compare Before bass with now"));
    const diff = await within(dialog).findByLabelText("Changes since this version");
    expect(within(diff).getByText("Tracks")).toBeInTheDocument();
    expect(within(diff).getByText("+1")).toBeInTheDocument();
    expect(within(diff).getByText("Bass")).toBeInTheDocument();

    fireEvent.click(await enabled(dialog, "Restore Before bass"));
    await waitFor(() => expect(trackCount()).toBe(before));
    expect(await within(dialog).findByRole("status")).toHaveTextContent(/Restored “Before bass”/);
    // The state before the restore is a version too: restoring it brings the track back.
    fireEvent.click(await enabled(dialog, "Restore Before restore"));
    await waitFor(() => expect(trackCount()).toBe(before + 1));

    // Rename (via the row menu) and delete with confirmation.
    const { useContextMenuStore } = await import("@/kit");
    fireEvent.click(await enabled(dialog, "More actions for Before bass"));
    const rename = useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === "Rename");
    act(() => (rename as { onSelect(): void }).onSelect());
    const input = await within(dialog).findByLabelText("New name for Before bass");
    fireEvent.change(input, { target: { value: "Bassless" } });
    fireEvent.submit(input.closest("form")!);
    expect(await within(dialog).findByText("Bassless")).toBeInTheDocument();

    fireEvent.click(await enabled(dialog, "More actions for Bassless"));
    const del = useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === "Delete…");
    act(() => (del as { onSelect(): void }).onSelect());
    fireEvent.click(await enabled(dialog, "Confirm delete Bassless"));
    await waitFor(() => expect(within(dialog).queryByText("Bassless")).not.toBeInTheDocument());
  });

  it("lists autosave versions as they roll", async () => {
    const { mock } = await renderWithMock(<VersionsRoot />);
    act(() => openVersions());
    const dialog = await screen.findByRole("dialog", { name: /Versions of/ });
    await within(dialog).findByText(/No versions yet/);
    await addTrack(mock, "Keys");
    act(() => mock.tick(VERSION_INTERVAL_MS));
    expect(await within(dialog).findByText("Autosave")).toBeInTheDocument();
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
    const dialog = await screen.findByRole("dialog", { name: "Recover unsaved work?" });
    // The project screen waits behind it.
    expect(useRecoveryShown.getState().shown).toBe(true);
    const name = mock.snapshot().settings.name;
    expect(within(dialog).getByText(name)).toBeInTheDocument();
    fireEvent.click(await enabled(dialog, `Recover ${name}`));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Recover unsaved work?" })).not.toBeInTheDocument());
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
    const dialog = await screen.findByRole("dialog", { name: "Recover unsaved work?" });
    const name = mock.snapshot().settings.name;
    fireEvent.click(await enabled(dialog, `Keep the saved ${name}`));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Recover unsaved work?" })).not.toBeInTheDocument());
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
    const dialog = await screen.findByRole("dialog", { name: "Ethereal didn’t close properly" });
    expect(within(dialog).getByText("No unsaved work")).toBeInTheDocument();
    fireEvent.click(await enabled(dialog, `Open ${name} without plugins`));
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
