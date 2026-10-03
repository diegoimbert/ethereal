// Opening the AI chat: the palette's "Ask AI" and the shortcut (Mod+Shift+L) open its rail
// tab and focus the input.
import { useEffect } from "react";
import { create } from "zustand";
import { useShellStore } from "@/app/shell/shellStore";

export const AI_TAB = "ai" as const;

/** Bumped by `openAiChat()`: the panel focuses its input. */
export const useAiFocus = create<{ request: number }>()(() => ({ request: 0 }));

export function aiChatOpen(): boolean {
  const { left } = useShellStore.getState();
  return left.open && left.tab === AI_TAB;
}

/** Open the AI chat (if needed) and focus its input. */
export function openAiChat(): void {
  useAiFocus.setState((s) => ({ request: s.request + 1 }));
  if (!aiChatOpen()) useShellStore.getState().toggleLeft(AI_TAB);
}

/** `Mod+Shift+L` (not Alt). */
export function isAiShortcut(e: Pick<KeyboardEvent, "metaKey" | "ctrlKey" | "shiftKey" | "altKey" | "key" | "code">): boolean {
  return (e.metaKey || e.ctrlKey) && e.shiftKey && !e.altKey && (e.code === "KeyL" || e.key.toLowerCase() === "l");
}

/** Window-level shortcut (mounted once, by the left rail). */
export function useAiChatShortcut(): void {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || !isAiShortcut(e)) return;
      e.preventDefault();
      openAiChat();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
