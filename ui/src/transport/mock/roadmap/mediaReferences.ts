/**
 * Mock of `MediaRef::*` (v0.2, contracts-3). Owned by `media-references`: missing media,
 * relink, search and collect-all over the mock library (CONTRACTS.md §12.9).
 *
 * Library imports are referenced in place (`MediaLocation::External` at
 * `MOCK_LIBRARY_ROOT/<path>`, like the desktop engine). The "disk" is the mock library; a
 * test takes files offline with `setOffline(path)` (a moved or deleted sample) and reopens
 * the project: `Media::Missing` for every external media whose file is offline and that
 * has no project copy. OS paths and folders exist only on the desktop engine: `Relink` to a
 * `Path` and `Search { folder }` reply `Unsupported`.
 */

import type { Event, MediaId, MediaRef, MediaRefCommand, MediaSource, Project, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";
import { findLibraryFile, LIBRARY_FILES, LIBRARY_ID, normalize } from "../library";

/** Where the mock "engine machine" keeps its library (external paths). */
export const MOCK_LIBRARY_ROOT = "/Users/demo/Music/Ethereal Library";

/** The external path of a library file. */
export function libraryPath(rel: string): string {
  return `${MOCK_LIBRARY_ROOT}/${normalize(rel)}`;
}

/** Library-relative path of an external path in the library, if it is one. */
function relOf(path: string): string | null {
  const prefix = `${MOCK_LIBRARY_ROOT}/`;
  return path.startsWith(prefix) ? path.slice(prefix.length) : null;
}

const baseName = (p: string) => p.slice(p.lastIndexOf("/") + 1).toLowerCase();

export interface MediaRefsHost {
  project(): Project;
  emit(event: Event): void;
  /** Replace media (location/hash changes) as one undo step. */
  updateMedia(label: string, media: MediaRef[]): void;
  /** Save the current project (`Project::Save`). */
  save(): void;
  /** Mock content hash of a library file (what an import of it records). */
  libraryHash(rel: string): string;
}

export class MockMediaRefs {
  /** External paths whose file is gone. */
  private offline = new Set<string>();
  /** Media with a copy in the project folder (collected). */
  private copies = new Set<MediaId>();
  /** Media reported missing since the project opened. */
  private reported = new Set<MediaId>();

  constructor(private host: MediaRefsHost) {}

  /** Take a file of the mock library offline (or back). */
  setOffline(path: string, offline = true): void {
    if (offline) this.offline.add(path);
    else this.offline.delete(path);
  }

  private available(path: string): boolean {
    const rel = relOf(path);
    return !this.offline.has(path) && rel !== null && findLibraryFile(rel)?.kind === "Audio";
  }

  isMissing(m: MediaRef): boolean {
    return m.location.type === "External" && !this.copies.has(m.id) && !this.available(m.location.path);
  }

  private missing(): MediaRef[] {
    return Object.values(this.host.project().media).filter((m) => this.isMissing(m));
  }

  /** A project was opened: check every media (the engine does this in the background). */
  projectOpened(): void {
    this.reported.clear();
    this.check();
  }

  /** Report newly missing media and resolved ones. */
  private check(): void {
    const media = this.host.project().media;
    for (const m of Object.values(media)) {
      const missing = this.isMissing(m);
      if (missing && !this.reported.has(m.id)) {
        this.reported.add(m.id);
        this.host.emit({ type: "Media", event: { type: "Missing", media: m.id } });
      } else if (!missing && this.reported.delete(m.id)) {
        this.host.emit({ type: "MediaRef", event: { type: "Resolved", media: m.id } });
      }
    }
  }

  /** After any media change (relink, undo): report what changed. */
  mediaChanged(): void {
    this.check();
  }

  command(c: MediaRefCommand): ReplyValue {
    switch (c.type) {
      case "ListMissing": {
        const media = this.missing().map((m) => m.id);
        this.check();
        return { type: "MissingMedia", media };
      }
      case "Relink":
        this.relink(c.media, c.source);
        return { type: "Unit" };
      case "Search":
        if (c.folder) fail("Unsupported", "searching OS folders needs the desktop engine");
        this.search(c.media);
        return { type: "Unit" };
      case "CollectAll":
        this.collectAll();
        return { type: "Unit" };
    }
  }

  private media(id: MediaId): MediaRef {
    return this.host.project().media[id] ?? fail("NotFound", `media ${id}`);
  }

  private relink(id: MediaId, source: MediaSource): void {
    const m = this.media(id);
    if (source.type === "Path") fail("Unsupported", "relinking to OS files needs the desktop engine");
    if (source.type !== "Location" || source.location.type !== "Library") fail("InvalidArgument", "relink to a library file");
    if (source.location.id !== LIBRARY_ID) fail("NotFound", `location ${source.location.id}`);
    const rel = normalize(source.path);
    const f = findLibraryFile(rel) ?? fail("NotFound", `file ${source.path}`);
    if (f.kind !== "Audio") fail("Decode", `${source.path} is not an audio file`);
    const path = libraryPath(rel);
    if (this.offline.has(path)) fail("NotFound", `file ${source.path}`);
    if (f.sample_rate !== m.sample_rate || f.channels !== m.channels) {
      fail("InvalidArgument", `"${m.name}" is ${m.sample_rate} Hz, ${m.channels} ch; the file is ${f.sample_rate} Hz, ${f.channels} ch`);
    }
    const hash = this.host.libraryHash(rel);
    this.host.updateMedia("Relink", [{ ...m, location: { type: "External", path }, hash }]);
    if (hash !== m.hash) {
      this.host.emit({ type: "Notification", level: "Warning", message: `"${m.name}" was relinked to a file with different content` });
    }
  }

  private search(only: MediaId | null): void {
    const targets = only ? [this.media(only)] : this.missing();
    const files = LIBRARY_FILES.filter((f) => f.kind === "Audio" && !this.offline.has(libraryPath(f.path)));
    const relinked: MediaRef[] = [];
    const candidates: [MediaId, MediaSource[]][] = [];
    for (const m of targets) {
      const names = new Set([baseName(m.name), ...(m.location.type === "External" ? [baseName(m.location.path)] : [])]);
      const byName = files.filter((f) => names.has(baseName(f.path)));
      const same = byName.find((f) => this.host.libraryHash(f.path) === m.hash);
      if (same) relinked.push({ ...m, location: { type: "External", path: libraryPath(same.path) } });
      else candidates.push([m.id, byName.map((f) => ({ type: "Location", location: { type: "Library", id: LIBRARY_ID }, path: f.path }))]);
    }
    if (relinked.length > 0) this.host.updateMedia("Relink", relinked);
    for (const [media, c] of candidates) this.host.emit({ type: "MediaRef", event: { type: "Candidates", media, candidates: c } });
    const scanned = new Set(LIBRARY_FILES.map((f) => f.path.split("/").slice(0, -1).join("/"))).size;
    this.host.emit({ type: "MediaRef", event: { type: "SearchProgress", scanned, total: scanned } });
  }

  private collectAll(): void {
    const external = Object.values(this.host.project().media).filter((m) => m.location.type === "External");
    const collected: MediaRef[] = [];
    let skipped = 0;
    external.forEach((m, i) => {
      if (this.isMissing(m)) skipped++;
      else {
        this.copies.add(m.id);
        collected.push({ ...m, location: { type: "Project" } });
      }
      this.host.emit({ type: "MediaRef", event: { type: "CollectProgress", done: i + 1, total: external.length } });
    });
    if (collected.length > 0) this.host.updateMedia("Collect All", collected);
    if (skipped > 0) {
      this.host.emit({ type: "Notification", level: "Warning", message: `${skipped} missing file(s) could not be collected` });
    }
    this.host.save();
  }
}
