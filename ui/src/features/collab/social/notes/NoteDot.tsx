// One pinned note on a surface: a small dot in the author's colour; a click opens its card
// (text collapsed to a few lines with "more", edit, resolve, "Discard note"), right-click
// the same actions, and dragging the dot moves the note (one undo gesture).
import clsx from "clsx";
import { useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import type { NotePosition, PinnedNote, SiteId } from "@/generated";
import { Button, openContextMenu, Popover, setDragCursor } from "@/kit";
import { cmd, nextGestureId, newId, useTransport, type EngineTransport } from "@/transport";
import { relativeTime } from "../chatText";
import { authorLabel, localUserName, NOTE_MAX_CHARS, noteColor, notePositionAt, useNotesUi, type NoteSurface } from "./notesStore";

/** Text longer than this (or with more lines) is collapsed behind "more". */
const COLLAPSE_CHARS = 180;
const COLLAPSE_LINES = 4;
const DRAG_THRESHOLD_PX = 3;

const report = (p: Promise<unknown>) => p.catch((e: unknown) => console.warn("[notes] command failed:", e));

const setOpen = (id: string | null) => useNotesUi.setState({ open: id });

function editNote(transport: EngineTransport, note: PinnedNote, change: { text?: string; resolved?: boolean }) {
  void report(
    transport.send(
      cmd("PinnedNote", { type: "Edit", id: note.id, text: change.text ?? null, position: null, resolved: change.resolved ?? null }),
    ),
  );
}

function discard(transport: EngineTransport, note: PinnedNote) {
  setOpen(null);
  void report(transport.send(cmd("PinnedNote", { type: "Delete", ids: [note.id] })));
}

export interface NoteDotProps {
  note: PinnedNote;
  surface: NoteSurface;
  /** Position in the layer (px). */
  x: number;
  y: number;
  me: SiteId | null;
}

export function NoteDot({ note, surface, x, y, me }: NoteDotProps) {
  const transport = useTransport();
  const open = useNotesUi((s) => s.open === note.id);
  const [editingState, setEditing] = useState(false);
  // Closing the card (click away, drag, discard) ends the edit.
  const editing = editingState && open;
  const dragged = useRef(false);

  const onPointerDown = (e: ReactPointerEvent<HTMLButtonElement>) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    dragged.current = false;
    const x0 = e.clientX;
    const y0 = e.clientY;
    let gesture: ReturnType<typeof nextGestureId> | null = null;
    let frame = 0;
    let last: { x: number; y: number } | null = null;
    let sent = "";
    const flush = () => {
      frame = 0;
      if (!last) return;
      const position = notePositionAt(surface, last.x, last.y);
      if (!position) return;
      const key = JSON.stringify(position);
      if (key === sent) return;
      sent = key;
      gesture ??= nextGestureId();
      void report(transport.send(cmd("PinnedNote", { type: "Edit", id: note.id, text: null, position, resolved: null }), { gesture }));
    };
    const move = (ev: PointerEvent) => {
      if (!dragged.current && Math.hypot(ev.clientX - x0, ev.clientY - y0) < DRAG_THRESHOLD_PX) return;
      if (!dragged.current) {
        dragged.current = true;
        setOpen(null);
        setDragCursor("grabbing");
      }
      last = { x: ev.clientX, y: ev.clientY };
      if (!frame) frame = requestAnimationFrame(flush);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      if (frame) cancelAnimationFrame(frame);
      if (dragged.current) {
        flush();
        setDragCursor(null);
      }
      if (gesture !== null) void report(transport.send(cmd("Edit", { type: "EndGesture", gesture })));
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };

  const name = authorLabel(note, me);
  const menu = (e: React.MouseEvent) =>
    openContextMenu(e, [
      {
        label: "Edit note",
        onSelect: () => {
          setOpen(note.id);
          setEditing(true);
        },
      },
      { label: note.resolved ? "Reopen note" : "Resolve note", onSelect: () => editNote(transport, note, { resolved: !note.resolved }) },
      "separator",
      { label: "Discard note", danger: true, onSelect: () => discard(transport, note) },
    ]);

  return (
    <div
      className={clsx("eth-note", note.resolved && "eth-note--resolved")}
      style={{ transform: `translate(${x}px, ${y}px)`, "--eth-note-color": noteColor(note) } as CSSProperties}
      data-testid="pinned-note"
      data-note={note.id}
      data-resolved={note.resolved || undefined}
    >
      <Popover
        open={open}
        onOpenChange={(o) => {
          setOpen(o ? note.id : null);
          if (!o) setEditing(false);
        }}
        aria-label={`Note by ${name}`}
        className="eth-note-card"
        trigger={(t) => (
          <button
            type="button"
            className="eth-note__dot"
            aria-label={`Note by ${name}: ${note.text}`}
            title={`${name}: ${note.text}`}
            aria-expanded={t["aria-expanded"]}
            aria-haspopup={t["aria-haspopup"]}
            onPointerDown={onPointerDown}
            onClick={() => {
              if (dragged.current) return;
              t.onClick();
            }}
            onContextMenu={menu}
          />
        )}
      >
        <NoteCard note={note} name={name} editing={editing} setEditing={setEditing} transport={transport} />
      </Popover>
    </div>
  );
}

function NoteCard({
  note,
  name,
  editing,
  setEditing,
  transport,
}: {
  note: PinnedNote;
  name: string;
  editing: boolean;
  setEditing: (v: boolean) => void;
  transport: EngineTransport;
}) {
  const [expanded, setExpanded] = useState(false);
  const [now] = useState(Date.now);
  const long = note.text.length > COLLAPSE_CHARS || note.text.split("\n").length > COLLAPSE_LINES;
  return (
    <div className="eth-note-card__inner" style={{ "--eth-note-color": noteColor(note) } as CSSProperties} data-testid="note-card">
      <div className="eth-note-card__meta">
        <span className="eth-note-card__swatch" aria-hidden />
        <span className="eth-note-card__author">{name}</span>
        <span className="eth-note-card__time">{relativeTime(note.created_at, now)}</span>
        {note.resolved && <span className="eth-note-card__badge">Resolved</span>}
      </div>
      {editing ? (
        <NoteEditor
          initial={note.text}
          submitLabel="Save"
          onCancel={() => setEditing(false)}
          onSubmit={(text) => {
            setEditing(false);
            if (text !== note.text) editNote(transport, note, { text });
          }}
        />
      ) : (
        <>
          <p className={clsx("eth-note-card__text", long && !expanded && "eth-note-card__text--collapsed")} data-testid="note-text">
            {note.text}
          </p>
          {long && (
            <button type="button" className="eth-note-card__more" onClick={() => setExpanded((v) => !v)}>
              {expanded ? "less" : "more"}
            </button>
          )}
          <div className="eth-note-card__actions">
            <Button size="sm" onClick={() => setEditing(true)}>
              Edit
            </Button>
            <Button size="sm" onClick={() => editNote(transport, note, { resolved: !note.resolved })}>
              {note.resolved ? "Reopen" : "Resolve"}
            </Button>
            <Button size="sm" tone="danger" onClick={() => discard(transport, note)}>
              Discard note
            </Button>
          </div>
        </>
      )}
    </div>
  );
}

/** Text area for a note: Enter saves, Shift+Enter new line, Escape cancels. */
function NoteEditor({
  initial,
  submitLabel,
  onSubmit,
  onCancel,
}: {
  initial: string;
  submitLabel: string;
  onSubmit: (text: string) => void;
  onCancel: () => void;
}) {
  const [text, setText] = useState(initial);
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
  }, []);
  const chars = [...text].length;
  const valid = text.trim().length > 0 && chars <= NOTE_MAX_CHARS;
  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      if (valid) onSubmit(text);
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onCancel();
    }
    e.stopPropagation();
  };
  return (
    <div className="eth-note-editor">
      <textarea
        ref={ref}
        className="eth-input eth-note-editor__input"
        aria-label="Note"
        placeholder="Leave a note for everyone"
        rows={3}
        value={text}
        aria-invalid={chars > NOTE_MAX_CHARS || undefined}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        data-testid="note-input"
      />
      <div className="eth-note-editor__actions">
        {chars > NOTE_MAX_CHARS - 200 && <span className="eth-chat__counter">{NOTE_MAX_CHARS - chars}</span>}
        <Button size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button size="sm" tone="accent" disabled={!valid} onClick={() => onSubmit(text)}>
          {submitLabel}
        </Button>
      </div>
    </div>
  );
}

/** The note being written: a dot with the composer open; sends `PinnedNote::Add` on save. */
export function DraftDot({ x, y, position, color }: { x: number; y: number; position: NotePosition; color: string }) {
  const transport = useTransport();
  const [error, setError] = useState<string | null>(null);
  const close = () => useNotesUi.setState({ draft: null });
  return (
    <div className="eth-note eth-note--draft" style={{ transform: `translate(${x}px, ${y}px)`, "--eth-note-color": color } as CSSProperties} data-testid="note-draft">
      <Popover
        open
        onOpenChange={(o) => !o && close()}
        aria-label="Leave a note"
        className="eth-note-card"
        trigger={() => <span className="eth-note__dot" aria-hidden />}
      >
        <div className="eth-note-card__inner">
          <NoteEditor
            initial=""
            submitLabel="Pin note"
            onCancel={close}
            onSubmit={(text) => {
              const id = newId();
              transport
                .send(cmd("PinnedNote", { type: "Add", id, position, text, author_name: localUserName() }))
                .then(close, (e: unknown) => setError(e instanceof Error ? e.message : String(e)));
            }}
          />
          {error && (
            <p className="eth-chat__error" role="alert">
              {error}
            </p>
          )}
        </div>
      </Popover>
    </div>
  );
}
