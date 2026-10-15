// Opening the AI chat: the palette's "Ask AI" and the shortcut (Mod+Shift+A) open its rail
// tab and focus the input.
import { useEffect } from "react";
import { create } from "zustand";
import { leftTabOpen, openLeftTab } from "@/state/shell";

export const AI_TAB = "ai" as const;

/** Bumped by `openAiChat()`: the panel focuses its input. */
export const useAiFocus = create<{ request: number }>()(() => ({ request: 0 }));

export function aiChatOpen(): boolean {
  return leftTabOpen(AI_TAB);
}

/** Open the AI chat (if needed) and focus its input. */
export function openAiChat(): void {
  useAiFocus.setState((s) => ({ request: s.request + 1 }));
  openLeftTab(AI_TAB);
}

/** `Mod+Shift+A` (not Alt; Mod+Shift+L is the arrangement clip-loop toggle). */
export function isAiShortcut(e: Pick<KeyboardEvent, "metaKey" | "ctrlKey" | "shiftKey" | "altKey" | "key" | "code">): boolean {
  return (e.metaKey || e.ctrlKey) && e.shiftKey && !e.altKey && (e.code === "KeyA" || e.key.toLowerCase() === "a");
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
