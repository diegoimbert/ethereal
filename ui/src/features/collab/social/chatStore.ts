// Chat UI state (docs/COLLAB.md §12.1): toasts for peers' live messages while the chat
// section is closed, and "focus the chat input" requests (Mod+Shift+M, the palette). The
// messages themselves are document state (`project.chat`, patched like any edit).
import { create } from "zustand";
import type { ChatMessageId } from "@/generated";
import { useShellStore } from "@/app/shell/shellStore";
import { matchesAction } from "@/features/keymap";

/** Toasts on screen at most (older ones are dropped). */
export const MAX_TOASTS = 3;
/** Auto-dismiss delay of a chat toast. */
export const TOAST_MS = 6000;

interface ChatUiState {
  /** Messages toasted now (oldest first). */
  toasts: ChatMessageId[];
  /** Bumped by `focusChat()`: the chat panel focuses its input. */
  focusRequest: number;
  /** Where focus was before `focusChat()` (Escape in the input returns there). */
  returnFocus: HTMLElement | null;
}

export const useChatUi = create<ChatUiState>()(() => ({ toasts: [], focusRequest: 0, returnFocus: null }));

/** The chat section is showing (no toasts then). */
export function chatOpen(): boolean {
  const { left } = useShellStore.getState();
  return left.open && left.tab === "chat";
}

/** `CollabEvent::ChatReceived`: toast the messages unless the chat is open. */
export function onChatReceived(ids: ReadonlyArray<ChatMessageId>): void {
  if (ids.length === 0 || chatOpen()) return;
  useChatUi.setState((s) => ({ toasts: [...s.toasts.filter((t) => !ids.includes(t)), ...ids].slice(-MAX_TOASTS) }));
}

export function dismissToast(id: ChatMessageId): void {
  useChatUi.setState((s) => ({ toasts: s.toasts.filter((t) => t !== id) }));
}

export function clearToasts(): void {
  useChatUi.setState({ toasts: [] });
}

/** Open the chat section (if needed) and focus its input. */
export function focusChat(): void {
  const active = document.activeElement;
  const inChat = active instanceof HTMLElement && active.closest("[data-chat-panel]");
  useChatUi.setState((s) => ({
    focusRequest: s.focusRequest + 1,
    returnFocus: !inChat && active instanceof HTMLElement && active !== document.body ? active : s.returnFocus,
    toasts: [],
  }));
  const shell = useShellStore.getState();
  if (!chatOpen()) shell.toggleLeft("chat");
}

/** Escape in the chat input: back to where the user was. */
export function returnFocus(): void {
  const el = useChatUi.getState().returnFocus;
  useChatUi.setState({ returnFocus: null });
  if (el?.isConnected) el.focus({ preventScroll: true });
  else (document.activeElement as HTMLElement | null)?.blur();
}

/** `Mod+Shift+M` (not Alt): the chat shortcut. */
export function isChatShortcut(e: Pick<KeyboardEvent, "metaKey" | "ctrlKey" | "shiftKey" | "altKey" | "key" | "code">): boolean {
  // keymap: `collab.focusChat` (default Mod+Shift+M).
  return matchesAction("collab.focusChat", e);
}
