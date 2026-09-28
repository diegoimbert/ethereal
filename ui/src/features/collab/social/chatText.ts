// Chat text rules and formatting (docs/COLLAB.md §12.1), shared by the chat panel, its toasts
// and the notes' cards.
import type { ChatMessage } from "@/generated";

/** Longest message (`CHAT_TEXT_MAX_CHARS`, in characters). */
export const CHAT_MAX_CHARS = 2000;
/** Largest message in UTF-8 bytes (`TEXT_MAX_BYTES`). */
export const CHAT_MAX_BYTES = 4096;

/** Messages in chat order (`seq`, ties by id). */
export function chatOrdered(chat: Record<string, ChatMessage> | undefined): ChatMessage[] {
  return Object.values(chat ?? {}).sort((a, b) => a.seq - b.seq || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
}

/** Characters as the engine counts them (Unicode scalar values). */
export const charCount = (s: string) => [...s].length;
const byteCount = (s: string) => new TextEncoder().encode(s).length;

/** Why `text` can't be sent (`null`: it can). */
export function chatTextError(text: string): string | null {
  if (!text.trim()) return "empty";
  if (charCount(text) > CHAT_MAX_CHARS) return `Messages are at most ${CHAT_MAX_CHARS} characters`;
  if (byteCount(text) > CHAT_MAX_BYTES) return "This message is too long";
  return null;
}

/** "just now", "5 min ago", "14:05", "Mar 3, 14:05" (relative to `now`, local time). */
export function relativeTime(at: number, now: number): string {
  const s = Math.max(0, now - at) / 1000;
  if (s < 45) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  const d = new Date(at);
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (new Date(now).toDateString() === d.toDateString()) return time;
  return `${d.toLocaleDateString([], { month: "short", day: "numeric" })}, ${time}`;
}

/** First non-empty line of a message, for a toast. */
export function firstLine(text: string): string {
  return text.split("\n").find((l) => l.trim()) ?? text;
}
