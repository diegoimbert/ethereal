import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { PREVIEW_STEPS } from "@/transport/mock/roadmap/mediaPreview";
import { BROWSER_DRAG_MIME, readBrowserDrag } from "./dragPayload";
// The folder browser (engines without the library index; `Browser` picks the indexed one on the mock).
import { FolderBrowser as Browser } from "./index";

afterEach(resetStores);

/**
 * Waits on an engine round trip (a folder listing, then the focus effect it triggers). The
 * default 1 s flaked under a loaded machine (the mock replies are in-process but the worker
 * can be starved); the steps themselves are deterministic.
 */
const ROUND_TRIP = { timeout: 5_000 };

const mediaNames = () => Object.values(useProjectStore.getState().project!.media).map((m) => m.name);

/** Minimal DataTransfer stand-in (jsdom has none). */
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

async function files() {
  return within(await screen.findByRole("list", { name: "files" }));
}

describe("Browser", () => {
  it("renders without an engine", () => {
    render(<Browser />);
    expect(screen.getByText("no engine connected")).toBeInTheDocument();
  });

  it("lists locations and navigates folders", async () => {
    const { mock } = await renderWithMock(<Browser />);
    const lib = await screen.findByRole("tab", { name: "Library" });
    expect(lib).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "project media" })).toBeInTheDocument();

    const list = await files();
    fireEvent.click(await list.findByRole("button", { name: "Drums" }));
    fireEvent.click(await list.findByRole("button", { name: "Loops" }));
    expect(await list.findByRole("button", { name: "Break 120.wav" })).toBeInTheDocument();
    expect(within(screen.getByRole("navigation", { name: "path" })).getByText("Loops")).toBeInTheDocument();

    fireEvent.click(list.getByRole("button", { name: "parent folder" }));
    expect(await list.findByRole("button", { name: "Kick.wav" })).toBeInTheDocument();
    fireEvent.click(within(screen.getByRole("navigation", { name: "path" })).getByText("Library"));
    expect(await list.findByRole("button", { name: "Synths" })).toBeInTheDocument();
    // base-145: unsupported files are hidden by the default file filter...
    expect(list.queryByText("Readme.txt")).toBeNull();
    // ...a whitelist rule lists them (re-listed on `FileFilterChanged`), not actionable.
    const got = await mock.send(cmd("Browser", { type: "GetFileFilter" }));
    if (got.type !== "BrowserFileFilter") throw new Error(got.type);
    await act(() => mock.send(cmd("Browser", { type: "SetFileFilter", filter: { ...got.filter, whitelist: [...got.filter.whitelist, String.raw`\.txt$`] } })));
    expect(await list.findByText("Readme.txt", undefined, ROUND_TRIP)).toBeInTheDocument();
    expect(list.queryByRole("button", { name: "Readme.txt" })).toBeNull();
  });

  it("scopes to the library or the project's media (no location tabs for one root)", async () => {
    await renderWithMock(<Browser scope="project" />);
    await screen.findByRole("list", { name: "files" });
    await waitFor(() => expect(screen.getByRole("searchbox", { name: "search files" })).toHaveAttribute("placeholder", "search project media"));
    expect(screen.queryByRole("button", { name: "Drums" })).toBeNull();
  });

  it("searches recursively from the current folder and navigates to a result's folder", async () => {
    await renderWithMock(<Browser scope="library" />);
    const list = await files();
    await list.findByRole("button", { name: "Drums" });
    fireEvent.change(screen.getByRole("searchbox", { name: "search files" }), { target: { value: "kick" } });
    const results = within(await screen.findByRole("list", { name: "search results" }));
    const kick = await results.findByRole("button", { name: "Kick.wav" });
    expect(kick.textContent).toContain("Drums");
    expect(results.queryByRole("button", { name: "Snare.wav" })).toBeNull();

    fireEvent.change(screen.getByRole("searchbox", { name: "search files" }), { target: { value: "zzz" } });
    expect(await screen.findByText("no matches")).toBeInTheDocument();

    fireEvent.change(screen.getByRole("searchbox", { name: "search files" }), { target: { value: "loop" } });
    fireEvent.click(await within(await screen.findByRole("list", { name: "search results" })).findByRole("button", { name: "Loops" }));
    expect(await (await files()).findByRole("button", { name: "Break 120.wav" })).toBeInTheDocument();
    expect(screen.getByRole("searchbox", { name: "search files" })).toHaveValue("");
  });

  it("previews on click (Enter too) and stops on a second click, without importing", async () => {
    await renderWithMock(<Browser />);
    const list = await files();
    fireEvent.click(await list.findByRole("button", { name: "Drums" }));
    const snare = await list.findByRole("button", { name: "Snare.wav" });
    const before = mediaNames().length;
    fireEvent.click(snare);
    await waitFor(() => expect(snare).toHaveAttribute("aria-pressed", "true"));
    fireEvent.click(snare);
    await waitFor(() => expect(snare).toHaveAttribute("aria-pressed", "false"));
    fireEvent.keyDown(snare, { key: "Enter" });
    await waitFor(() => expect(snare).toHaveAttribute("aria-pressed", "true"));
    expect(mediaNames()).toHaveLength(before);
    expect(list.queryByRole("button", { name: /^(Preview|Import) / })).toBeNull();
  });

  it("walks the rows with the arrow keys, previewing audio files, and opens / leaves folders", { timeout: 20_000 }, async () => {
    await renderWithMock(<Browser />);
    const list = await files();
    const drums = await list.findByRole("button", { name: "Drums" }, ROUND_TRIP);
    drums.focus();
    fireEvent.keyDown(drums, { key: "ArrowRight" });
    const parent = await list.findByRole("button", { name: "parent folder" }, ROUND_TRIP);
    await waitFor(() => expect(parent).toHaveFocus(), ROUND_TRIP);
    const kick = await list.findByRole("button", { name: "Kick.wav" }, ROUND_TRIP);
    const snare = list.getByRole("button", { name: "Snare.wav" });
    const rows = list.getAllByRole("button");
    const k = rows.indexOf(kick);
    // Walk down from the parent row to the kick: it plays; the next row replaces it.
    for (let i = 0; i < k; i++) fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    expect(kick).toHaveFocus();
    await waitFor(() => expect(kick).toHaveAttribute("aria-pressed", "true"), ROUND_TRIP);
    expect(rows[k + 1]).toBe(snare);
    fireEvent.keyDown(kick, { key: "ArrowDown" });
    expect(snare).toHaveFocus();
    await waitFor(() => expect(snare).toHaveAttribute("aria-pressed", "true"), ROUND_TRIP);
    expect(kick).toHaveAttribute("aria-pressed", "false");
    // Moving back onto a playing row keeps it playing (no toggle).
    fireEvent.click(kick);
    await waitFor(() => expect(kick).toHaveAttribute("aria-pressed", "true"), ROUND_TRIP);
    fireEvent.keyDown(kick, { key: "ArrowUp" });
    fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    expect(kick).toHaveFocus();
    await waitFor(() => expect(kick).toHaveAttribute("aria-pressed", "true"), ROUND_TRIP);
    // Home / End clamp to the ends; ← goes up a folder.
    fireEvent.keyDown(kick, { key: "Home" });
    expect(parent).toHaveFocus();
    fireEvent.keyDown(parent, { key: "ArrowUp" });
    expect(parent).toHaveFocus();
    fireEvent.keyDown(parent, { key: "ArrowLeft" });
    const back = await list.findByRole("button", { name: "Drums" }, ROUND_TRIP);
    await waitFor(() => expect(list.getAllByRole("button")[0]).toHaveFocus(), ROUND_TRIP);
    expect(back).toBeInTheDocument();
  });

  it("follows replaced previews and resets the row when the preview ends", async () => {
    const { mock } = await renderWithMock(<Browser />);
    const list = await files();
    fireEvent.click(await list.findByRole("button", { name: "Drums" }));
    const kick = await list.findByRole("button", { name: "Kick.wav" });
    const snare = list.getByRole("button", { name: "Snare.wav" });
    fireEvent.click(kick);
    await waitFor(() => expect(kick).toHaveAttribute("aria-pressed", "true"));
    // Replaced: the old row resets, the new one is previewing.
    fireEvent.click(snare);
    await waitFor(() => expect(snare).toHaveAttribute("aria-pressed", "true"), ROUND_TRIP);
    expect(kick).toHaveAttribute("aria-pressed", "false");
    // Played to its end (PreviewEnded { Finished }): the row resets by itself.
    act(() => mock.tick(16 * (PREVIEW_STEPS + 2)));
    await waitFor(() => expect(snare).toHaveAttribute("aria-pressed", "false"));
  });

  it("a late Stopped of a file never resets a re-preview of the same file", async () => {
    const { mock } = await renderWithMock(<Browser />);
    const list = await files();
    fireEvent.click(await list.findByRole("button", { name: "Drums" }));
    const kick = await list.findByRole("button", { name: "Kick.wav" });
    fireEvent.click(kick);
    await waitFor(() => expect(kick).toHaveAttribute("aria-pressed", "true"));
    // Stop, then preview it again before the stop's `PreviewEnded { Stopped }` arrives.
    fireEvent.click(kick);
    expect(kick).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(kick);
    expect(kick).toHaveAttribute("aria-pressed", "true");
    // Both round trips done: the second preview is still the one shown.
    await new Promise((r) => setTimeout(r, 50));
    expect(kick).toHaveAttribute("aria-pressed", "true");
    // Its own natural end still resets it.
    act(() => mock.tick(16 * (PREVIEW_STEPS + 2)));
    await waitFor(() => expect(kick).toHaveAttribute("aria-pressed", "false"));
  });

  it("puts a media drag payload on audio files", async () => {
    await renderWithMock(<Browser />);
    const list = await files();
    fireEvent.click(await list.findByRole("button", { name: "Drums" }));
    const row = await list.findByRole("button", { name: "Kick.wav" });
    expect(row).toHaveAttribute("draggable", "true");
    const dt = fakeDataTransfer();
    fireEvent.dragStart(row, { dataTransfer: dt });
    expect(dt.types).toContain(BROWSER_DRAG_MIME);
    expect(dt.getData("text/plain")).toBe("Kick.wav");
    expect(readBrowserDrag(dt)).toEqual({
      version: 1,
      kind: "media",
      name: "Kick.wav",
      file_kind: "Audio",
      source: { type: "Location", location: { type: "Library", id: "library" }, path: "Drums/Kick.wav" },
    });
    expect(list.getByRole("button", { name: "Loops" })).not.toHaveAttribute("draggable", "true");
  });

  it("shows engine errors", async () => {
    const { mock } = await renderWithMock(<Browser />);
    const list = await files();
    fireEvent.click(await list.findByRole("button", { name: "Drums" }));
    const row = await list.findByRole("button", { name: "Kick.wav" });
    mock.dispose(); // every further command fails
    fireEvent.click(row);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/disposed/);
    fireEvent.click(within(alert).getByRole("button"));
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
