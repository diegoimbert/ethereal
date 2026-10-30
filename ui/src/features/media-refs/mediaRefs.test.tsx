import { act, createEvent, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { Clip, Command, Event, MediaRef, Project, ReplyValue } from "@/generated";
import type { ContextMenuItem } from "@/kit";
import { useProjectStore } from "@/state";
import { type EngineTransport } from "@/transport";
import { createDemoProject } from "@/transport/mock";
import { MissingMediaStatus } from "./MediaRefsRoot";
import { mediaRefCommands, withMediaRefClipEntries } from "./menus";
import { RelinkDialog } from "./RelinkDialog";
import { applyMediaRefEvent, openRelink, refreshMissing, resetMediaRefs, useMediaRefs } from "./store";
import { BROWSER_DRAG_MIME } from "@/features/browser/dragPayload";
import { deliverPathDrop, resetHover } from "@/features/import/osDrop";
import { readSampleDrop, relinkableSource } from "./drop";

/** Records commands; the test pushes events and chooses replies. */
class FakeTransport {
  readonly kind = "mock" as const;
  sent: Command[] = [];
  reply: (c: Command) => ReplyValue = () => ({ type: "Unit" });
  private listeners = new Set<(e: Event) => void>();
  connect = () => Promise.reject(new Error("unused"));
  send = async (c: Command): Promise<ReplyValue> => {
    this.sent.push(c);
    return this.reply(c);
  };
  onEvent = (l: (e: Event) => void) => {
    this.listeners.add(l);
    return () => this.listeners.delete(l);
  };
  emit(e: Event) {
    for (const l of this.listeners) l(e);
  }
  subscribePlayhead = () => () => {};
  subscribeMeters = () => () => {};
  dispose = () => {};
}

/** A desktop transport (OS file and folder dialogs). */
class DesktopTransport extends FakeTransport {
  picked: string[] | null = ["/Users/ana/Archive/Kick.wav"];
  folder: string | null = "/Volumes/Backup";
  onPathDrop = () => () => {};
  pickAudioFiles = async () => this.picked;
  pickFolder = async () => this.folder;
}

const flush = () => act(() => new Promise((r) => setTimeout(r, 0)));

let project: Project;
let clip: Clip;
let media: MediaRef;

beforeEach(() => {
  const demo = createDemoProject();
  clip = Object.values(demo.clips).find((c) => c.content.type === "Audio")!;
  const id = clip.content.type === "Audio" ? clip.content.media : "";
  media = { ...demo.media[id]!, name: "Kick.wav", location: { type: "External", path: "/Users/ana/Samples/Kick.wav" } };
  project = { ...demo, media: { ...demo.media, [id]: media } };
  useProjectStore.setState({ project });
  resetMediaRefs();
});

afterEach(() => useProjectStore.setState({ project: null }));

describe("missing media state", () => {
  it("mirrors Missing / Resolved and resets on project load", async () => {
    applyMediaRefEvent({ type: "Media", event: { type: "Missing", media: media.id } });
    expect(useMediaRefs.getState().missing.has(media.id)).toBe(true);
    applyMediaRefEvent({ type: "MediaRef", event: { type: "Resolved", media: media.id } });
    expect(useMediaRefs.getState().missing.size).toBe(0);
    applyMediaRefEvent({ type: "Media", event: { type: "Missing", media: media.id } });
    applyMediaRefEvent({ type: "ProjectLoaded", project });
    expect(useMediaRefs.getState().missing.size).toBe(0);
    // A UI connecting later asks.
    const t = new FakeTransport();
    t.reply = () => ({ type: "MissingMedia", media: [media.id] });
    await refreshMissing(t as unknown as EngineTransport);
    expect([...useMediaRefs.getState().missing]).toEqual([media.id]);
  });

  it("adds Relink to the clip menu and a notice", () => {
    render(<MissingMediaStatus />);
    expect(screen.queryByTestId("missing-media-notice")).toBeNull();
    act(() => applyMediaRefEvent({ type: "Media", event: { type: "Missing", media: media.id } }));
    expect(screen.getByTestId("missing-media-notice")).toHaveTextContent("1 sample is missing");

    const menu = withMediaRefClipEntries([{ label: "rename", onSelect: () => {} }, "separator", { label: "delete", onSelect: () => {} }], clip);
    const labels = menu.map((e) => (e === "separator" ? "—" : e.label));
    expect(labels).toEqual(["rename", "—", "relink sample…", "—", "delete"]);
    act(() => (menu[2] as ContextMenuItem).onSelect());
    expect(useMediaRefs.getState().dialog).toEqual({ media: media.id });
    // The notice hides while the dialog is open.
    expect(screen.queryByTestId("missing-media-notice")).toBeNull();
  });

  it("offers palette entries only when they apply", () => {
    const t = new FakeTransport() as unknown as EngineTransport;
    expect(mediaRefCommands(t, project).filter((c) => c.id !== "media:manager").map((c) => c.label)).toEqual(["collect all and save"]);
    applyMediaRefEvent({ type: "Media", event: { type: "Missing", media: media.id } });
    expect(mediaRefCommands(t, project).filter((c) => c.id !== "media:manager").map((c) => c.label)).toEqual(["relink missing samples…", "collect all and save"]);
    const local = { ...project, media: { [media.id]: { ...media, location: { type: "Project" as const } } } };
    resetMediaRefs();
    // Only the media manager, which is always there.
    expect(mediaRefCommands(t, local).map((c) => c.id)).toEqual(["media:manager"]);
  });
});

describe("Relink dialog", () => {
  it("relinks with the OS file dialog on the desktop, then shows it found", async () => {
    const t = new DesktopTransport();
    applyMediaRefEvent({ type: "Media", event: { type: "Missing", media: media.id } });
    openRelink();
    render(<RelinkDialog transport={t as unknown as EngineTransport} />);
    const row = screen.getByTestId("relink-row");
    expect(row).toHaveTextContent("Kick.wav");
    expect(row).toHaveTextContent("/Users/ana/Samples/Kick.wav");
    expect(within(row).getByText("missing")).toBeTruthy();
    expect(screen.getByTestId("relink-summary")).toHaveTextContent("1 sample can’t be found");
    fireEvent.click(within(row).getByRole("button", { name: "locate…" }));
    await flush();
    expect(t.sent).toContainEqual({
      domain: "MediaRef",
      command: { type: "Relink", media: media.id, source: { type: "Path", path: "/Users/ana/Archive/Kick.wav" } },
    });
    act(() => t.emit({ type: "MediaRef", event: { type: "Resolved", media: media.id } }));
    act(() => applyMediaRefEvent({ type: "MediaRef", event: { type: "Resolved", media: media.id } }));
    expect(within(screen.getByTestId("relink-row")).getByText("found")).toBeTruthy();
    expect(screen.getByTestId("relink-summary")).toHaveTextContent("every sample is linked.");
  });

  it("searches the library and a folder, shows candidates and collects", async () => {
    const t = new DesktopTransport();
    applyMediaRefEvent({ type: "Media", event: { type: "Missing", media: media.id } });
    openRelink(media.id);
    render(<RelinkDialog transport={t as unknown as EngineTransport} />);
    fireEvent.click(screen.getByRole("button", { name: /search library/ }));
    await flush();
    expect(t.sent.at(-1)).toEqual({ domain: "MediaRef", command: { type: "Search", media: media.id } });
    // One search at a time.
    expect(screen.getByRole("button", { name: /search in folder/ })).toBeDisabled();
    act(() => applyMediaRefEvent({ type: "MediaRef", event: { type: "SearchProgress", scanned: 3, total: 3 } }));
    fireEvent.click(screen.getByRole("button", { name: /search in folder/ }));
    await flush();
    expect(t.sent.at(-1)).toEqual({ domain: "MediaRef", command: { type: "Search", media: media.id, folder: "/Volumes/Backup" } });
    act(() => {
      applyMediaRefEvent({ type: "MediaRef", event: { type: "SearchProgress", scanned: 12, total: 12 } });
      applyMediaRefEvent({
        type: "MediaRef",
        event: { type: "Candidates", media: media.id, candidates: [{ type: "Path", path: "/Volumes/Backup/Kick.wav" }] },
      });
    });
    expect(screen.getByTestId("relink-search-status")).toHaveTextContent("searched 12 folders");
    fireEvent.click(screen.getByRole("button", { name: "use" }));
    await flush();
    expect(t.sent.at(-1)).toEqual({
      domain: "MediaRef",
      command: { type: "Relink", media: media.id, source: { type: "Path", path: "/Volumes/Backup/Kick.wav" } },
    });
    fireEvent.click(screen.getByRole("button", { name: "collect all and save" }));
    await flush();
    expect(t.sent.at(-1)).toEqual({ domain: "MediaRef", command: { type: "CollectAll" } });
  });

  it("shows errors and hides the folder search without a folder dialog (web)", async () => {
    const t = new FakeTransport();
    t.reply = (c) => {
      if (c.domain === "MediaRef" && c.command.type === "Search") throw Object.assign(new Error("boom"), {});
      return { type: "Unit" };
    };
    applyMediaRefEvent({ type: "Media", event: { type: "Missing", media: media.id } });
    openRelink();
    render(<RelinkDialog transport={t as unknown as EngineTransport} />);
    expect(screen.queryByRole("button", { name: /search in folder/ })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /search library/ }));
    await flush();
    expect(screen.getByRole("alert")).toHaveTextContent("boom");
  });
});

describe("dropping a sample on a missing one", () => {
  const sample = { type: "Location" as const, location: { type: "Library" as const, id: "library" }, path: "Drums/Kick 2.wav" };
  const browserDrag = (source: object = sample) => {
    const data = JSON.stringify({ version: 1, kind: "media", source, name: "Kick 2.wav", file_kind: "Audio" });
    return { types: [BROWSER_DRAG_MIME, "text/plain"], getData: (f: string) => (f === BROWSER_DRAG_MIME ? data : ""), dropEffect: "none" };
  };
  const relinks = (t: FakeTransport) => t.sent.filter((c) => c.domain === "MediaRef" && c.command.type === "Relink").map((c) => c.command);

  afterEach(() => resetHover());

  it("highlights a missing row under a browser sample and relinks to it", async () => {
    const t = new FakeTransport();
    applyMediaRefEvent({ type: "Media", event: { type: "Missing", media: media.id } });
    openRelink();
    render(<RelinkDialog transport={t as unknown as EngineTransport} />);
    const row = screen.getByTestId("relink-row");
    fireEvent(row, createEvent.dragOver(row, { dataTransfer: browserDrag() }));
    expect(row.className).toContain("eth-media-refs__row--drop");
    fireEvent(row, createEvent.drop(row, { dataTransfer: browserDrag() }));
    await flush();
    expect(row.className).not.toContain("eth-media-refs__row--drop");
    expect(relinks(t)).toEqual([{ type: "Relink", media: media.id, source: sample }]);
  });

  it("relinks to an OS path the desktop shell delivers after the hover", async () => {
    const t = new DesktopTransport();
    applyMediaRefEvent({ type: "Media", event: { type: "Missing", media: media.id } });
    openRelink();
    render(<RelinkDialog transport={t as unknown as EngineTransport} />);
    const row = screen.getByTestId("relink-row");
    fireEvent(row, createEvent.dragOver(row, { dataTransfer: { types: ["Files"], dropEffect: "none" } }));
    expect(row.className).toContain("eth-media-refs__row--drop");
    act(() => void deliverPathDrop(["/Users/ana/Archive/notes.txt", "/Users/ana/Archive/Kick.wav"]));
    await flush();
    // The first audio file is used.
    expect(relinks(t)).toEqual([{ type: "Relink", media: media.id, source: { type: "Path", path: "/Users/ana/Archive/Kick.wav" } }]);
  });

  it("refuses MIDI files and relinks project media to their file", () => {
    const midi = JSON.stringify({ version: 1, kind: "media", source: sample, name: "a.mid", file_kind: "Midi" });
    expect(readSampleDrop({ types: [BROWSER_DRAG_MIME], getData: () => midi })).toBeNull();
    expect(relinkableSource({ type: "Project", media: media.id })).toEqual({ type: "Path", path: "/Users/ana/Samples/Kick.wav" });
    useProjectStore.setState({
      project: { ...project, media: { ...project.media, [media.id]: { ...media, location: { type: "Project" }, file: "media/k.wav" } } },
    });
    expect(relinkableSource({ type: "Project", media: media.id })).toEqual({ type: "Location", location: { type: "ProjectMedia" }, path: "k.wav" });
  });
});
