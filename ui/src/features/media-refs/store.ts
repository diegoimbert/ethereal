/**
 * Media references from the UI (`media-references`, CONTRACTS.md §12.9): which media of the
 * open project are missing, the Relink dialog's state, a running search and "Collect All".
 *
 * The engine reports missing media (`Media::Missing`) while it checks every media of a
 * project it opens, and `MediaRef::Resolved` when one is found again. `ListMissing` also
 * re-checks (a restored file resolves). The UI never reads files: it passes the paths the
 * desktop's OS dialogs return (`MediaSource::Path`) or uploads the bytes (web / remote).
 */

import { create } from "zustand";
import type { Event, MediaId, MediaSource } from "@/generated";
import { cmd, isCommandFailed, type EngineTransport } from "@/transport";

export interface SearchState {
  /** The media searched for (`null` = all missing). */
  media: MediaId | null;
  scanned: number;
  done: boolean;
}

interface MediaRefsState {
  /** Unresolved media of the open project. */
  missing: ReadonlySet<MediaId>;
  /** Possible files per media from the last search (empty = nothing found). */
  candidates: Readonly<Record<MediaId, MediaSource[]>>;
  search: SearchState | null;
  /** "Collect All" in progress. */
  collect: { done: number; total: number } | null;
  /** The Relink dialog: open, focused on one media (`null` = every missing one). */
  dialog: { media: MediaId | null } | null;
  /** Last failure shown in the dialog. */
  error: string | null;
}

const INITIAL: MediaRefsState = { missing: new Set(), candidates: {}, search: null, collect: null, dialog: null, error: null };

export const useMediaRefs = create<MediaRefsState>()(() => ({ ...INITIAL }));

/** Is `media` missing? (Reactive; `false` for no media.) */
export function useMediaMissing(media: MediaId | null | undefined): boolean {
  return useMediaRefs((s) => (media ? s.missing.has(media) : false));
}

export function errorText(e: unknown): string {
  if (isCommandFailed(e)) return e.error.message || e.error.code;
  return e instanceof Error ? e.message : String(e);
}

function setMissing(media: MediaId, missing: boolean): void {
  useMediaRefs.setState((s) => {
    if (s.missing.has(media) === missing) return s;
    const next = new Set(s.missing);
    if (missing) next.add(media);
    else next.delete(media);
    const { [media]: _, ...candidates } = s.candidates;
    return { missing: next, candidates: missing ? s.candidates : candidates };
  });
}

/** Mirror the engine's media-reference events. */
export function applyMediaRefEvent(e: Event): void {
  if (e.type === "ProjectLoaded") {
    useMediaRefs.setState({ ...INITIAL });
  } else if (e.type === "Media" && e.event.type === "Missing") {
    setMissing(e.event.media, true);
  } else if (e.type === "MediaRef") {
    const ev = e.event;
    switch (ev.type) {
      case "Resolved":
        setMissing(ev.media, false);
        break;
      case "Candidates":
        useMediaRefs.setState((s) => ({ candidates: { ...s.candidates, [ev.media]: ev.candidates } }));
        break;
      case "SearchProgress":
        useMediaRefs.setState((s) => ({
          search: { media: s.search?.media ?? null, scanned: ev.scanned, done: ev.total !== null && ev.total !== undefined },
        }));
        break;
      case "CollectProgress":
        useMediaRefs.setState({ collect: ev.done >= ev.total ? null : { done: ev.done, total: ev.total } });
        break;
    }
  } else if (e.type === "Patch") {
    // Media removed (undo of an import, a peer's delete): no longer missing.
    const gone = e.patch.changes.flatMap((c) => (c.type === "Remove" && c.key.type === "Media" ? [c.key.id] : []));
    for (const m of gone) setMissing(m, false);
  }
}

/** The current missing set from the engine (also makes it check them again). */
export async function refreshMissing(transport: EngineTransport): Promise<void> {
  try {
    const reply = await transport.send(cmd("MediaRef", { type: "ListMissing" }));
    if (reply.type === "MissingMedia") useMediaRefs.setState({ missing: new Set(reply.media) });
  } catch {
    // Unsupported on an engine without media references: nothing is missing.
  }
}

export function openRelink(media: MediaId | null = null): void {
  useMediaRefs.setState({ dialog: { media }, error: null });
}

export function closeRelink(): void {
  useMediaRefs.setState({ dialog: null, error: null });
}

async function run(body: () => Promise<unknown>): Promise<boolean> {
  useMediaRefs.setState({ error: null });
  try {
    await body();
    return true;
  } catch (e) {
    useMediaRefs.setState({ error: errorText(e) });
    return false;
  }
}

/** Point `media` at `source` (one undo step). */
export function relink(transport: EngineTransport, media: MediaId, source: MediaSource): Promise<boolean> {
  return run(() => transport.send(cmd("MediaRef", { type: "Relink", media, source })));
}

/** Search the library (and `folder`, desktop) for `media` (`null` = all missing). */
export function search(transport: EngineTransport, media: MediaId | null, folder?: string): Promise<boolean> {
  useMediaRefs.setState((s) => {
    const candidates = { ...s.candidates };
    if (media) delete candidates[media];
    return { search: { media, scanned: 0, done: false }, candidates: media ? candidates : {} };
  });
  return run(() => transport.send(cmd("MediaRef", { type: "Search", media, ...(folder ? { folder } : {}) }))).then((ok) => {
    if (!ok) useMediaRefs.setState({ search: null });
    return ok;
  });
}

/** Copy every external media into the project (one undo step), then save. */
export async function collectAll(transport: EngineTransport): Promise<boolean> {
  useMediaRefs.setState({ collect: { done: 0, total: 0 } });
  const ok = await run(() => transport.send(cmd("MediaRef", { type: "CollectAll" })));
  useMediaRefs.setState({ collect: null });
  return ok;
}

/** Test helper. */
export function resetMediaRefs(): void {
  useMediaRefs.setState({ ...INITIAL });
}
