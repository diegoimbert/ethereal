import { afterEach, describe, expect, it } from "vitest";
import type { MediaRef } from "@/generated";
import { useProjectStore } from "@/state";
import { isCommandFailed, MockTransport } from "@/transport";
import { formatSize, parentPath, pathSegments, sameLocation, sourceOf } from "./paths";
import {
  BROWSER_DRAG_MIME,
  hasBrowserDrag,
  parseBrowserDragPayload,
  readBrowserDrag,
  resolveDroppedMedia,
  writeBrowserDrag,
  type BrowserDragPayload,
} from "./dragPayload";

const library = { type: "Library" as const, id: "library" };
const kick: BrowserDragPayload = {
  version: 1,
  kind: "media",
  name: "Kick.wav",
  file_kind: "Audio",
  source: { type: "Location", location: library, path: "Drums/Kick.wav" },
};

function dataTransfer() {
  const data = new Map<string, string>();
  return {
    get types() {
      return [...data.keys()];
    },
    setData: (f: string, v: string) => void data.set(f, v),
    getData: (f: string) => data.get(f) ?? "",
    effectAllowed: "all",
  } as unknown as DataTransfer;
}

afterEach(() => useProjectStore.getState().reset());

describe("drag payload", () => {
  it("round-trips through a DataTransfer", () => {
    const dt = dataTransfer();
    expect(hasBrowserDrag(dt)).toBe(false);
    expect(readBrowserDrag(dt)).toBeNull();
    writeBrowserDrag(dt, kick);
    expect(hasBrowserDrag(dt)).toBe(true);
    expect(dt.effectAllowed).toBe("copy");
    expect(readBrowserDrag(dt)).toEqual(kick);
  });

  it("rejects malformed payloads", () => {
    expect(parseBrowserDragPayload(null)).toBeNull();
    expect(parseBrowserDragPayload({ ...kick, version: 2 })).toBeNull();
    expect(parseBrowserDragPayload({ ...kick, source: { type: "Location", path: 3 } })).toBeNull();
    expect(parseBrowserDragPayload({ ...kick, source: { type: "Upload", upload: "x" } })).toBeNull();
    expect(parseBrowserDragPayload({ ...kick, source: { type: "Project", media: "01H" } })).not.toBeNull();
    const dt = dataTransfer();
    dt.setData(BROWSER_DRAG_MIME, "{not json");
    expect(readBrowserDrag(dt)).toBeNull();
  });

  it("resolves dropped media: imports library files, reuses project media", async () => {
    const mock = new MockTransport({ timers: "manual", seed: 3 });
    mock.onEvent((e) => {
      if (e.type === "Patch") useProjectStore.getState().applyPatch(e.patch);
    });
    useProjectStore.getState().loadProject(await mock.connect());

    const media = await resolveDroppedMedia(mock, kick);
    expect(media.name).toBe("Kick.wav");
    expect(mock.snapshot().media[media.id]).toBeDefined();

    const again = await resolveDroppedMedia(mock, { ...kick, source: { type: "Project", media: media.id } });
    expect(again).toEqual(media);
    expect(Object.keys(mock.snapshot().media)).toHaveLength(Object.keys(useProjectStore.getState().project!.media).length);

    await expect(
      resolveDroppedMedia(mock, { ...kick, source: { type: "Location", location: library, path: "Nope.wav" } }),
    ).rejects.toSatisfy((e) => isCommandFailed(e, "NotFound"));
    mock.dispose();
  });
});

describe("path helpers", () => {
  it("splits and walks relative paths", () => {
    expect(pathSegments("Drums/Loops")).toEqual([
      { name: "Drums", path: "Drums" },
      { name: "Loops", path: "Drums/Loops" },
    ]);
    expect(pathSegments("")).toEqual([]);
    expect(parentPath("Drums/Loops")).toBe("Drums");
    expect(parentPath("Drums")).toBe("");
    expect(parentPath("")).toBe("");
  });

  it("compares locations", () => {
    expect(sameLocation(library, { type: "Library", id: "library" })).toBe(true);
    expect(sameLocation(library, { type: "Library", id: "other" })).toBe(false);
    expect(sameLocation({ type: "ProjectMedia" }, { type: "ProjectMedia" })).toBe(true);
    expect(sameLocation(library, { type: "ProjectMedia" })).toBe(false);
  });

  it("maps project media files to their MediaRef", () => {
    const m: MediaRef = { id: "M1", name: "a.wav", file: "media/M1-a.wav", sample_rate: 48000, channels: 2, frames: 10, hash: null, location: { type: "Project" } };
    expect(sourceOf({ type: "ProjectMedia" }, { path: "M1-a.wav" }, { M1: m })).toEqual({ type: "Project", media: "M1" });
    expect(sourceOf({ type: "ProjectMedia" }, { path: "x.wav" }, { M1: m })).toEqual({
      type: "Location",
      location: { type: "ProjectMedia" },
      path: "x.wav",
    });
    expect(sourceOf(library, { path: "M1-a.wav" }, { M1: m }).type).toBe("Location");
  });

  it("formats sizes", () => {
    expect(formatSize(512)).toBe("512 B");
    expect(formatSize(1536)).toBe("1.5 KB");
    expect(formatSize(1_300_000)).toBe("1.2 MB");
    expect(formatSize(50 * 1024 * 1024)).toBe("50 MB");
  });
});
