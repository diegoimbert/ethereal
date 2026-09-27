import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { useProjectStore } from "@/state";
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

  it("imports on double-click and shows the file in project media", async () => {
    await renderWithMock(<Browser />);
    const list = await files();
    fireEvent.click(await list.findByRole("button", { name: "Drums" }));
    fireEvent.doubleClick(await list.findByRole("button", { name: "Kick.wav" }));
    await waitFor(() => expect(mediaNames()).toContain("Kick.wav"));
    expect((await screen.findByRole("status")).textContent).toBe("Imported Kick.wav");

    fireEvent.click(screen.getByRole("tab", { name: "Project media" }));
    const row = await list.findByRole("button", { name: /^\w+-Kick\.wav$/ });
    // Already in the project: no import button.
    expect(within(row).queryByRole("button", { name: /Import/ })).toBeNull();
  });

  it("imports with the + button and Enter", async () => {
    await renderWithMock(<Browser />);
    const list = await files();
    fireEvent.click(await list.findByRole("button", { name: "Vocals" }));
    fireEvent.click(await list.findByRole("button", { name: "Import Chop 1.wav" }));
    await waitFor(() => expect(mediaNames()).toContain("Chop 1.wav"));
    fireEvent.keyDown(list.getByRole("button", { name: "Phrase 2.wav" }), { key: "Enter" });
    await waitFor(() => expect(mediaNames()).toContain("Phrase 2.wav"));
  });

  it("previews and stops a preview", async () => {
    await renderWithMock(<Browser />);
    const list = await files();
    fireEvent.click(await list.findByRole("button", { name: "Drums" }));
    fireEvent.click(await list.findByRole("button", { name: "Preview Snare.wav" }));
    fireEvent.click(await list.findByRole("button", { name: "Stop preview of Snare.wav" }));
    expect(await list.findByRole("button", { name: "Preview Snare.wav" })).toBeInTheDocument();
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
    fireEvent.doubleClick(row);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/disposed/);
    fireEvent.click(alert);
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
