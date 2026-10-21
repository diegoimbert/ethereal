// Project entries of the command palette (`buildCommands`).
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { ConfirmDialog } from "@/kit";
import { MidiLearnRoot } from "@/features/midi-learn";
import { useMidiLearnStore } from "@/features/midi-learn/store";
import { MIDI_MODE_CLASS } from "@/features/midi-learn/useMidiMode";
import { useNotices } from "@/features/notifications";
import { useProjectScreen } from "@/features/project/screenStore";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
import { ProjectMenu } from "@/features/project";
import { CommandPalette } from "./CommandPalette";
import { buildCommands } from "./commands";
import { resetShell } from "@/state/shell";

afterEach(() => {
  resetStores();
  resetShell();
  useProjectScreen.setState({ open: false, mode: "home", launchPending: false });
  useNotices.getState().clear();
  localStorage.clear();
});

const byId = (cmds: ReturnType<typeof buildCommands>, id: string) => cmds.find((c) => c.id === id);

describe("palette: MIDI mode", () => {
  afterEach(() => useMidiLearnStore.getState().reset());

  it("turns MIDI mode on and off without the MIDI tab", async () => {
    const { mock } = await renderWithMock(<MidiLearnRoot />);
    const toggle = () => act(() => byId(buildCommands(mock, []), "transport:midiMode")!.run());
    expect(byId(buildCommands(mock, []), "transport:midiMode")?.label).toBe("turn midi mode on");
    toggle();
    await waitFor(() => expect(document.body).toHaveClass(MIDI_MODE_CLASS));
    expect(byId(buildCommands(mock, []), "transport:midiMode")?.label).toBe("turn midi mode off");
    toggle();
    await waitFor(() => expect(document.body).not.toHaveClass(MIDI_MODE_CLASS));
    // No MIDI input on the web build: no entry.
    Object.defineProperty(mock, "kind", { value: "wasm" });
    expect(byId(buildCommands(mock, []), "transport:midiMode")).toBeUndefined();
  });
});

describe("palette: project commands", () => {
  it("lists the project actions", async () => {
    const { mock } = await renderWithMock(<CommandPalette />);
    const cmds = buildCommands(mock, []);
    const project = cmds.filter((c) => c.group === "Project").map((c) => c.label);
    expect(project).toEqual([
      "new project",
      "open project…",
      "save",
      "save as…",
      "duplicate project",
      "rename project",
      "import project…",
      "export project…",
      "export audio…",
      // The versions dialog, and a named version.
      "versions…",
      "save snapshot…",
      "delete project…",
    ]);
    // No engine: no project entries.
    expect(buildCommands(null, []).some((c) => c.group === "Project")).toBe(false);
  });

  it("new project, save as, rename (in place) and open from the palette", async () => {
    const { mock } = await renderWithMock(<CommandPalette />);
    const cmds = buildCommands(mock, []);
    const before = useProjectStore.getState().project!.id;
    act(() => byId(cmds, "project:new")!.run());
    // A new draft project, named after today's date, straight away (no form).
    await waitFor(() => expect(useProjectStore.getState().project!.id).not.toBe(before));
    expect(useProjectStore.getState().project!.settings.name).toBe(`${new Date().toLocaleDateString()} project`);
    act(() => byId(cmds, "project:save-as")!.run());
    expect(useProjectScreen.getState()).toMatchObject({ open: true, mode: "saveAs" });
    act(() => useProjectScreen.getState().hide());
    const request = useProjectScreen.getState().renameRequest;
    act(() => byId(cmds, "project:rename")!.run());
    // Renaming happens in the top bar: the picker stays closed.
    expect(useProjectScreen.getState()).toMatchObject({ open: false, renameRequest: request + 1 });
    act(() => byId(cmds, "project:open")!.run());
    expect(useProjectScreen.getState()).toMatchObject({ open: true, mode: "home" });
  });

  it("saves and duplicates from the palette", async () => {
    const { mock } = await renderWithMock(<CommandPalette />);
    await mock.send({ domain: "Transport", command: { type: "SetMetronome", enabled: true } });
    await waitFor(() => expect(useProjectStore.getState().dirty).toBe(true));
    act(() => byId(buildCommands(mock, []), "project:save")!.run());
    await waitFor(() => expect(useProjectStore.getState().dirty).toBe(false));
    act(() => byId(buildCommands(mock, []), "project:duplicate")!.run());
    await waitFor(() => expect(useProjectStore.getState().projects.map((p) => p.name)).toContain("Demo copy"));
  });

  it("palette labels are unique (no two identical rows), playing or stopped (#577)", async () => {
    const { mock } = await renderWithMock(<CommandPalette />);
    const dupes = (labels: string[]) => labels.filter((l, i) => labels.indexOf(l) !== i);
    expect(dupes(buildCommands(mock, []).map((c) => c.label))).toEqual([]);
    await mock.send({ domain: "Transport", command: { type: "Play" } });
    await waitFor(() => expect(useProjectStore.getState().transport?.playing).toBe(true));
    expect(dupes(buildCommands(mock, []).map((c) => c.label))).toEqual([]);
  });

  it("every project menu item is a palette command under the same label, and back (#577)", async () => {
    const { mock } = await renderWithMock(<ProjectMenu />);
    const palette = buildCommands(mock, []);
    fireEvent.click(screen.getByRole("button", { name: "projects" }));
    const menu = await screen.findByRole("menu", { name: "project" });
    const read = () =>
      within(menu)
        .getAllByRole("menuitem")
        .filter((el) => el.getAttribute("aria-haspopup") !== "menu")
        .map((el) => ({ label: el.textContent!, inSub: !!el.closest(".eth-project-menu__submenu") }));
    const items = read();
    expect(items.length).toBeGreaterThan(12);
    for (const { label } of items) {
      const match = palette.find((c) => c.label === label);
      expect(match, `menu item "${label}" has no palette command`).toBeDefined();
    }
    // The palette's project actions are all in the menu ("open project…" as the submenu of
    // the recent projects).
    const labels = new Set(items.map((i) => i.label));
    expect(within(menu).getByRole("menuitem", { name: "open project" })).toHaveAttribute("aria-haspopup", "menu");
    for (const c of palette.filter((c) => c.group === "Project" && c.id !== "project:open")) {
      expect(labels.has(c.label), `palette "${c.label}" is not in the menu`).toBe(true);
    }
  });

  it("delete project… in the palette asks first, then deletes the open project", async () => {
    const { mock } = await renderWithMock(
      <>
        <CommandPalette />
        <ConfirmDialog />
      </>,
    );
    const id = useProjectStore.getState().project!.id;
    act(() => byId(buildCommands(mock, []), "project:delete")!.run());
    const dialog = await screen.findByRole("dialog", { name: /delete “Demo”\?/ });
    expect(useProjectStore.getState().projects.some((p) => p.id === id)).toBe(true);
    fireEvent.click(within(dialog).getByRole("button", { name: "delete" }));
    await waitFor(() => expect(useProjectStore.getState().projects.some((p) => p.id === id)).toBe(false));
  });

  it("are found by typing in the palette", async () => {
    await renderWithMock(<CommandPalette />);
    act(() => void fireEvent.keyDown(window, { key: "k", metaKey: true }));
    fireEvent.change(screen.getByRole("combobox", { name: "search commands" }), { target: { value: "export project" } });
    const first = within(screen.getByRole("listbox", { name: "commands" })).getAllByRole("option")[0]!;
    expect(first).toHaveTextContent("export project…");
  });
});
