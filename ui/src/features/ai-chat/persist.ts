// The AI chat's conversation, kept per project on this machine (#717): localStorage
// `eth.ai.chat.<project id>`, never in the project file (it holds the model's replies and the
// tool results, which are the user's own business, and may name an API provider). What the
// panel shows and the model-side history are both kept, so a reload continues the same
// conversation. Too big to keep: the model-side history goes first (the next message starts
// a fresh model conversation), then the oldest messages.
import type { ProviderId } from "./providers";
import { isProviderId } from "./providers/catalog";

/** What the panel shows (`chatStore`'s `ChatItem`; kept loose here to avoid a cycle). */
export type SavedItem = { kind: string; id: string } & Record<string, unknown>;

export interface SavedChat {
  items: SavedItem[];
  /** The provider the model-side history is for (`null`: none kept). */
  provider: ProviderId | null;
  history: unknown[] | null;
}

const PREFIX = "eth.ai.chat.";
const VERSION = 1;
/** Largest saved conversation (characters of JSON), well under a browser's localStorage quota. */
export const MAX_SAVED_CHARS = 1_000_000;

export const chatStorageKey = (project: string) => `${PREFIX}${project}`;

export function loadChat(project: string): SavedChat | null {
  try {
    const raw = localStorage.getItem(chatStorageKey(project));
    if (!raw) return null;
    const v = JSON.parse(raw) as { version?: number; items?: unknown; provider?: unknown; history?: unknown };
    if (v.version !== VERSION || !Array.isArray(v.items)) return null;
    const items = v.items.filter(
      (i): i is SavedItem => !!i && typeof i === "object" && typeof (i as SavedItem).kind === "string" && typeof (i as SavedItem).id === "string",
    );
    const provider = isProviderId(v.provider) ? v.provider : null;
    return { items, provider, history: provider && Array.isArray(v.history) ? v.history : null };
  } catch {
    return null;
  }
}

/** Save (or, with nothing to keep, remove) `chat` for `project`. Never throws. */
export function saveChat(project: string, chat: SavedChat): void {
  if (chat.items.length === 0) return clearChat(project);
  const encode = (c: SavedChat) => JSON.stringify({ version: VERSION, ...c });
  let json = encode(chat);
  if (json.length > MAX_SAVED_CHARS) json = encode({ ...chat, provider: null, history: null });
  let items = chat.items;
  while (json.length > MAX_SAVED_CHARS && items.length > 1) {
    items = items.slice(Math.ceil(items.length / 4));
    json = encode({ items, provider: null, history: null });
  }
  try {
    localStorage.setItem(chatStorageKey(project), json);
  } catch {
    /* not kept (private mode, quota): the conversation lasts this session */
  }
}

export function clearChat(project: string): void {
  try {
    localStorage.removeItem(chatStorageKey(project));
  } catch {
    /* nothing kept */
  }
}
