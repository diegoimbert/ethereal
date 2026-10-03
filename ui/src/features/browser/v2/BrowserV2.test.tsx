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
const browserCommands = (spy: Spy) =>
  sent(spy).flatMap((c) => (c.domain === "Browser" ? [c.command] : []));

async function setup() {
  const r = await renderWithMock(
    <>
      <Browser scope="library" />
      <ContextMenuHost />
    </>,
  );
  await screen.findByRole("list", { name: "Files" });
  const spy = vi.spyOn(r.mock, "send");
  return { ...r, spy };
}

const search = (text: string) => fireEvent.change(screen.getByRole("searchbox", { name: "Search files" }), { target: { value: text } });
const kindChip = (name: string) => within(screen.getByRole("group", { name: "Kinds" })).getByRole("button", { name });
const tab = (name: string) => within(screen.getByRole("tablist", { name: "Locations" })).getByRole("tab", { name });
async function results(name = "Search results") {
  return within(await screen.findByRole("list", { name }));
}

describe("Browser v2 (indexed)", () => {
  it("shows places (All, library, packs, user library, factory) and keeps folder browsing", async () => {
    await setup();
    expect(tab("Library")).toHaveAttribute("aria-selected", "true");
    for (const name of ["All", "Vocals", "User Library", "Factory Presets"]) expect(tab(name)).toBeInTheDocument();
    fireEvent.click(tab("Vocals"));
    expect(await within(screen.getByRole("list", { name: "Files" })).findByRole("button", { name: "Chop 1.wav" })).toBeInTheDocument();
    // Factory presets aren't folders: index results.
    fireEvent.click(tab("Factory Presets"));
    const presets = await results("Results");
    await waitFor(() => expect(presets.getAllByRole("button").length).toBeGreaterThan(1));
    expect(screen.getByText(/\d+ items/)).toBeInTheDocument();
  });

  it("searches the index with kind filters and relevance sort", async () => {
    const { spy } = await setup();
    search("loop");
    const list = await results();
    expect(await list.findByRole("button", { name: "Break 120.wav" })).toBeInTheDocument();
    expect(list.getByText("120 BPM")).toBeInTheDocument();
    expect(list.getByText("8.0 s")).toBeInTheDocument();
    const q = browserCommands(spy).findLast((c) => c.type === "Query");
    expect(q).toMatchObject({ query: { text: "loop", sort: "Relevance", roots: ["library"], kinds: [] } });
    expect(screen.getByRole("combobox", { name: "Sort" })).toHaveTextContent("Relevance");

    fireEvent.click(kindChip("MIDI"));
    expect(kindChip("MIDI")).toHaveAttribute("aria-pressed", "true");
    expect(await screen.findByText("No matches")).toBeInTheDocument();
    search("");
    const midi = await results("Results");
    expect(await midi.findByRole("button", { name: "Groove 1.mid" })).toBeInTheDocument();
    expect(midi.queryByRole("button", { name: "Kick.wav" })).toBeNull();
    expect(browserCommands(spy).findLast((c) => c.type === "Query")).toMatchObject({ query: { text: "", kinds: ["Midi"], sort: "Name" } });

    // Back to "All" kinds without text: the folder view again.
    fireEvent.click(kindChip("All"));
    expect(await screen.findByRole("list", { name: "Files" })).toBeInTheDocument();
  });

  it("leads into matching folders from a search", async () => {
    await setup();
    search("loops");
    fireEvent.click(await (await results()).findByRole("button", { name: "Loops" }));
    expect(await within(await screen.findByRole("list", { name: "Files" })).findByRole("button", { name: "Break 120.wav" })).toBeInTheDocument();
    expect(screen.getByRole("searchbox", { name: "Search files" })).toHaveValue("");
  });

  it("toggles favourites and filters by them", async () => {
    const { spy } = await setup();
    search("kick");
    const list = await results();
    fireEvent.click(await list.findByRole("button", { name: "Favourite Kick.wav" }));
    expect(await list.findByRole("button", { name: "Unfavourite Kick.wav" })).toHaveAttribute("aria-pressed", "true");
    expect(browserCommands(spy)).toContainEqual({ type: "SetFavourite", item: "library/Drums/Kick.wav", favourite: true });

    search("");
    fireEvent.click(screen.getByRole("button", { name: "Favourites only" }));
    const favs = await results("Results");
    expect(await favs.findByRole("button", { name: "Kick.wav" })).toBeInTheDocument();
    await waitFor(() => expect(favs.queryByRole("button", { name: "Snare.wav" })).toBeNull());
    fireEvent.click(favs.getByRole("button", { name: "Unfavourite Kick.wav" }));
    expect(await screen.findByText("No favourites")).toBeInTheDocument();
  });

  it("edits tags from the context menu and filters by a clicked tag", async () => {
    const { spy } = await setup();
    search("kick");
    const row = await (await results()).findByRole("button", { name: "Kick.wav" });
    fireEvent.contextMenu(row);
    fireEvent.click(await screen.findByRole("menuitem", { name: "Edit tags…" }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByRole("textbox", { name: "Tags" }), { target: { value: "Punchy, dark, punchy" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(browserCommands(spy)).toContainEqual({ type: "SetTags", item: "library/Drums/Kick.wav", tags: ["dark", "punchy"] });

    search("");
    fireEvent.click(tab("All"));
    fireEvent.click(kindChip("Samples"));
    fireEvent.click(await (await results("Results")).findByRole("button", { name: "punchy" }));
    expect(await screen.findByRole("button", { name: "Remove tag filter punchy" })).toBeInTheDocument();
    await waitFor(() => expect(browserCommands(spy).findLast((c) => c.type === "Query")).toMatchObject({ query: { tags: ["punchy"], roots: [] } }));
    const filtered = await results("Results");
    await waitFor(() => expect(filtered.queryByRole("button", { name: "Snare.wav" })).toBeNull());
    expect(filtered.getByRole("button", { name: "Kick.wav" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Remove tag filter punchy" }));
    expect(await (await results("Results")).findByRole("button", { name: "Snare.wav" })).toBeInTheDocument();
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

    fireEvent.click(screen.getByRole("button", { name: "Sync" }));
    expect(localStorage.getItem("eth-browser-sync")).toBe("0");
    fireEvent.keyDown(snare, { key: "Enter" });
    await waitFor(() => expect(snare).toHaveAttribute("aria-pressed", "true"));
    expect(browserCommands(spy)).toContainEqual({ type: "Preview", item: "library/Drums/Snare.wav", sync: false });
  });

  it("previews folder rows by item id too, and walks results with the arrow keys", async () => {
    const { spy } = await setup();
    const files = within(screen.getByRole("list", { name: "Files" }));
    fireEvent.click(await files.findByRole("button", { name: "Drums" }));
    fireEvent.click(await files.findByRole("button", { name: "Kick.wav" }));
    await waitFor(() => expect(browserCommands(spy)).toContainEqual({ type: "Preview", item: "library/Drums/Kick.wav", sync: true }));

    search("wav");
    const list = await results();
    const first = (await list.findAllByRole("button", { name: /\.wav$/ }))[0]!;
    first.focus();
    fireEvent.keyDown(first, { key: "ArrowDown" });
    await waitFor(() => expect(document.activeElement).not.toBe(first));
    const focused = document.activeElement as HTMLElement;
    await waitFor(() => expect(focused).toHaveAttribute("aria-pressed", "true"));
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

  it("rescans a place from its context menu", async () => {
    const { spy } = await setup();
    fireEvent.contextMenu(tab("Library"));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Rescan" }));
    await waitFor(() => expect(browserCommands(spy)).toContainEqual({ type: "Rescan", root: "library" }));
    // IndexChanged: the roots are listed again.
    await waitFor(() => expect(browserCommands(spy).filter((c) => c.type === "ListRoots").length).toBeGreaterThan(0));
  });

  it("imports a folder dropped on the places, then renames and removes it (base-136)", async () => {
    const { spy } = await setup();
    // No native folder dialog on this host: the folder is copied.
    expect(within(screen.getByRole("tablist", { name: "Locations" })).getByRole("button", { name: /Import folder…/ })).toBeInTheDocument();
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
    const places = screen.getByRole("tablist", { name: "Locations" });
    const dataTransfer = { types: ["Files"], files: [], items: [{ kind: "file", webkitGetAsEntry: () => folder, getAsFile: () => null }] };
    fireEvent.dragEnter(places, { dataTransfer });
    fireEvent.dragOver(places, { dataTransfer });
    fireEvent.drop(places, { dataTransfer });
    expect(await screen.findByRole("tab", { name: "Drums Kit" }, { timeout: 3000 })).toHaveAttribute("aria-selected", "true");
    expect(await screen.findByText(/Imported 2 files into “Drums Kit” · 1 file skipped/)).toBeInTheDocument();
    expect(browserCommands(spy)).toContainEqual({ type: "ImportFolder", name: "Drums Kit" });
    expect(browserCommands(spy).filter((c) => c.type === "ImportFile").map((c) => c.type === "ImportFile" && c.path)).toEqual(["Kick.wav", "Snare.wav"]);

    fireEvent.contextMenu(tab("Drums Kit"));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Rename…" }));
    fireEvent.change(await screen.findByRole("textbox", { name: "Folder name" }), { target: { value: "Kit A" } });
    fireEvent.click(screen.getByRole("button", { name: "Rename" }));
    expect(await screen.findByRole("tab", { name: "Kit A" })).toBeInTheDocument();

    fireEvent.contextMenu(tab("Kit A"));
    fireEvent.click(await screen.findByRole("menuitem", { name: "Remove folder" }));
    await waitFor(() => expect(screen.queryByRole("tab", { name: "Kit A" })).toBeNull());
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
    expect(await within(await screen.findByRole("list", { name: "Files" })).findByRole("button", { name: "Drums" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "Project media" })).toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "Kinds" })).toBeNull();
    mock.dispose();
  });
});
