import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
import { PREVIEW_STEPS } from "@/transport/mock/roadmap/mediaPreview";
import { BROWSER_DRAG_MIME, readBrowserDrag } from "./dragPayload";
import { Browser } from "./index";

afterEach(resetStores);

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
  return within(await screen.findByRole("list", { name: "Files" }));
}

describe("Browser", () => {
  it("renders without an engine", () => {
    render(<Browser />);
    expect(screen.getByText("No engine connected")).toBeInTheDocument();
  });

  it("lists locations and navigates folders", async () => {
    await renderWithMock(<Browser />);
    const lib = await screen.findByRole("tab", { name: "Library" });
    expect(lib).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "Project media" })).toBeInTheDocument();

    const list = await files();
    fireEvent.click(await list.findByRole("button", { name: "Drums" }));
    fireEvent.click(await list.findByRole("button", { name: "Loops" }));
    expect(await list.findByRole("button", { name: "Break 120.wav" })).toBeInTheDocument();
    expect(within(screen.getByRole("navigation", { name: "Path" })).getByText("Loops")).toBeInTheDocument();

    fireEvent.click(list.getByRole("button", { name: "Parent folder" }));
    expect(await list.findByRole("button", { name: "Kick.wav" })).toBeInTheDocument();
    fireEvent.click(within(screen.getByRole("navigation", { name: "Path" })).getByText("Library"));
    expect(await list.findByRole("button", { name: "Synths" })).toBeInTheDocument();
    // Non-audio files are listed but not actionable.
    expect(list.getByText("Readme.txt")).toBeInTheDocument();
    expect(list.queryByRole("button", { name: "Readme.txt" })).toBeNull();
  });

  it("scopes to the library or the project's media (no location tabs for one root)", async () => {
    await renderWithMock(<Browser scope="project" />);
    await screen.findByRole("list", { name: "Files" });
    await waitFor(() => expect(screen.getByRole("searchbox", { name: "Search files" })).toHaveAttribute("placeholder", "Search Project media"));
    expect(screen.queryByRole("button", { name: "Drums" })).toBeNull();
  });

  it("searches recursively from the current folder and navigates to a result's folder", async () => {
    await renderWithMock(<Browser scope="library" />);
    const list = await files();
    await list.findByRole("button", { name: "Drums" });
    fireEvent.change(screen.getByRole("searchbox", { name: "Search files" }), { target: { value: "kick" } });
    const results = within(await screen.findByRole("list", { name: "Search results" }));
    const kick = await results.findByRole("button", { name: "Kick.wav" });
    expect(kick.textContent).toContain("Drums");
    expect(results.queryByRole("button", { name: "Snare.wav" })).toBeNull();

    fireEvent.change(screen.getByRole("searchbox", { name: "Search files" }), { target: { value: "zzz" } });
    expect(await screen.findByText("No matches")).toBeInTheDocument();

    fireEvent.change(screen.getByRole("searchbox", { name: "Search files" }), { target: { value: "loop" } });
    fireEvent.click(await within(await screen.findByRole("list", { name: "Search results" })).findByRole("button", { name: "Loops" }));
    expect(await (await files()).findByRole("button", { name: "Break 120.wav" })).toBeInTheDocument();
    expect(screen.getByRole("searchbox", { name: "Search files" })).toHaveValue("");
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

  it("walks the rows with the arrow keys, previewing audio files, and opens / leaves folders", async () => {
    await renderWithMock(<Browser />);
    const list = await files();
    const drums = await list.findByRole("button", { name: "Drums" });
    drums.focus();
    fireEvent.keyDown(drums, { key: "ArrowRight" });
    const parent = await list.findByRole("button", { name: "Parent folder" });
    await waitFor(() => expect(parent).toHaveFocus());
    const kick = await list.findByRole("button", { name: "Kick.wav" });
    const snare = list.getByRole("button", { name: "Snare.wav" });
    const rows = list.getAllByRole("button");
    const k = rows.indexOf(kick);
    // Walk down from the parent row to the kick: it plays; the next row replaces it.
    for (let i = 0; i < k; i++) fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    expect(kick).toHaveFocus();
    await waitFor(() => expect(kick).toHaveAttribute("aria-pressed", "true"));
    expect(rows[k + 1]).toBe(snare);
    fireEvent.keyDown(kick, { key: "ArrowDown" });
    expect(snare).toHaveFocus();
    await waitFor(() => expect(snare).toHaveAttribute("aria-pressed", "true"));
    expect(kick).toHaveAttribute("aria-pressed", "false");
    // Moving back onto a playing row keeps it playing (no toggle).
    fireEvent.click(kick);
    await waitFor(() => expect(kick).toHaveAttribute("aria-pressed", "true"));
    fireEvent.keyDown(kick, { key: "ArrowUp" });
    fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
    expect(kick).toHaveFocus();
    await waitFor(() => expect(kick).toHaveAttribute("aria-pressed", "true"));
    // Home / End clamp to the ends; ← goes up a folder.
    fireEvent.keyDown(kick, { key: "Home" });
    expect(parent).toHaveFocus();
    fireEvent.keyDown(parent, { key: "ArrowUp" });
    expect(parent).toHaveFocus();
    fireEvent.keyDown(parent, { key: "ArrowLeft" });
    const back = await list.findByRole("button", { name: "Drums" });
    await waitFor(() => expect(list.getAllByRole("button")[0]).toHaveFocus());
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
    await waitFor(() => expect(snare).toHaveAttribute("aria-pressed", "true"));
    expect(kick).toHaveAttribute("aria-pressed", "false");
    // Played to its end (PreviewEnded { Finished }): the row resets by itself.
    act(() => mock.tick(16 * (PREVIEW_STEPS + 2)));
    await waitFor(() => expect(snare).toHaveAttribute("aria-pressed", "false"));
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
    fireEvent.click(alert);
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
