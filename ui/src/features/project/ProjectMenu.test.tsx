import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { ProjectMenu } from "./index";

const store = () => useProjectStore.getState();
const names = () => store().projects.map((p) => p.name).sort();

afterEach(resetStores);

async function openManager() {
  fireEvent.click(screen.getByRole("button", { name: "Projects" }));
  return screen.findByRole("dialog", { name: "Projects" });
}

describe("ProjectMenu", () => {
  it("renders without an engine", () => {
    render(<ProjectMenu />);
    expect(screen.getByTestId("project-name").textContent).toBe("No project");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("shows the dirty indicator and saves (button and Ctrl+S)", async () => {
    const { mock } = await renderWithMock(<ProjectMenu />);
    expect(screen.getByTestId("project-name").textContent).toBe("Demo");
    expect(screen.queryByLabelText("Unsaved changes")).toBeNull();

    await mock.send(cmd("Transport", { type: "SetMetronome", enabled: true }));
    await screen.findByLabelText("Unsaved changes");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByLabelText("Unsaved changes")).toBeNull());

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
    fireEvent.click(within(dialog).getByRole("button", { name: "New" }));
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toBe("Song"));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(names()).toContain("Song");
  });

  it("opens another project", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();
    fireEvent.click(await within(dialog).findByRole("button", { name: "Open Beat sketch" }));
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toBe("Beat sketch"));
  });

  it("saves as a copy and switches to it", async () => {
    await renderWithMock(<ProjectMenu />);
    const before = store().project!.id;
    const dialog = await openManager();
    fireEvent.change(within(dialog).getByLabelText("Save as name"), { target: { value: "Demo v2" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save as" }));
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toBe("Demo v2"));
    expect(store().project!.id).not.toBe(before);
    expect(names()).toEqual(["Ambient idea", "Beat sketch", "Demo", "Demo v2"]);
  });

  it("duplicates, renames and deletes stored projects", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();

    fireEvent.click(await within(dialog).findByRole("button", { name: "Duplicate Beat sketch" }));
    await waitFor(() => expect(names()).toContain("Beat sketch copy"));

    fireEvent.click(within(dialog).getByRole("button", { name: "Rename Beat sketch copy" }));
    const input = within(dialog).getByLabelText("New name for Beat sketch copy");
    fireEvent.change(input, { target: { value: "Beats 2" } });
    fireEvent.submit(input);
    await waitFor(() => expect(names()).toContain("Beats 2"));
    expect(names()).not.toContain("Beat sketch copy");

    fireEvent.click(within(dialog).getByRole("button", { name: "Delete Beats 2" }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Confirm delete Beats 2" }));
    await waitFor(() => expect(names()).not.toContain("Beats 2"));
  });

  it("renames the current project (undoable document edit)", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();
    fireEvent.click(await within(dialog).findByRole("button", { name: "Rename Demo" }));
    const input = within(dialog).getByLabelText("New name for Demo");
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
