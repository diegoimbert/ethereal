// Pinned notes UI state (docs/COLLAB.md §12.2): the note being written ("Leave a note"),
// the note whose card is open, and how each surface maps a click to a `NotePosition` (the
// arranger and the piano roll register their mapping while mounted).
import { create } from "zustand";
import type { ClipId, NotePosition, PinnedNote, PinnedNoteId, SiteId } from "@/generated";
import type { ContextMenuEntry } from "@/kit";
import { peerColor, useCollabStore } from "../../store";

/** "You" for our own notes, else the author's name ("Someone" when empty). */
export function authorLabel(note: PinnedNote, me: SiteId | null): string {
  if (me !== null && note.author.site === me) return "You";
  return note.author.name || "Someone";
}

/** The author's snapshot colour (neutral outside a session). */
export function noteColor(note: PinnedNote): string {
  return note.author.color !== null ? peerColor(note.author.color) : "var(--eth-color-text-dim)";
}

/** Longest note (`NOTE_TEXT_MAX_CHARS`). */
export const NOTE_MAX_CHARS = 2000;
/** `MAX_PINNED_NOTES`. */
export const MAX_NOTES = 500;

export type NoteSurface = { kind: "arranger" } | { kind: "editor"; clip: ClipId };

export interface NoteDraft {
  surface: NoteSurface;
  position: NotePosition;
}

interface NotesUiState {
  draft: NoteDraft | null;
  /** The note whose card is open. */
  open: PinnedNoteId | null;
  /** The note being dragged: shown at `position` until the patch lands. */
  dragging: { id: PinnedNoteId; position: NotePosition } | null;
}

export const useNotesUi = create<NotesUiState>()(() => ({ draft: null, open: null, dragging: null }));

type Mapper = (clientX: number, clientY: number) => NotePosition | null;
const mappers = new Map<string, Mapper>();
const surfaceKey = (s: NoteSurface) => (s.kind === "arranger" ? "arranger" : `editor:${s.clip}`);

/** A surface registers how a client point maps to a note position (returns the unregister). */
export function registerNoteSurface(surface: NoteSurface, map: Mapper): () => void {
  const key = surfaceKey(surface);
  mappers.set(key, map);
  return () => {
    if (mappers.get(key) === map) mappers.delete(key);
  };
}

export function notePositionAt(surface: NoteSurface, clientX: number, clientY: number): NotePosition | null {
  return mappers.get(surfaceKey(surface))?.(clientX, clientY) ?? null;
}

/** Start writing a note at a client point of `surface`. */
export function startNote(surface: NoteSurface, clientX: number, clientY: number): void {
  const position = notePositionAt(surface, clientX, clientY);
  if (position) useNotesUi.setState({ draft: { surface, position }, open: null });
}

/**
 * The "Leave a note" context-menu entry at the right-click point (none while "Hide users and
 * notes" is on, or when the surface isn't mounted).
 */
export function leaveNoteEntries(surface: NoteSurface, e: { clientX: number; clientY: number }): ContextMenuEntry[] {
  if (useCollabStore.getState().hideOthers || !mappers.has(surfaceKey(surface))) return [];
  const { clientX, clientY } = e;
  return [{ label: "Leave a note", onSelect: () => startNote(surface, clientX, clientY) }];
}

/** "Leave a note" on the arranger's ruler row at `beats` (the ruler's own menu). */
export function rulerNoteEntries(beats: number): ContextMenuEntry[] {
  if (useCollabStore.getState().hideOthers || !mappers.has("arranger")) return [];
  const position: NotePosition = { beats: Math.max(0, beats), track: null, y: 0 };
  return [{ label: "Leave a note", onSelect: () => useNotesUi.setState({ draft: { surface: { kind: "arranger" }, position }, open: null }) }];
}

/** `entries` after a separator, when there are any. */
export function withSeparator(entries: ContextMenuEntry[]): ContextMenuEntry[] {
  return entries.length ? ["separator", ...entries] : [];
}

/** Notes shown on `surface`: arranger notes, or the piano-roll notes of that clip. */
export function notesOn(notes: Record<string, PinnedNote> | undefined, surface: NoteSurface): PinnedNote[] {
  return Object.values(notes ?? {})
    .filter((n) => (surface.kind === "arranger" ? !n.position.editor : n.position.editor?.clip === surface.clip))
    .sort((a, b) => a.created_at - b.created_at || (a.id < b.id ? -1 : 1));
}

/** Remembered collab dialog name (the author name outside a session). */
export function localUserName(): string | null {
  try {
    const v = JSON.parse(localStorage.getItem("eth-collab-join") ?? "{}") as { name?: string };
    return v.name?.trim() || null;
  } catch {
    return null;
  }
}
