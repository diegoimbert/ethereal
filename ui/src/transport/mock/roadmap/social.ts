/**
 * Mock of `Chat::*` and `PinnedNote::*` (docs/COLLAB.md §12). Owned by `collab-social`.
 *
 * - `PinnedNote::*`: ordinary undoable document edits (the `Marker::*` pattern in
 *   `clipEditing.ts`). The author is this site's session identity in a session (from
 *   `MockCollab`), else `author_name`.
 * - `Chat::Send`: `InvalidState` outside a session; otherwise the message is inserted with
 *   `seq` = next in order, the oldest pruned past `CHAT_MAX_MESSAGES`, **outside** the undo
 *   history (`MockHost.applyUntracked`). `MockCollab.simulateChat` posts a peer's message
 *   (patch + `CollabEvent::ChatReceived`).
 */

import type { Author, ChatCommand, ChatMessage, NotePosition, PinnedNoteCommand, ReplyValue } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";
import type { Tx } from "../tx";

/** `CHAT_MAX_MESSAGES`. */
export const CHAT_MAX_MESSAGES = 2000;
/** `CHAT_TEXT_MAX_CHARS` / `NOTE_TEXT_MAX_CHARS`. */
export const TEXT_MAX_CHARS = 2000;
/** `TEXT_MAX_BYTES`. */
export const TEXT_MAX_BYTES = 4096;
/** `MAX_PINNED_NOTES`. */
export const MAX_PINNED_NOTES = 500;

const UNIT: ReplyValue = { type: "Unit" };

/** What the chat needs from the session (`MockCollab`). */
export interface ChatSession {
  /** This site's identity in the session (`null`: not in one). */
  author(): Author | null;
  /** Apply chat ops outside the undo history and emit the patch. */
  applyUntracked(body: (tx: Tx) => void): void;
}

/** The engine's text rule (1..=2000 chars, ≤ 4096 bytes, not only whitespace). */
export function checkText(what: string, text: string): void {
  if (!text.trim()) fail("InvalidArgument", `${what} text is empty`);
  const n = [...text].length;
  if (n > TEXT_MAX_CHARS) fail("InvalidArgument", `${what} text is ${n} characters (max ${TEXT_MAX_CHARS})`);
  const bytes = new TextEncoder().encode(text).length;
  if (bytes > TEXT_MAX_BYTES) fail("InvalidArgument", `${what} text is ${bytes} bytes (max ${TEXT_MAX_BYTES})`);
}

/** Messages in chat order. */
export function chatOrdered(messages: ChatMessage[]): ChatMessage[] {
  return [...messages].sort((a, b) => a.seq - b.seq || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

/** Insert a message with the next `seq` and prune the oldest past the cap (one transaction). */
export function insertChat(tx: Tx, message: Omit<ChatMessage, "seq">): void {
  const all = chatOrdered(tx.all("ChatMessage"));
  const seq = all.reduce((m, x) => Math.max(m, x.seq), 0) + 1;
  const overflow = Math.max(0, all.length - (CHAT_MAX_MESSAGES - 1));
  tx.upsert("ChatMessage", { ...message, seq });
  for (const old of all.slice(0, overflow)) tx.remove("ChatMessage", old.id);
}

/** `Chat::*` (not a document command). */
export function chatCommand(c: ChatCommand, session?: ChatSession): ReplyValue {
  const author = session?.author() ?? null;
  if (!session || !author) return fail("InvalidState", "chat needs a collaboration session");
  checkText("the message", c.text);
  session.applyUntracked((tx) => {
    if (tx.get("ChatMessage", c.id)) return;
    insertChat(tx, { id: c.id, author, text: c.text, sent_at: Date.now() });
  });
  return UNIT;
}

/** The session identity for notes (set by `MockCollab` while in a session). */
let noteAuthor: () => Author | null = () => null;

/** `MockCollab` registers this site's session identity for `PinnedNote::Add`. */
export function setNoteAuthor(f: () => Author | null): void {
  noteAuthor = f;
}

/** `PinnedNote::*` (a document command). */
export function pinnedNoteCommand(ctx: ReducerContext, c: PinnedNoteCommand): void {
  const { tx } = ctx;
  const note = (id: string) => tx.get("PinnedNote", id) ?? fail("NotFound", `note ${id}`);
  const checkPosition = (p: NotePosition | null) => {
    if (!p) return;
    if (!(Number.isFinite(p.beats) && p.beats >= 0)) fail("InvalidArgument", "note position must be >= 0");
    if (p.editor && !(p.editor.beats >= 0 && p.editor.pitch >= 0 && p.editor.pitch <= 128)) fail("InvalidArgument", "note position out of range");
  };
  switch (c.type) {
    case "Add": {
      if (tx.get("PinnedNote", c.id)) return;
      checkText("note", c.text);
      checkPosition(c.position);
      if (tx.all("PinnedNote").length >= MAX_PINNED_NOTES) fail("InvalidArgument", `a project holds at most ${MAX_PINNED_NOTES} notes`);
      const author = noteAuthor() ?? { name: (c.author_name ?? "").trim().slice(0, 64), site: null, actor: null, color: null };
      tx.upsert("PinnedNote", { id: c.id, position: c.position, text: c.text, author, created_at: Date.now(), resolved: false });
      return;
    }
    case "Edit": {
      const n = note(c.id);
      if (c.text !== null) checkText("note", c.text);
      checkPosition(c.position);
      tx.upsert("PinnedNote", {
        ...n,
        text: c.text ?? n.text,
        position: c.position ?? n.position,
        resolved: c.resolved ?? n.resolved,
      });
      return;
    }
    case "Delete":
      for (const id of c.ids) tx.remove("PinnedNote", id);
      return;
  }
}
