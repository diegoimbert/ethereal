/** Folder import (base-136): the walkers (fakes of both browser APIs), skip/quota rules, and the copy through the MockTransport. */
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { MockTransport } from "@/transport/mock/MockTransport";
import { cmd } from "@/transport";
import { importFolder, useFolderImports } from "./importFolder";
import { QUOTA_MARGIN_BYTES, neededBytes, planImport, quotaProblem } from "./plan";
import {
  droppedItems,
  filesFromInput,
  walkEntry,
  walkHandle,
  type DirectoryEntryLike,
  type DirectoryHandleLike,
  type EntryLike,
  type FileHandleLike,
  type PickedFolder,
} from "./walk";

/** A 16-bit PCM WAV of `frames` silent mono frames. */
function wav(frames = 100): Uint8Array<ArrayBuffer> {
  const data = frames * 2;
  const b = new Uint8Array(44 + data);
  const v = new DataView(b.buffer);
  const tag = (at: number, s: string) => [...s].forEach((c, i) => (b[at + i] = c.charCodeAt(0)));
  tag(0, "RIFF");
  v.setUint32(4, 36 + data, true);
  tag(8, "WAVE");
  tag(12, "fmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, 1, true);
  v.setUint32(24, 44100, true);
  v.setUint32(28, 88200, true);
  v.setUint16(32, 2, true);
  v.setUint16(34, 16, true);
  tag(36, "data");
  v.setUint32(40, data, true);
  return b;
}

const file = (name: string, bytes: Uint8Array<ArrayBuffer> | string = wav()) => new File([bytes], name);

/** A tree: names → file bytes, or a sub-tree. */
interface Tree {
  [name: string]: Tree | File;
}

const tree: Tree = {
  "Kick.wav": file("Kick.wav"),
  ".DS_Store": file(".DS_Store", "x"),
  "notes.txt": file("notes.txt", "hello"),
  Loops: { "Break 120.WAV": file("Break 120.WAV"), Deep: { "Hat.flac": file("Hat.flac") } },
  ".git": { "HEAD.wav": file("HEAD.wav") },
};

function handleOf(name: string, t: Tree): DirectoryHandleLike {
  return {
    kind: "directory",
    name,
    async *values() {
      for (const [n, v] of Object.entries(t)) {
        yield v instanceof File ? ({ kind: "file", name: n, getFile: async () => v } satisfies FileHandleLike) : handleOf(n, v);
      }
    },
  };
}

/** `readEntries` hands out at most two entries per call (browsers batch too). */
function entryOf(name: string, t: Tree): DirectoryEntryLike {
  return {
    isFile: false,
    isDirectory: true,
    name,
    createReader() {
      const all: EntryLike[] = Object.entries(t).map(([n, v]) =>
        v instanceof File ? { isFile: true, isDirectory: false, name: n, file: (ok: (f: File) => void) => ok(v) } : entryOf(n, v),
      );
      return { readEntries: (ok: (e: EntryLike[]) => void) => setTimeout(() => ok(all.splice(0, 2))) };
    },
  };
}

const paths = (f: PickedFolder) => f.files.map((x) => x.path).sort();
const WALKED = ["Kick.wav", "Loops/Break 120.WAV", "Loops/Deep/Hat.flac", "notes.txt"];

describe("folder walkers", () => {
  it("walks File System Access handles, skipping hidden entries", async () => {
    const f = await walkHandle(handleOf("Drums", tree));
    expect(f.name).toBe("Drums");
    expect(paths(f)).toEqual(WALKED);
  });

  it("walks directory entries, reading every batch", async () => {
    const f = await walkEntry(entryOf("Drums", tree));
    expect(f.name).toBe("Drums");
    expect(paths(f)).toEqual(WALKED);
  });

  it("stops when aborted", async () => {
    const ctl = new AbortController();
    ctl.abort(new DOMException("cancelled", "AbortError"));
    await expect(walkHandle(handleOf("Drums", tree), ctl.signal)).rejects.toMatchObject({ name: "AbortError" });
  });

  it("groups webkitdirectory input files by their top folder", () => {
    const withPath = (path: string) => {
      const f = file(path.split("/").pop()!);
      Object.defineProperty(f, "webkitRelativePath", { value: path });
      return f;
    };
    const folders = filesFromInput([withPath("Drums/Kick.wav"), withPath("Drums/Loops/Break.wav"), withPath("Drums/.hidden/x.wav")]);
    expect(folders).toHaveLength(1);
    expect(folders[0]!.name).toBe("Drums");
    expect(paths(folders[0]!)).toEqual(["Kick.wav", "Loops/Break.wav"]);
  });

  it("splits drops into folders and loose files, via handles or entries", async () => {
    const loose = file("Snare.wav");
    const viaEntries = droppedItems([
      { kind: "file", webkitGetAsEntry: () => entryOf("Drums", tree), getAsFile: () => null },
      { kind: "file", webkitGetAsEntry: () => ({ isFile: true, isDirectory: false, name: "Snare.wav", file: () => undefined }), getAsFile: () => loose },
      { kind: "string" },
    ]);
    expect(viaEntries).not.toBeNull();
    const a = await viaEntries!;
    expect(a.files).toEqual([loose]);
    expect(a.folders).toHaveLength(1);
    expect(paths(await a.folders[0]!())).toEqual(WALKED);

    // Chromium: the File System Access handle wins over the entry.
    const viaHandles = droppedItems([
      {
        kind: "file",
        getAsFileSystemHandle: async () => handleOf("Kit", { "A.wav": file("A.wav") }),
        webkitGetAsEntry: () => entryOf("Kit", {}),
      },
    ]);
    const b = await viaHandles!;
    expect(paths(await b.folders[0]!())).toEqual(["A.wav"]);

    // Files only: not a folder drop.
    expect(droppedItems([{ kind: "file", webkitGetAsEntry: () => ({ isFile: true, isDirectory: false, name: "x.wav", file: () => undefined }) }])).toBeNull();
    expect(droppedItems(null)).toBeNull();
  });
});

describe("import plan", () => {
  it("keeps supported audio and counts the rest", async () => {
    const empty = file("Empty.wav", new Uint8Array(0));
    const folder = await walkHandle(handleOf("Drums", { ...tree, "Empty.wav": empty, "Song.mid": file("Song.mid", "MThd") }));
    const plan = planImport(folder);
    expect(plan.files.map((f) => f.path)).toEqual(["Kick.wav", "Loops/Break 120.WAV", "Loops/Deep/Hat.flac"]);
    expect(plan.skipped).toBe(3);
    expect(plan.bytes).toBe(3 * wav().length);
    expect(plan.largest).toBe(wav().length);
  });

  it("refuses what does not fit in the quota", () => {
    const plan = planImport({ name: "Big", files: [{ path: "a.wav", file: file("a.wav", wav(5000)) }] });
    const need = neededBytes(plan);
    expect(need).toBe(2 * plan.bytes + QUOTA_MARGIN_BYTES);
    expect(quotaProblem(plan, { quota: need, usage: 0 })).toBeNull();
    expect(quotaProblem(plan, null)).toBeNull();
    expect(quotaProblem(plan, { usage: 5 })).toBeNull();
    expect(quotaProblem(plan, { quota: need + 10, usage: 11 })).toMatch(/“Big” needs .* of browser storage but only .* is free/);
  });
});

describe("importFolder (MockTransport)", () => {
  let mock: MockTransport;
  beforeEach(async () => {
    mock = new MockTransport({ timers: "manual", seed: 7 });
    await mock.connect();
    useFolderImports.setState({ jobs: [] });
  });
  afterEach(() => mock.dispose());

  const folder = (): PickedFolder => ({
    name: "Drums",
    files: [
      { path: "Kick.wav", file: file("Kick.wav") },
      { path: "Loops/Break.wav", file: file("Break.wav") },
      { path: "readme.md", file: file("readme.md", "#") },
    ],
  });

  it("creates a folder root, copies the audio, skips the rest and rescans", async () => {
    let shown: string | null = null;
    const job = await importFolder(mock, async () => folder(), { onRoot: (r) => (shown = r), estimate: async () => ({ quota: 1e12, usage: 0 }) });
    expect(job).toMatchObject({ state: "done", name: "Drums", total: 2, done: 2, skipped: 1, failed: 0 });
    expect(shown).toBe(job.root);
    const roots = await mock.send(cmd("Browser", { type: "ListRoots" }));
    expect(roots.type === "BrowserRoots" && roots.roots.find((r) => r.id === job.root)).toMatchObject({ kind: "Folder", name: "Drums", items: 2 });
    const page = await mock.send(
      cmd("Browser", {
        type: "Query",
        query: { text: "", kinds: [], tags: [], favourites_only: false, roots: [job.root!], folder: null, device: null, sort: "Name", offset: 0, limit: 50 },
      }),
    );
    expect(page.type === "BrowserPage" && page.page.items.map((i) => i.path)).toEqual(["Loops/Break.wav", "Kick.wav"]);
  });

  it("refuses a folder over quota before creating anything", async () => {
    const job = await importFolder(mock, async () => folder(), { estimate: async () => ({ quota: 1000, usage: 0 }) });
    expect(job.state).toBe("error");
    expect(job.error).toMatch(/browser storage/);
    expect(job.root).toBeNull();
    const roots = await mock.send(cmd("Browser", { type: "ListRoots" }));
    expect(roots.type === "BrowserRoots" && roots.roots.some((r) => r.kind === "Folder")).toBe(false);
  });

  it("says so when nothing is audio", async () => {
    const job = await importFolder(mock, async () => ({ name: "Docs", files: [{ path: "a.txt", file: file("a.txt", "x") }] }));
    expect(job).toMatchObject({ state: "error", skipped: 1 });
    expect(job.error).toMatch(/No supported audio files in “Docs”/);
  });

  it("cancels between files and keeps what was copied", async () => {
    const run = importFolder(mock, async () => folder(), {
      estimate: async () => null,
      onRoot: () => {
        const id = useFolderImports.getState().jobs[0]!.id;
        // Cancel once the first file is in.
        const off = useFolderImports.subscribe((s) => {
          if (s.jobs[0]?.done === 1) {
            off();
            useFolderImports.getState().cancel(id);
          }
        });
      },
    });
    const job = await run;
    expect(job).toMatchObject({ state: "cancelled", done: 1, total: 2 });
    const roots = await mock.send(cmd("Browser", { type: "ListRoots" }));
    expect(roots.type === "BrowserRoots" && roots.roots.find((r) => r.id === job.root)?.items).toBe(1);
  });
});
