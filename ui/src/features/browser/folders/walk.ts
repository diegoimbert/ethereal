/**
 * Reading a folder of the user's computer in the browser (`base-136`), from every entry
 * point, into one shape (`PickedFolder`: the folder's name and its files with their path
 * relative to it):
 * - File System Access handles (`showDirectoryPicker()`, `DataTransferItem.getAsFileSystemHandle()`;
 *   Chromium): `walkHandle`;
 * - File and Directory Entries (`DataTransferItem.webkitGetAsEntry()`; every browser):
 *   `walkEntry`;
 * - `<input type=file webkitdirectory>` (Firefox / Safari picker): `filesFromInput`, from
 *   each file's `webkitRelativePath`.
 *
 * Hidden entries (`.DS_Store`, `.git/`…) are left out; sub-folders are walked to
 * `MAX_DEPTH`. The walkers take minimal structural types so tests pass fakes.
 */

/** A file of a picked folder. */
export interface FolderFile {
  /** Relative to the picked folder, `/`-separated (`Kicks/Kick 01.wav`). */
  path: string;
  file: File;
}

export interface PickedFolder {
  name: string;
  files: FolderFile[];
}

/** Deepest sub-folder level walked (as the engine's index). */
export const MAX_DEPTH = 16;

const hidden = (name: string) => name.startsWith(".");
const join = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);

// ---- File System Access ------------------------------------------------------------------

export interface FileHandleLike {
  kind: "file";
  name: string;
  getFile(): Promise<File>;
}

export interface DirectoryHandleLike {
  kind: "directory";
  name: string;
  values(): AsyncIterable<FileHandleLike | DirectoryHandleLike>;
}

/** Every file below `dir` (File System Access). */
export async function walkHandle(dir: DirectoryHandleLike, signal?: AbortSignal): Promise<PickedFolder> {
  const files: FolderFile[] = [];
  const visit = async (d: DirectoryHandleLike, at: string, depth: number) => {
    for await (const h of d.values()) {
      signal?.throwIfAborted();
      if (hidden(h.name)) continue;
      const path = join(at, h.name);
      if (h.kind === "directory") {
        if (depth < MAX_DEPTH) await visit(h, path, depth + 1);
      } else {
        files.push({ path, file: await h.getFile() });
      }
    }
  };
  await visit(dir, "", 1);
  return { name: dir.name, files };
}

// ---- File and Directory Entries ----------------------------------------------------------

export interface FileEntryLike {
  isFile: true;
  isDirectory: false;
  name: string;
  file(ok: (f: File) => void, err?: (e: unknown) => void): void;
}

export interface DirectoryReaderLike {
  readEntries(ok: (entries: EntryLike[]) => void, err?: (e: unknown) => void): void;
}

export interface DirectoryEntryLike {
  isFile: false;
  isDirectory: true;
  name: string;
  createReader(): DirectoryReaderLike;
}

export type EntryLike = FileEntryLike | DirectoryEntryLike;

/** All entries of a directory (`readEntries` returns them in batches until an empty one). */
async function readAll(dir: DirectoryEntryLike): Promise<EntryLike[]> {
  const reader = dir.createReader();
  const out: EntryLike[] = [];
  for (;;) {
    const batch = await new Promise<EntryLike[]>((ok, err) => reader.readEntries(ok, err));
    if (batch.length === 0) return out;
    out.push(...batch);
  }
}

/** Every file below `dir` (File and Directory Entries). */
export async function walkEntry(dir: DirectoryEntryLike, signal?: AbortSignal): Promise<PickedFolder> {
  const files: FolderFile[] = [];
  const visit = async (d: DirectoryEntryLike, at: string, depth: number) => {
    for (const e of await readAll(d)) {
      signal?.throwIfAborted();
      if (hidden(e.name)) continue;
      const path = join(at, e.name);
      if (e.isDirectory) {
        if (depth < MAX_DEPTH) await visit(e, path, depth + 1);
      } else {
        files.push({ path, file: await new Promise<File>((ok, err) => e.file(ok, err)) });
      }
    }
  };
  await visit(dir, "", 1);
  return { name: dir.name, files };
}

// ---- <input webkitdirectory> -------------------------------------------------------------

/**
 * The folder(s) of an `<input webkitdirectory>` selection (normally one), grouped by the
 * first segment of each file's `webkitRelativePath` (`Drums/Kicks/Kick.wav`).
 */
export function filesFromInput(files: ArrayLike<File>): PickedFolder[] {
  const folders = new Map<string, FolderFile[]>();
  for (const file of Array.from(files)) {
    const rel = (file.webkitRelativePath || file.name).split("/").filter(Boolean);
    const [top, ...rest] = rel.length > 1 ? rel : ["", ...rel];
    if (rest.length === 0 || rel.some(hidden) || rest.length > MAX_DEPTH) continue;
    const list = folders.get(top!) ?? [];
    list.push({ path: rest.join("/"), file });
    folders.set(top!, list);
  }
  return [...folders].map(([name, list]) => ({ name: name || "Imported", files: list }));
}

// ---- Drops ---------------------------------------------------------------------------------

/** Minimal `DataTransferItem` (tests pass plain objects). */
export interface DropItemLike {
  kind: string;
  getAsFile?(): File | null;
  getAsFileSystemHandle?(): Promise<FileHandleLike | DirectoryHandleLike | null>;
  webkitGetAsEntry?(): EntryLike | null;
}

/** What an OS drop carries: folders (read lazily) and loose files. */
export interface DroppedItems {
  /** One per dropped folder; reading starts when called. */
  folders: Array<(signal?: AbortSignal) => Promise<PickedFolder>>;
  files: File[];
}

/**
 * Split an OS drop into folders and loose files. Must run synchronously inside the `drop`
 * handler (the browser clears the items afterwards): it takes every item's entry (and, in
 * Chromium, starts its File System Access handle) right away. `null` when the drop holds
 * no folder (the caller handles plain files as before).
 */
export function droppedItems(items: ArrayLike<DropItemLike> | DataTransferItemList | null | undefined): Promise<DroppedItems> | null {
  // `DataTransferItemList`'s entries are the DOM's (structurally richer) `FileSystemEntry`s.
  const taken = (Array.from((items ?? []) as ArrayLike<unknown>) as DropItemLike[])
    .filter((i) => i.kind === "file")
    .map((i) => ({
      handle: i.getAsFileSystemHandle?.().catch(() => null) ?? null,
      entry: i.webkitGetAsEntry?.() ?? null,
      file: i.getAsFile?.() ?? null,
    }));
  if (!taken.some((t) => t.entry?.isDirectory || (!t.entry && t.handle))) return null;
  return (async () => {
    const out: DroppedItems = { folders: [], files: [] };
    for (const t of taken) {
      const handle = t.handle ? await t.handle : null;
      if (handle?.kind === "directory") out.folders.push((signal) => walkHandle(handle, signal));
      else if (t.entry?.isDirectory) {
        const entry = t.entry;
        out.folders.push((signal) => walkEntry(entry, signal));
      } else if (t.file) out.files.push(t.file);
    }
    return out;
  })();
}

// ---- Pickers -------------------------------------------------------------------------------

type DirectoryPicker = (opts?: { id?: string; mode?: "read" }) => Promise<DirectoryHandleLike>;

/** `window.showDirectoryPicker` where the browser has it (Chromium). */
export function directoryPicker(win: Window = window): DirectoryPicker | null {
  const f = (win as unknown as { showDirectoryPicker?: DirectoryPicker }).showDirectoryPicker;
  return typeof f === "function" ? f.bind(win) : null;
}

/** `<input type=file webkitdirectory>`: the picked folder(s), `[]` when dismissed. */
export function pickWithInput(doc: Document = document): Promise<PickedFolder[]> {
  return new Promise((resolve) => {
    const input = doc.createElement("input");
    input.type = "file";
    input.webkitdirectory = true;
    input.multiple = true;
    input.hidden = true;
    input.setAttribute("data-testid", "import-folder-input");
    let done = false;
    const finish = (folders: PickedFolder[]) => {
      if (done) return;
      done = true;
      input.remove();
      resolve(folders);
    };
    input.addEventListener("change", () => finish(filesFromInput(input.files ?? [])));
    input.addEventListener("cancel", () => finish([]));
    doc.body.appendChild(input);
    input.click();
  });
}

/**
 * "Import folder…": the File System Access picker where available, else the
 * `webkitdirectory` input. Resolves with a reader of the picked folder, or `null` when
 * dismissed.
 */
export async function pickFolder(win: Window = window): Promise<((signal?: AbortSignal) => Promise<PickedFolder>) | null> {
  const picker = directoryPicker(win);
  if (picker) {
    try {
      const handle = await picker({ id: "ethereal-import-folder", mode: "read" });
      return (signal) => walkHandle(handle, signal);
    } catch (e) {
      if (e instanceof DOMException && e.name === "AbortError") return null;
      throw e;
    }
  }
  const [folder] = await pickWithInput(win.document);
  return folder ? async () => folder : null;
}
