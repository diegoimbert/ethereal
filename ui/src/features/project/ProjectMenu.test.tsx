import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { pickOption } from "@/kit/testing";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { useContextMenuStore } from "@/kit";
import { AUTOSAVE_MS, ProjectMenu } from "./index";
import { useProjectScreen } from "./screenStore";

const store = () => useProjectStore.getState();
const names = () => store().projects.map((p) => p.name).sort();

afterEach(() => {
  resetStores();
  useProjectScreen.setState({ open: false, launchPending: false });
});

/** Run the context-menu item `label` of the stored project `name` (via its "…" button). */
async function projectAction(dialog: HTMLElement, name: string, label: string) {
  fireEvent.click(await enabledButton(dialog, `More actions for ${name}`));
  const item = useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === label);
  if (!item || item === "separator") throw new Error(`no ${label} item`);
  act(() => item.onSelect());
}

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

  it("shows the project screen once on launch, with the open project", async () => {
    useProjectScreen.setState({ launchPending: true });
    await renderWithMock(<ProjectMenu />);
    const dialog = await screen.findByRole("dialog", { name: "Projects" });
    expect(within(dialog).getByLabelText("Project name")).toHaveValue("Demo");
    expect(within(dialog).getByRole("button", { name: "New project" })).toBeInTheDocument();
    // The other stored projects, not the open one.
    await waitFor(() => expect(within(dialog).getAllByRole("listitem")).toHaveLength(2));
    expect(within(dialog).queryByRole("button", { name: "Open Demo" })).toBeNull();
    fireEvent.click(within(dialog).getByRole("button", { name: "Continue" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(useProjectScreen.getState().launchPending).toBe(false);
  });

  it("creates a new project after asking for its name", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();
    fireEvent.click(within(dialog).getByRole("button", { name: "New project" }));
    fireEvent.change(await within(dialog).findByLabelText("New project name"), { target: { value: "Song" } });
    fireEvent.click(await enabledButton(dialog, "Create"));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByTestId("project-name").textContent).toBe("Song");
    expect(names()).toContain("Song");
  });

  it("opens another project", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();
    fireEvent.click(await enabledButton(dialog, "Open Beat sketch"));
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toBe("Beat sketch"));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("duplicates, renames and deletes stored projects", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();

    await projectAction(dialog, "Beat sketch", "Duplicate");
    await waitFor(() => expect(names()).toContain("Beat sketch copy"));

    await projectAction(dialog, "Beat sketch copy", "Rename");
    const input = await within(dialog).findByLabelText("New name for Beat sketch copy");
    fireEvent.change(input, { target: { value: "Beats 2" } });
    fireEvent.submit(input);
    await waitFor(() => expect(names()).toContain("Beats 2"));
    expect(names()).not.toContain("Beat sketch copy");

    await projectAction(dialog, "Beats 2", "Delete…");
    fireEvent.click(await enabledButton(dialog, "Confirm delete Beats 2"));
    await waitFor(() => expect(names()).not.toContain("Beats 2"));
  });

  it("renames the open project from the screen (undoable document edit)", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();
    const input = within(dialog).getByLabelText("Project name");
    fireEvent.change(input, { target: { value: "My song" } });
    fireEvent.submit(input);
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toBe("My song"));
    expect(store().history.can_undo).toBe(true);
  });

  it("closes on Escape", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();
    fireEvent.keyDown(dialog, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});
