import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { Project, Track } from "@/generated";
import { useArrangementUi } from "@/features/arrangement/uiStore";
import { ProjectMenu } from "@/features/project";
import { useProjectScreen } from "@/features/project/screenStore";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { ContextMenuHost, useContextMenuStore } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { findByName, matchesQuery, newProjectCommand, placementAfter, templateTrackEntries, tracksToSave, useTemplateDialog } from "./model";
import { TemplateDialogs } from "./TemplateDialogs";

const store = () => useProjectStore.getState();
const project = () => store().project!;

afterEach(() => {
  resetStores();
  useTemplateDialog.setState({ dialog: null });
  useProjectScreen.setState({ open: false, launchPending: false, naming: false });
});

const regular = (p: Project): Track[] =>
  Object.values(p.tracks)
    .filter((t) => t.parent === null && t.kind !== "Master" && t.kind !== "Return")
    .sort((a, b) => (a.order < b.order ? -1 : 1));

function select(label: string) {
  const item = useContextMenuStore.getState().menu!.items.find((i) => i !== "separator" && i.label === label);
  if (!item || item === "separator") throw new Error(`no ${label} item`);
  act(() => item.onSelect());
}

describe("templates model", () => {
  it("places after a track, picks the selection and matches", () => {
    const t = (id: string, order: string, parent: string | null = null) => ({ id, order, parent, kind: "Audio", name: id }) as Track;
    const p = { tracks: { a: t("a", "a0"), b: t("b", "a1"), c: t("c", "a0", "b"), m: { ...t("m", "a2"), kind: "Master" } } } as unknown as Project;
    expect(placementAfter(p, p.tracks.a!)).toEqual({ parent: null, before: "b" });
    expect(placementAfter(p, p.tracks.c!)).toEqual({ parent: "b", before: null });
    expect(tracksToSave("a", new Set(["a", "b", "m"]), p)).toEqual(["a", "b"]);
    expect(tracksToSave("c", new Set(["a"]), p)).toEqual(["c"]);
    const info = { id: "tracks/Vox", kind: "Tracks", name: "Vox", meta: { tags: ["vocal"], author: null, description: "Lead" }, factory: false, default: false, modified_ms: 0 } as const;
    expect(findByName([info], "Tracks", " vox ")).toBe(info);
    expect(findByName([info], "Tracks", "vox", "tracks/Vox")).toBeUndefined();
    expect(findByName([info], "Project", "vox")).toBeUndefined();
    expect(matchesQuery(info, "VOC")).toBe(true);
    expect(matchesQuery(info, "lead")).toBe(true);
    expect(matchesQuery(info, "drum")).toBe(false);
    expect(newProjectCommand("p", "Song", undefined)).toEqual(cmd("Template", { type: "NewProject", id: "p", name: "Song", template: null }));
    expect(newProjectCommand("p", "Song", null)).toEqual(cmd("Project", { type: "Create", id: "p", name: "Song" }));
    expect(newProjectCommand("p", "Song", "projects/Band")).toEqual(cmd("Template", { type: "NewProject", id: "p", name: "Song", template: "projects/Band" }));
  });
});

describe("TemplateDialogs", () => {
  it("saves a track as a template and inserts it after another track (one undo step)", async () => {
    const { mock } = await renderWithMock(
      <>
        <TemplateDialogs />
        <ContextMenuHost />
      </>,
    );
    const [first] = regular(project());
    expect(first).toBeDefined();
    const before = regular(project()).map((t) => t.id);

    // Save from the track menu.
    const entries = templateTrackEntries(project(), first!, new Set());
    act(() => useContextMenuStore.setState({ menu: { id: 1, x: 0, y: 0, items: entries } }));
    select("Save as Template…");
    const save = await screen.findByRole("dialog", { name: "Save track as a template" });
    expect(within(save).getByRole("textbox", { name: "Template name" })).toHaveValue(first!.name);
    fireEvent.change(within(save).getByRole("textbox", { name: "Template name" }), { target: { value: "My chain" } });
    fireEvent.change(within(save).getByRole("textbox", { name: "Template tags" }), { target: { value: "Lead, live" } });
    fireEvent.click(within(save).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Save track as a template" })).toBeNull());
    const listed = await mock.send(cmd("Template", { type: "List", kind: "Tracks" }));
    expect(listed.type === "Templates" && listed.templates.find((t) => !t.factory)).toMatchObject({ name: "My chain", meta: { tags: ["lead", "live"] } });

    // Saving again under the same name offers to replace.
    act(() => useTemplateDialog.getState().open({ type: "save-tracks", tracks: [first!.id], name: "my chain" }));
    const again = await screen.findByRole("dialog", { name: "Save track as a template" });
    expect(await within(again).findByRole("status")).toHaveTextContent("“My chain” exists: saving replaces it.");
    fireEvent.click(within(again).getByRole("button", { name: "Cancel" }));

    // Insert from the track menu: after `first`.
    act(() => useContextMenuStore.setState({ menu: { id: 2, x: 0, y: 0, items: templateTrackEntries(project(), first!, new Set()) } }));
    select("Insert Track Template…");
    const insert = await screen.findByRole("dialog", { name: "Insert track template" });
    const list = within(insert).getByRole("group", { name: "Track templates" });
    expect(await within(list).findByRole("button", { name: /^Vocal Chain/ })).toBeInTheDocument();
    fireEvent.change(within(insert).getByRole("textbox", { name: "Search templates" }), { target: { value: "chain" } });
    await act(async () => {
      fireEvent.click(await within(list).findByRole("button", { name: /^My chain/ }));
    });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Insert track template" })).toBeNull());
    const after = regular(project());
    expect(after).toHaveLength(before.length + 1);
    expect(after[1]!.id).not.toBe(before[1]);
    expect(after[0]!.id).toBe(first!.id);
    expect(useArrangementUi.getState().selectedTracks.has(after[1]!.id)).toBe(true);
    await act(async () => {
      await mock.send(cmd("Edit", { type: "Undo" }));
    });
    expect(regular(project()).map((t) => t.id)).toEqual(before);
  });

  it("renames and deletes user templates from the insert list", async () => {
    const { mock } = await renderWithMock(
      <>
        <TemplateDialogs />
        <ContextMenuHost />
      </>,
    );
    const [first] = regular(project());
    await mock.send(cmd("Template", { type: "SaveTracks", tracks: [first!.id], name: "Keys", meta: { tags: [], author: null, description: null }, overwrite: false }));
    act(() => useTemplateDialog.getState().open({ type: "insert", placement: { parent: null, before: null } }));
    const insert = await screen.findByRole("dialog", { name: "Insert track template" });
    fireEvent.click(await within(insert).findByRole("button", { name: "More actions for Keys" }));
    select("Rename…");
    const rename = await screen.findByRole("dialog", { name: "Rename “Keys”" });
    fireEvent.change(within(rename).getByRole("textbox", { name: "Template name" }), { target: { value: "Keys 2" } });
    fireEvent.click(within(rename).getByRole("button", { name: "Rename" }));
    await waitFor(async () => {
      const r = await mock.send(cmd("Template", { type: "List", kind: "Tracks" }));
      expect(r.type === "Templates" && r.templates.some((t) => t.id === "tracks/Keys 2")).toBe(true);
    });
    act(() => useTemplateDialog.getState().open({ type: "insert", placement: { parent: null, before: null } }));
    const again = await screen.findByRole("dialog", { name: "Insert track template" });
    fireEvent.click(await within(again).findByRole("button", { name: "More actions for Keys 2" }));
    select("Delete Template…");
    const del = await screen.findByRole("dialog", { name: "Delete “Keys 2”?" });
    fireEvent.click(within(del).getByRole("button", { name: "Delete" }));
    await waitFor(async () => {
      const r = await mock.send(cmd("Template", { type: "List", kind: "Tracks" }));
      expect(r.type === "Templates" && r.templates.every((t) => t.factory)).toBe(true);
    });
  });
});

describe("New project from a template", () => {
  it("offers the project templates, starts from the default, and saves the open project as one", async () => {
    const { mock } = await renderWithMock(
      <>
        <ProjectMenu />
        <ContextMenuHost />
      </>,
    );
    const demoTracks = Object.keys(project().tracks).length;
    fireEvent.click(screen.getByRole("button", { name: "Projects" }));
    const screenDialog = await screen.findByRole("dialog", { name: "Projects" });
    fireEvent.click(within(screenDialog).getByRole("button", { name: "Save as template…" }));
    const save = await screen.findByRole("dialog", { name: "Save project as a template" });
    fireEvent.change(within(save).getByRole("textbox", { name: "Template name" }), { target: { value: "Band" } });
    fireEvent.click(within(save).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Save project as a template" })).toBeNull());
    await mock.send(cmd("Template", { type: "SetDefault", template: "projects/Band" }));

    // The new-project form: the default template is picked.
    act(() => useProjectScreen.getState().showNew());
    const form = await screen.findByRole("dialog", { name: "Projects" });
    const picker = await within(form).findByRole("group", { name: "Project templates" });
    const band = await within(picker).findByRole("button", { name: /^Band/ });
    await waitFor(() => expect(band).toHaveAttribute("aria-current", "true"));
    expect(band).toHaveTextContent("Default");
    fireEvent.change(within(form).getByLabelText("New project name"), { target: { value: "Gig" } });
    fireEvent.click(within(form).getByRole("button", { name: "Create" }));
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toBe("Gig"));
    expect(Object.keys(project().tracks)).toHaveLength(demoTracks);

    // "Empty project" overrides the default.
    act(() => useProjectScreen.getState().showNew());
    const form2 = await screen.findByRole("dialog", { name: "Projects" });
    const picker2 = await within(form2).findByRole("group", { name: "Project templates" });
    fireEvent.click(await within(picker2).findByRole("button", { name: "Empty project" }));
    fireEvent.change(within(form2).getByLabelText("New project name"), { target: { value: "Blank" } });
    fireEvent.click(within(form2).getByRole("button", { name: "Create" }));
    await waitFor(() => expect(screen.getByTestId("project-name").textContent).toBe("Blank"));
    expect(Object.keys(project().tracks)).toHaveLength(1);
  });
});
