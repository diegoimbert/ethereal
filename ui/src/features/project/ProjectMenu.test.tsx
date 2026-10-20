import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { pickOption } from "@/kit/testing";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { useContextMenuStore } from "@/kit";
import { AUTOSAVE_MS, ProjectMenu } from "./index";
import { useProjectScreen } from "./screenStore";

const store = () => useProjectStore.getState();
const names = () =>
  store()
    .projects.map((p) => p.name)
    .sort();

afterEach(() => {
  resetStores();
  useProjectScreen.setState({ open: false, launchPending: false });
});

/** Run the context-menu item `label` of the stored project `name` (via its "…" button). */
async function projectAction(dialog: HTMLElement, name: string, label: string) {
  fireEvent.click(await enabledButton(dialog, `more actions for ${name}`));
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
  fireEvent.click(screen.getByRole("button", { name: "projects" }));
  fireEvent.click(await screen.findByRole("menuitem", { name: "open project…" }));
  return screen.findByRole("dialog", { name: "open project" });
}

describe("ProjectMenu", () => {
  it("sets the global scale from the project menu and saves it", async () => {
    const { mock } = await renderWithMock(<ProjectMenu />);
    fireEvent.click(screen.getByRole("button", { name: "projects" }));
    await screen.findByRole("menu", { name: "project" });
    pickOption(screen.getByLabelText("project scale type"), { value: "Minor" });
    await waitFor(() => expect(store().project!.settings.scale.kind).toBe("Minor"));
    pickOption(screen.getByLabelText("project scale root"), { value: "3" });
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
    expect(screen.getByTestId("project-name").textContent).toBe("no project");
    expect(screen.queryByRole("button", { name: "save" })).toBeNull();
  });

  it("shows the dirty indicator, autosaves a second after the last change, and saves on Ctrl+S", async () => {
    const { mock } = await renderWithMock(<ProjectMenu />);
    expect(screen.getByTestId("project-name").textContent).toBe("Demo");
    expect(screen.queryByLabelText("unsaved changes")).toBeNull();
    const send = vi.spyOn(mock, "send");
    const saves = () => send.mock.calls.map(([c]) => c).flatMap((c) => (c.domain === "Project" && c.command.type === "Save" ? [c.command] : []));

    await mock.send(cmd("Transport", { type: "SetMetronome", enabled: true }));
    await screen.findByLabelText("unsaved changes");
    // Not saved right away; saved once edits stop for AUTOSAVE_MS.
    await new Promise((r) => setTimeout(r, AUTOSAVE_MS / 2));
    expect(store().dirty).toBe(true);
    await waitFor(() => expect(screen.queryByLabelText("unsaved changes")).toBeNull(), { timeout: AUTOSAVE_MS * 3 });
    // An autosave: the engine may re-read only the plugins that changed.
    expect(saves()).toEqual([{ type: "Save", autosave: true }]);

    await mock.send(cmd("Transport", { type: "SetMetronome", enabled: false }));
    await screen.findByLabelText("unsaved changes");
    fireEvent.keyDown(window, { key: "s", metaKey: true });
    await waitFor(() => expect(store().dirty).toBe(false));
    // An explicit save: every plugin's live state.
    expect(saves()[1]).toEqual({ type: "Save" });
  });

  it("shows the project screen once on launch, with the open project", async () => {
    useProjectScreen.setState({ launchPending: true });
    await renderWithMock(<ProjectMenu />);
    const dialog = await screen.findByRole("dialog", { name: "open project" });
    expect(within(dialog).getByRole("button", { name: "new project" })).toBeInTheDocument();
    // The other stored projects, not the open one.
    await waitFor(() => expect(within(within(dialog).getByRole("list", { name: "stored projects" })).getAllByRole("listitem")).toHaveLength(2));
    // The welcome screen: the wordmark and Get started next to the projects.
    expect(within(dialog).getByText("ethereal")).toBeInTheDocument();
    expect(within(dialog).getByRole("region", { name: "get started" })).toBeInTheDocument();
    // The search over the recent projects is focused and filters them.
    const search = within(dialog).getByRole("searchbox", { name: "search projects" });
    await waitFor(() => expect(document.activeElement).toBe(search));
    const list = within(dialog).getByRole("list", { name: "stored projects" });
    const first = within(list)
      .getAllByRole("button", { name: /^open / })[0]!
      .getAttribute("aria-label")!
      .slice("open ".length);
    fireEvent.change(search, { target: { value: first.slice(0, 4).toUpperCase() } });
    expect(
      within(list)
        .getAllByRole("button", { name: /^open / })
        .every((b) => b.getAttribute("aria-label")!.toLowerCase().includes(first.slice(0, 4).toLowerCase())),
    ).toBe(true);
    fireEvent.change(search, { target: { value: "zzz-no-such-project" } });
    expect(within(dialog).getByText(/no project matches/)).toBeInTheDocument();
    fireEvent.change(search, { target: { value: "" } });
    expect(within(dialog).queryByRole("button", { name: "open Demo" })).toBeNull();
    // Dismissed: back to the open project.
    fireEvent.keyDown(dialog, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByTestId("project-name").textContent).toBe("Demo");
    expect(useProjectScreen.getState().launchPending).toBe(false);
  });

  it('the welcome screen has no close button: Escape leaves it, "new project" ends its row', async () => {
    useProjectScreen.setState({ launchPending: true });
    await renderWithMock(<ProjectMenu />);
    const dialog = await screen.findByRole("dialog", { name: "open project" });
    expect(within(dialog).queryByRole("button", { name: "close" })).toBeNull();
    // The header's actions: "join discord" first, "new project" at the very right.
    const actions = within(dialog).getByTestId("welcome-new-project").parentElement!;
    expect(actions.lastElementChild).toBe(within(dialog).getByTestId("welcome-new-project"));
    expect(actions.firstElementChild).toBe(within(dialog).getByRole("button", { name: "join discord" }));
    fireEvent.keyDown(dialog, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByTestId("project-name").textContent).toBe("Demo");
  });

  it("creates a new project at once, named after today's date (numbered when taken)", async () => {
    await renderWithMock(<ProjectMenu />);
    const today = `${new Date().toLocaleDateString()} project`;
    const dialog = await openManager();
    fireEvent.click(within(dialog).getByRole("button", { name: "new project" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toBe(today));
    // A second one the same day gets a number.
    fireEvent.click(screen.getByRole("button", { name: "projects" }));
    fireEvent.click(await screen.findByRole("menuitem", { name: "new project" }));
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toBe(`${today} (2)`));
  });

  it("opens another project", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();
    fireEvent.click(await enabledButton(dialog, "open Beat sketch"));
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toBe("Beat sketch"));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("duplicates, renames and deletes stored projects", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();

    await projectAction(dialog, "Beat sketch", "duplicate");
    await waitFor(() => expect(names()).toContain("Beat sketch copy"));

    await projectAction(dialog, "Beat sketch copy", "rename");
    const input = await within(dialog).findByLabelText("new name for Beat sketch copy");
    fireEvent.change(input, { target: { value: "Beats 2" } });
    fireEvent.submit(input);
    await waitFor(() => expect(names()).toContain("Beats 2"));
    expect(names()).not.toContain("Beat sketch copy");

    await projectAction(dialog, "Beats 2", "delete…");
    fireEvent.click(await enabledButton(dialog, "confirm delete Beats 2"));
    await waitFor(() => expect(names()).not.toContain("Beats 2"));
  });

  it("renames the open project in place: a click on its name (undoable document edit)", async () => {
    await renderWithMock(<ProjectMenu />);
    fireEvent.click(await enabledButton(document.body, "Demo"));
    const input = screen.getByLabelText("project name");
    expect(input).toHaveFocus();
    fireEvent.change(input, { target: { value: "My song" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toBe("My song"));
    expect(store().history.can_undo).toBe(true);
  });

  it("exports the project from a dialog: with its media by default, or the document alone", async () => {
    URL.createObjectURL = vi.fn(() => "blob:mock");
    URL.revokeObjectURL = vi.fn();
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(() => undefined);
    const { mock } = await renderWithMock(<ProjectMenu />);
    const sent: unknown[] = [];
    const send = mock.send.bind(mock);
    mock.send = (c, o) => {
      if (c.domain === "Project" && c.command.type === "ExportBundle") sent.push(c.command.include_media);
      return send(c, o);
    };
    const exportOnce = async (pack: boolean) => {
      fireEvent.click(screen.getByRole("button", { name: "projects" }));
      fireEvent.click(await screen.findByRole("menuitem", { name: "export project…" }));
      const dialog = await screen.findByRole("dialog", { name: "export project" });
      const toggle = within(dialog).getByRole("switch", { name: "pack media files" });
      if ((toggle.getAttribute("aria-checked") === "true") !== pack) fireEvent.click(toggle);
      fireEvent.click(within(dialog).getByRole("button", { name: "export" }));
      await waitFor(() => expect(screen.queryByRole("dialog", { name: "export project" })).toBeNull());
    };
    await exportOnce(true);
    await exportOnce(false);
    expect(sent).toEqual([true, false]);
    vi.restoreAllMocks();
  });

  it("has the menus' keyboard model: first item focused, ↑/↓ move and wrap", async () => {
    await renderWithMock(<ProjectMenu />);
    fireEvent.click(screen.getByRole("button", { name: "projects" }));
    const menu = await screen.findByRole("menu", { name: "project" });
    const item = (name: string) => within(menu).getByRole("menuitem", { name });
    await waitFor(() => expect(item("new project")).toHaveFocus());
    fireEvent.keyDown(item("new project"), { key: "ArrowDown" });
    expect(item("open project…")).toHaveFocus();
    fireEvent.keyDown(item("open project…"), { key: "ArrowUp" });
    expect(item("new project")).toHaveFocus();
    fireEvent.keyDown(item("new project"), { key: "ArrowUp" }); // wraps to the last item
    expect(within(menu).getAllByRole("menuitem").at(-1)).toHaveFocus();
  });

  it("closes on Escape", async () => {
    await renderWithMock(<ProjectMenu />);
    const dialog = await openManager();
    fireEvent.keyDown(dialog, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});
