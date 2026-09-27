import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { pickOption } from "@/kit/testing";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { AUTOSAVE_MS, ProjectMenu } from "./index";

const store = () => useProjectStore.getState();
const names = () => store().projects.map((p) => p.name).sort();

afterEach(resetStores);

/**
 * The button named `name` in `container`, once it exists and is enabled. The manager
 * disables its buttons while a store command is in flight, and the project list in the
 * store updates from `Event::Project` BEFORE that command's reply. Waiting only for the
 * store can therefore click a still-disabled button (a no-op); always wait for this.
 */
async function enabledButton(container: HTMLElement, name: string): Promise<HTMLElement> {
  const button = await within(container).findByRole("button", { name });
  await waitFor(() => expect(button).toBeEnabled());
  return button;
}

async function openManager() {
  fireEvent.click(screen.getByRole("button", { name: "Projects" }));
  return screen.findByRole("dialog", { name: "Projects" });
}

describe("ProjectMenu", () => {
  it("sets the global scale from the project menu and saves it", async () => {
    const { mock } = await renderWithMock(<ProjectMenu />);
    await openManager();
    pickOption(screen.getByLabelText("Project scale type"), { value: "Minor" });
    await waitFor(() => expect(store().project!.settings.scale.kind).toBe("Minor"));
    pickOption(screen.getByLabelText("Project scale root"), { value: "3" });
    await waitFor(() => expect(store().project!.settings.scale.root).toBe(3));
    const id = store().project!.id;
    await mock.send(cmd("Project", { type: "Save" }));
    await mock.send(cmd("Project", { type: "SetScale", scale: { root: 0, kind: "Chromatic" } }));
    await mock.send(cmd("Edit", { type: "Undo" }));
    await mock.send(cmd("Project", { type: "Open", id }));
    await waitFor(() => expect(store().project!.settings.scale).toEqual({ root: 3, kind: "Minor" }));
  });

  it("renders without an engine", () => {
    render(<ProjectMenu />);
    expect(screen.getByTestId("project-name").textContent).toBe("No project");
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
  });

  it("shows the dirty indicator, autosaves a second after the last change, and saves on Ctrl+S", async () => {
    const { mock } = await renderWithMock(<ProjectMenu />);
    expect(screen.getByTestId("project-name").textContent).toBe("Demo");
    expect(screen.queryByLabelText("Unsaved changes")).toBeNull();

    await mock.send(cmd("Transport", { type: "SetMetronome", enabled: true }));
    await screen.findByLabelText("Unsaved changes");
    // Not saved right away; saved once edits stop for AUTOSAVE_MS.
    await new Promise((r) => setTimeout(r, AUTOSAVE_MS / 2));
    expect(store().dirty).toBe(true);
    await waitFor(() => expect(screen.queryByLabelText("Unsaved changes")).toBeNull(), { timeout: AUTOSAVE_MS * 3 });

    await mock.send(cmd("Transport", { type: "SetMetronome", enabled: false }));
    await screen.findByLabelText("Unsaved changes");
    fireEvent.keyDown(window, { key: "s", metaKey: true });
    await waitFor(() => expect(store().dirty).toBe(false));
  });

  it("lists stored projects, current one marked and not deletable", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();
    await waitFor(() => expect(within(dialog).getAllByRole("listitem")).toHaveLength(3));
    const current = within(dialog).getAllByRole("listitem").find((li) => li.getAttribute("aria-current"))!;
    expect(current.textContent).toMatch(/Demo/);
    expect(within(current).getByRole("button", { name: "Delete Demo" })).toBeDisabled();
    expect(within(current).getByRole("button", { name: "Open Demo" })).toBeDisabled();
  });

  it("creates a new project and opens it", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();
    fireEvent.change(within(dialog).getByLabelText("New project name"), { target: { value: "Song" } });
    fireEvent.click(await enabledButton(dialog, "New"));
    // The popover closes once the reply arrives, after the ProjectLoaded event: wait for it.
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByTestId("project-name").textContent).toBe("Song");
    expect(names()).toContain("Song");
  });

  it("opens another project", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();
    fireEvent.click(await enabledButton(dialog, "Open Beat sketch"));
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toBe("Beat sketch"));
  });

  it("saves as a copy and switches to it", async () => {
    await renderWithMock(<ProjectMenu />);
    const before = store().project!.id;
    const dialog = await openManager();
    fireEvent.change(within(dialog).getByLabelText("Save as name"), { target: { value: "Demo v2" } });
    fireEvent.click(await enabledButton(dialog, "Save as"));
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toBe("Demo v2"));
    expect(store().project!.id).not.toBe(before);
    expect(names()).toEqual(["Ambient idea", "Beat sketch", "Demo", "Demo v2"]);
  });

  it("duplicates, renames and deletes stored projects", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();

    fireEvent.click(await enabledButton(dialog, "Duplicate Beat sketch"));
    await waitFor(() => expect(names()).toContain("Beat sketch copy"));

    fireEvent.click(await enabledButton(dialog, "Rename Beat sketch copy"));
    const input = await within(dialog).findByLabelText("New name for Beat sketch copy");
    fireEvent.change(input, { target: { value: "Beats 2" } });
    fireEvent.submit(input);
    await waitFor(() => expect(names()).toContain("Beats 2"));
    expect(names()).not.toContain("Beat sketch copy");

    fireEvent.click(await enabledButton(dialog, "Delete Beats 2"));
    fireEvent.click(await enabledButton(dialog, "Confirm delete Beats 2"));
    await waitFor(() => expect(names()).not.toContain("Beats 2"));
  });

  it("renames the current project (undoable document edit)", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();
    fireEvent.click(await enabledButton(dialog, "Rename Demo"));
    const input = await within(dialog).findByLabelText("New name for Demo");
    fireEvent.change(input, { target: { value: "My song" } });
    fireEvent.submit(input);
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toBe("My song"));
    expect(store().history.can_undo).toBe(true);
  });

  it("closes on Escape", async () => {
    await renderWithMock(<ProjectMenu />);
    await openManager();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
