// base-114: project and collab entries of the command palette (`buildCommands`).
import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { useCollabStore } from "@/features/collab/store";
import { useNotices } from "@/features/notifications";
import { useProjectScreen } from "@/features/project";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
import { CommandPalette } from "./CommandPalette";
import { buildCommands } from "./commands";
import { resetShell } from "./shellStore";

afterEach(() => {
  resetStores();
  resetShell();
  useCollabStore.getState().reset();
  useCollabStore.getState().setDialogOpen(false);
  useProjectScreen.setState({ open: false, mode: "home", launchPending: false });
  useNotices.getState().clear();
  localStorage.clear();
});

const byId = (cmds: ReturnType<typeof buildCommands>, id: string) => cmds.find((c) => c.id === id);

describe("palette: project and collab commands", () => {
  it("lists the project actions and Collab: Join… when offline", async () => {
    const { mock } = await renderWithMock(<CommandPalette />);
    const cmds = buildCommands(mock, []);
    const project = cmds.filter((c) => c.group === "Project").map((c) => c.label);
    expect(project).toEqual([
      "New project",
      "Open project…",
      "Save",
      "Save as…",
      "Duplicate project",
      "Rename project",
      "Export project…",
      "Import project…",
    ]);
    expect(byId(cmds, "collab:join")?.label).toBe("Collab: Join session…");
    expect(byId(cmds, "collab:leave")).toBeUndefined();
    // No engine: no project or collab entries.
    expect(buildCommands(null, []).some((c) => c.group === "Project" || c.group === "Collab")).toBe(false);
  });

  it("opens the project screen in the right mode", async () => {
    const { mock } = await renderWithMock(<CommandPalette />);
    const cmds = buildCommands(mock, []);
    act(() => byId(cmds, "project:new")!.run());
    expect(useProjectScreen.getState()).toMatchObject({ open: true, mode: "new" });
    act(() => byId(cmds, "project:save-as")!.run());
    expect(useProjectScreen.getState().mode).toBe("saveAs");
    const before = useProjectScreen.getState().renameRequest;
    act(() => byId(cmds, "project:rename")!.run());
    expect(useProjectScreen.getState()).toMatchObject({ open: true, mode: "home", renameRequest: before + 1 });
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

  it("Collab: Join… opens the dialog; in a session, Collab: Leave session leaves", async () => {
    const { mock } = await renderWithMock(<CommandPalette />);
    act(() => byId(buildCommands(mock, []), "collab:join")!.run());
    expect(useCollabStore.getState().dialogOpen).toBe(true);

    await mock.send({ domain: "Collab", command: { type: "Join", server: "ws://relay:1", session: "jam", token: null, name: "Ada" } });
    act(() => useCollabStore.setState({ status: { type: "Online", session: "jam", site: "1" } }));
    const leave = byId(buildCommands(mock, []), "collab:leave")!;
    expect(leave.label).toBe("Collab: Leave session");
    let left = false;
    const off = mock.onEvent((e) => {
      if (e.type === "Collab" && e.event.type === "Session" && e.event.status.type === "Offline") left = true;
    });
    act(() => leave.run());
    await waitFor(() => expect(left).toBe(true));
    off();
  });

  it("are found by typing in the palette", async () => {
    await renderWithMock(<CommandPalette />);
    act(() => void fireEvent.keyDown(window, { key: "k", metaKey: true }));
    fireEvent.change(screen.getByRole("combobox", { name: "Search commands" }), { target: { value: "export project" } });
    const first = within(screen.getByRole("listbox", { name: "Commands" })).getAllByRole("option")[0]!;
    expect(first).toHaveTextContent("Export project…");
  });
});
