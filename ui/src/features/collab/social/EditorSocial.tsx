// collab-social in the piano roll (docs/COLLAB.md §12.2): the notes pinned on the open clip,
// in its content coordinates (the same mapping as presence-v2's `EditorPresence`). Render
// inside the note grid (it fills it).
import "./social.css";
import { useEffect, useMemo, useState, type RefObject } from "react";
import type { ClipId, NotePosition } from "@/generated";
import { useProjectStore } from "@/state";
import type { EditorCursorMapping } from "../presence/EditorPresence";
import { useCollabStore, useHideOthers } from "../store";
import { DraftDot, NoteDot } from "./notes/NoteDot";
import { notesOn, registerNoteSurface, useNotesUi, type NoteSurface } from "./notes/notesStore";

export interface EditorNotesProps {
  clip: ClipId;
  gridRef: RefObject<HTMLElement | null>;
  mapping: RefObject<EditorCursorMapping>;
  /** Changes whenever the mapping does (zoom, scroll, key height, folded rows). */
  layoutKey: string;
}

/** The note position under a client point of the grid. */
function editorPositionAt(
  clip: ClipId,
  grid: HTMLElement,
  mapping: EditorCursorMapping,
  clientX: number,
  clientY: number,
): NotePosition {
  const box = grid.getBoundingClientRect();
  const { beats, pitch } = mapping.fromScreen(clientX - box.left, clientY - box.top);
  return { beats: 0, track: null, y: 0, editor: { clip, beats: Math.max(0, beats), pitch: Math.min(128, Math.max(0, pitch)) } };
}

export function EditorNotes({ clip, gridRef, mapping, layoutKey }: EditorNotesProps) {
  const hide = useHideOthers();
  const surface: NoteSurface = useMemo(() => ({ kind: "editor", clip }), [clip]);
  useEffect(
    () =>
      registerNoteSurface(surface, (x, y) => {
        const grid = gridRef.current;
        return grid ? editorPositionAt(clip, grid, mapping.current, x, y) : null;
      }),
    [surface, clip, gridRef, mapping],
  );
  const all = useProjectStore((s) => s.project?.pinned_notes);
  const notes = useMemo(() => notesOn(all, surface), [all, surface]);
  const draft = useNotesUi((s) => (s.draft?.surface.kind === "editor" && s.draft.surface.clip === clip ? s.draft : null));
  const status = useCollabStore((s) => s.status);
  const me = status.type === "Online" ? status.site : null;
  // The grid updates its mapping in a layout effect (after ours): draw again once it has.
  const [, setTick] = useState(0);
  useEffect(() => {
    const frame = requestAnimationFrame(() => setTick((t) => t + 1));
    return () => cancelAnimationFrame(frame);
  }, [layoutKey]);
  if (hide || (notes.length === 0 && !draft)) return null;
  const at = (p: NotePosition) => (p.editor ? mapping.current?.toScreen(p.editor.beats, p.editor.pitch) : undefined);
  return (
    <div className="eth-notes-layer" data-testid="editor-notes">
      {notes.map((n) => {
        const p = at(n.position);
        return p && <NoteDot key={n.id} note={n} surface={surface} x={p.x} y={p.y} me={me} />;
      })}
      {draft &&
        (() => {
          const p = at(draft.position);
          return p && <DraftDot x={p.x} y={p.y} position={draft.position} color="var(--eth-color-accent)" />;
        })()}
    </div>
  );
}
