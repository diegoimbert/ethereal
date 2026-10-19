import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Command } from "@/generated";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { ContextMenuHost } from "@/kit";
import { useProjectStore } from "@/state";
import { CommandFailedError, MockTransport, TransportProvider } from "@/transport";
import { BROWSER_DRAG_MIME, readBrowserDrag } from "../dragPayload";
import { Browser } from "../index";

afterEach(() => {
  resetStores();
  localStorage.clear();
});

function fakeDataTransfer() {
  const data = new Map<string, string>();
  return {
    get types() {
      return [...data.keys()];
    },
    setData: (f: string, v: string) => void data.set(f, v),
    getData: (f: string) => data.get(f) ?? "",
    effectAllowed: "all" as DataTransfer["effectAllowed"],
  };
}

interface Spy {
  mock: { calls: unknown[][] };
}
const sent = (spy: Spy) => spy.mock.calls.map((c) => c[0] as Command);
const browserCommands = (spy: Spy) => sent(spy).flatMap((c) => (c.domain === "Browser" ? [c.command] : []));

async function setup() {
  const r = await renderWithMock(
    <>
      <Browser scope="library" />
      <ContextMenuHost />
    </>,
  );
  await screen.findByRole("tree", { name: "folders" });
  const spy = vi.spyOn(r.mock, "send");
  return { ...r, spy };
}

const search = (text: string) => fireEvent.change(screen.getByRole("searchbox", { name: "search files" }), { target: { value: text } });
const kindChip = (name: string) => within(screen.getByRole("group", { name: "kinds" })).getByRole("button", { name });
const tree = () => within(screen.getByRole("tree", { name: "folders" }));
const node = (name: string) => tree().findByRole("treeitem", { name });
/** Expands the folders on a path of the tree ("Library", "Drums", …). */
async function expand(...names: string[]) {
  for (const name of names) {
    const row = await node(name);
    if (row.getAttribute("aria-expanded") !== "true") fireEvent.click(row);
    await waitFor(() => expect(row).toHaveAttribute("aria-expanded", "true"));
  }
}
async function results(name = "search results") {
  return within(await screen.findByRole("list", { name }));
}

describe("Browser v2 (indexed)", () => {
  it("shows the browsable roots as a folder tree, presets as index results", async () => {
    await setup();
    // Packs show inside their root; roots without a folder (user library, factory presets) don't.
    expect((await tree().findAllByRole("treeitem")).map((r) => r.getAttribute("aria-label"))).toEqual(["Library"]);
    expect(kindChip("samples")).toHaveAttribute("aria-pressed", "true");
    expect(screen.queryByRole("button", { name: "all" })).toBeNull();
    await expand("Library", "Vocals");
    expect(await node("Chop 1.wav")).toBeInTheDocument();
    await expand("Drums", "Loops");
    expect(await node("Break 120.wav")).toHaveAttribute("aria-level", "4");
    // Collapsing hides the folder's rows; expanded folders are remembered.
    fireEvent.click(await node("Drums"));
    await waitFor(() => expect(tree().queryByRole("treeitem", { name: "Kick.wav" })).toBeNull());
    expect(JSON.parse(localStorage.getItem("eth-browser-tree-expanded")!)).toContain("library\u0000Vocals");

    fireEvent.click(kindChip("presets"));
    const presets = await results("results");
    await waitFor(() => expect(presets.getAllByRole("button").length).toBeGreaterThan(1));
    expect(screen.getByText(/\d+ items/)).toBeInTheDocument();
  });

  it("searches the index with kind filters and relevance sort; MIDI browses the tree for MIDI files", async () => {
    const { spy } = await setup();
    search("loop");
    const list = await results();
    expect(await list.findByRole("button", { name: "Break 120.wav" })).toBeInTheDocument();
    expect(list.getByText("120 bpm")).toBeInTheDocument();
    expect(list.getByText("8.0 s")).toBeInTheDocument();
    const q = browserCommands(spy).findLast((c) => c.type === "Query");
    expect(q).toMatchObject({ query: { text: "loop", sort: "Relevance", roots: [], kinds: ["Audio"] } });
    expect(screen.getByRole("combobox", { name: "sort" })).toHaveTextContent("relevance");

    fireEvent.click(kindChip("midi"));
    expect(kindChip("midi")).toHaveAttribute("aria-pressed", "true");
    expect(await screen.findByText("no matches")).toBeInTheDocument();
    search("");
    await expand("Library", "MIDI");
    expect(await node("Groove 1.mid")).toBeInTheDocument();
    await expand("Drums");
    expect(tree().queryByRole("treeitem", { name: "Kick.wav" })).toBeNull();
  });

  it("leads into matching folders from a search", async () => {
    await setup();
    search("loops");
    fireEvent.click(await (await results()).findByRole("button", { name: "Loops" }));
    const loops = await node("Loops");
    expect(loops).toHaveAttribute("aria-expanded", "true");
    expect(await node("Break 120.wav")).toBeInTheDocument();
    await waitFor(() => expect(document.activeElement).toBe(loops));
    expect(screen.getByRole("searchbox", { name: "search files" })).toHaveValue("");
  });

  it("toggles favourites and filters by them", async () => {
    const { spy } = await setup();
    search("kick");
    const list = await results();
    fireEvent.click(await list.findByRole("button", { name: "favourite Kick.wav" }));
    expect(await list.findByRole("button", { name: "unfavourite Kick.wav" })).toHaveAttribute("aria-pressed", "true");
    expect(browserCommands(spy)).toContainEqual({ type: "SetFavourite", item: "library/Drums/Kick.wav", favourite: true });

    search("");
    fireEvent.click(screen.getByRole("button", { name: "favourites only" }));
    const favs = await results("results");
    expect(await favs.findByRole("button", { name: "Kick.wav" })).toBeInTheDocument();
    await waitFor(() => expect(favs.queryByRole("button", { name: "Snare.wav" })).toBeNull());
    fireEvent.click(favs.getByRole("button", { name: "unfavourite Kick.wav" }));
    expect(await screen.findByText("no favourites")).toBeInTheDocument();
  });

  it("edits tags from the context menu and filters by a clicked tag", async () => {
    const { spy } = await setup();
    search("kick");
    const row = await (await results()).findByRole("button", { name: "Kick.wav" });
    fireEvent.contextMenu(row);
    fireEvent.click(await screen.findByRole("menuitem", { name: "edit tags…" }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByRole("textbox", { name: "tags" }), { target: { value: "Punchy, dark, punchy" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "save" }));
    expect(browserCommands(spy)).toContainEqual({ type: "SetTags", item: "library/Drums/Kick.wav", tags: ["dark", "punchy"] });

    fireEvent.click(await (await results()).findByRole("button", { name: "punchy" }));
    expect(await screen.findByRole("button", { name: "remove tag filter punchy" })).toBeInTheDocument();
    // A tag filter shows index results even without a search.
    search("");
    await waitFor(() => expect(browserCommands(spy).findLast((c) => c.type === "Query")).toMatchObject({ query: { text: "", tags: ["punchy"], roots: [] } }));
    const filtered = await results("results");
    expect(await filtered.findByRole("button", { name: "Kick.wav" })).toBeInTheDocument();
    expect(filtered.queryByRole("button", { name: "Snare.wav" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "remove tag filter punchy" }));
    expect(await screen.findByRole("tree", { name: "folders" })).toBeInTheDocument();
  });

  it("previews through Browser::Preview with the remembered Sync choice", async () => {
    const { spy } = await setup();
    search("snare");
    const snare = await (await results()).findByRole("button", { name: "Snare.wav" });
    fireEvent.click(snare);
    await waitFor(() => expect(snare).toHaveAttribute("aria-pressed", "true"));
    expect(browserCommands(spy)).toContainEqual({ type: "Preview", item: "library/Drums/Snare.wav", sync: true });
    fireEvent.click(snare);
    await waitFor(() => expect(snare).toHaveAttribute("aria-pressed", "false"));
    expect(sent(spy)).toContainEqual({ domain: "Media", command: { type: "StopPreview" } });

    fireEvent.click(screen.getByRole("button", { name: "sync" }));
    expect(localStorage.getItem("eth-browser-sync")).toBe("0");
    fireEvent.keyDown(snare, { key: "Enter" });
    await waitFor(() => expect(snare).toHaveAttribute("aria-pressed", "true"));
    expect(browserCommands(spy)).toContainEqual({ type: "Preview", item: "library/Drums/Snare.wav", sync: false });
  });

  it("previews tree rows by item id too, and walks the tree and results with the arrow keys", async () => {
    const { spy } = await setup();
    await expand("Library", "Drums");
    const kick = await node("Kick.wav");
    fireEvent.click(kick);
    await waitFor(() => expect(browserCommands(spy)).toContainEqual({ type: "Preview", item: "library/Drums/Kick.wav", sync: true }));
    expect(kick).toHaveClass("eth-browser__row--previewing");
    // ↓ moves to the next row and previews it; ← steps to the parent folder, ← again collapses it.
    fireEvent.keyDown(kick, { key: "ArrowDown" });
    const snare = await node("Snare.wav");
    await waitFor(() => expect(document.activeElement).toBe(snare));
    await waitFor(() => expect(snare).toHaveClass("eth-browser__row--previewing"));
    fireEvent.keyDown(snare, { key: "ArrowLeft" });
    const drums = await node("Drums");
    await waitFor(() => expect(document.activeElement).toBe(drums));
    fireEvent.keyDown(drums, { key: "ArrowLeft" });
    await waitFor(() => expect(drums).toHaveAttribute("aria-expanded", "false"));
    fireEvent.keyDown(drums, { key: "ArrowRight" });
    await waitFor(() => expect(drums).toHaveAttribute("aria-expanded", "true"));

    search("wav");
    const list = await results();
    const first = (await list.findAllByRole("button", { name: /\.wav$/ }))[0]!;
    first.focus();
    fireEvent.keyDown(first, { key: "ArrowDown" });
    await waitFor(() => expect(document.activeElement).not.toBe(first));
    const focused = document.activeElement as HTMLElement;
    await waitFor(() => expect(focused).toHaveAttribute("aria-pressed", "true"));
  });

  it("walks the tree and the results with the arrow keys from the search field, which keeps the focus", async () => {
    const { spy } = await setup();
    await expand("Library", "Drums");
    const field = screen.getByRole("searchbox", { name: "search files" });
    field.focus();
    // From the last clicked row (Drums): Loops, then Hat Closed.wav.
    fireEvent.keyDown(field, { key: "ArrowDown" });
    fireEvent.keyDown(field, { key: "ArrowDown" });
    expect(await node("Hat Closed.wav")).toHaveAttribute("aria-selected", "true");
    await waitFor(() => expect(browserCommands(spy)).toContainEqual({ type: "Preview", item: "library/Drums/Hat Closed.wav", sync: true }));
    fireEvent.keyDown(field, { key: "ArrowUp" });
    fireEvent.keyDown(field, { key: "ArrowRight" });
    expect(await node("Break 120.wav")).toBeInTheDocument();
    expect(document.activeElement).toBe(field);

    search("snare");
    await (await results()).findByRole("button", { name: "Snare.wav" });
    // The first result is active right away: Enter previews it (no arrow needed), again stops.
    const snare = (await results()).getByRole("button", { name: "Snare.wav" });
    await waitFor(() => expect(snare).toHaveClass("eth-browser__row--active"));
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() => expect(snare).toHaveAttribute("aria-pressed", "true"));
    fireEvent.keyDown(field, { key: "Enter" });
    await waitFor(() => expect(snare).toHaveAttribute("aria-pressed", "false"));
    expect(document.activeElement).toBe(field);
  });

  it("puts the media drag payload on audio results", async () => {
    await setup();
    search("break");
    const row = await (await results()).findByRole("button", { name: "Break 120.wav" });
    expect(row).toHaveAttribute("draggable", "true");
    const dt = fakeDataTransfer();
    fireEvent.dragStart(row, { dataTransfer: dt });
    expect(dt.types).toContain(BROWSER_DRAG_MIME);
    expect(readBrowserDrag(dt)).toEqual({
      version: 1,
      kind: "media",
      name: "Break 120.wav",
      file_kind: "Audio",
      source: { type: "Location", location: { type: "Library", id: "library" }, path: "Drums/Loops/Break 120.wav" },
    });
  });

  it("rescans a root from its context menu", async () => {
    const { spy } = await setup();
    fireEvent.contextMenu(await node("Library"));
    fireEvent.click(await screen.findByRole("menuitem", { name: "rescan" }));
    await waitFor(() => expect(browserCommands(spy)).toContainEqual({ type: "Rescan", root: "library" }));
    // IndexChanged: the roots are listed again.
    await waitFor(() => expect(browserCommands(spy).filter((c) => c.type === "ListRoots").length).toBeGreaterThan(0));
  });

  it("imports a folder dropped on the tree, then renames and removes it (base-136)", async () => {
    const { spy } = await setup();
    // No native folder dialog on this host: the folder is copied.
    expect(tree().getByRole("button", { name: /import folder…/ })).toBeInTheDocument();
    const wav = (name: string) => {
      const b = new Uint8Array(44 + 200);
      const v = new DataView(b.buffer);
      [..."RIFF"].forEach((c, i) => (b[i] = c.charCodeAt(0)));
      v.setUint32(4, 236, true);
      [..."WAVEfmt "].forEach((c, i) => (b[8 + i] = c.charCodeAt(0)));
      v.setUint32(16, 16, true);
      v.setUint16(20, 1, true);
      v.setUint16(22, 1, true);
      v.setUint32(24, 44100, true);
      v.setUint32(28, 88200, true);
      v.setUint16(32, 2, true);
      v.setUint16(34, 16, true);
      [..."data"].forEach((c, i) => (b[36 + i] = c.charCodeAt(0)));
      v.setUint32(40, 200, true);
      return new File([b], name);
    };
    const fileEntry = (f: File) => ({ isFile: true, isDirectory: false, name: f.name, file: (ok: (x: File) => void) => ok(f) });
    const entries = [fileEntry(wav("Kick.wav")), fileEntry(wav("Snare.wav")), fileEntry(new File(["x"], "notes.txt"))];
    const folder = {
      isFile: false,
      isDirectory: true,
      name: "Drums Kit",
      createReader: () => ({ readEntries: (ok: (e: unknown[]) => void) => ok(entries.splice(0)) }),
    };
    const places = screen.getByRole("tree", { name: "folders" }).parentElement!;
    const dataTransfer = { types: ["Files"], files: [], items: [{ kind: "file", webkitGetAsEntry: () => folder, getAsFile: () => null }] };
    fireEvent.dragEnter(places, { dataTransfer });
    fireEvent.dragOver(places, { dataTransfer });
    fireEvent.drop(places, { dataTransfer });
    const kit = await tree().findByRole("treeitem", { name: "Drums Kit" }, { timeout: 3000 });
    await waitFor(() => expect(kit).toHaveAttribute("aria-expanded", "true"));
    expect(await screen.findByText(/imported 2 files into “Drums Kit” · 1 file skipped/)).toBeInTheDocument();
    expect(browserCommands(spy)).toContainEqual({ type: "ImportFolder", name: "Drums Kit" });
    expect(
      browserCommands(spy)
        .filter((c) => c.type === "ImportFile")
        .map((c) => c.type === "ImportFile" && c.path),
    ).toEqual(["Kick.wav", "Snare.wav"]);

    fireEvent.contextMenu(kit);
    fireEvent.click(await screen.findByRole("menuitem", { name: "rename…" }));
    fireEvent.change(await screen.findByRole("textbox", { name: "folder name" }), { target: { value: "Kit A" } });
    fireEvent.click(screen.getByRole("button", { name: "rename" }));
    // #571: removing asks first (it deletes the imported copy); cancel keeps the folder.
    const removes = () => browserCommands(spy).filter((c) => c.type === "RemoveFolder");
    fireEvent.contextMenu(await node("Kit A"));
    fireEvent.click(await screen.findByRole("menuitem", { name: "remove folder…" }));
    const confirm = await screen.findByRole("dialog", { name: /remove “Kit A”/ });
    expect(within(confirm).getByTestId("remove-folder-warning")).toHaveTextContent(/deleted/);
    expect(removes()).toEqual([]);
    fireEvent.click(within(confirm).getByRole("button", { name: "cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: /remove “Kit A”/ })).toBeNull());
    expect(removes()).toEqual([]);
    expect(tree().getByRole("treeitem", { name: "Kit A" })).toBeInTheDocument();
    fireEvent.contextMenu(await node("Kit A"));
    fireEvent.click(await screen.findByRole("menuitem", { name: "remove folder…" }));
    fireEvent.click(within(await screen.findByRole("dialog", { name: /remove “Kit A”/ })).getByRole("button", { name: "remove" }));
    await waitFor(() => expect(tree().queryByRole("treeitem", { name: "Kit A" })).toBeNull());
    expect(removes()).toHaveLength(1);
  });

  it("falls back to the folder browser when the engine has no index", async () => {
    const mock = new MockTransport({ timers: "manual", seed: 7 });
    const send = mock.send.bind(mock);
    vi.spyOn(mock, "send").mockImplementation((c, opts) =>
      c.domain === "Browser" ? Promise.reject(new CommandFailedError({ code: "Unsupported", message: "no index" })) : send(c, opts),
    );
    render(
      <TransportProvider transport={mock}>
        <Browser />
      </TransportProvider>,
    );
    await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
    expect(await within(await screen.findByRole("list", { name: "files" })).findByRole("button", { name: "Drums" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "project media" })).toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "kinds" })).toBeNull();
    mock.dispose();
  });
});
