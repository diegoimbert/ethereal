// Toasts for peers' live chat messages while the chat section is closed (top right; a few at
// most, auto-dismissed; a click opens the chat), and the chat shortcut (Mod+Shift+M) while in
// a session. Mounted once by the presence bar.
import { useEffect } from "react";
import { Toast, ToastStack } from "@/kit";
import { useShellStore } from "@/app/shell/shellStore";
import { useProjectStore } from "@/state";
import { peerColor, useCollabStore } from "../store";
import { firstLine } from "./chatText";
import { chatOpen, clearToasts, dismissToast, focusChat, isChatShortcut, TOAST_MS, useChatUi } from "./chatStore";

export function ChatToasts() {
  const online = useCollabStore((s) => s.status.type === "Online");
  const toasts = useChatUi((s) => s.toasts);
  const chat = useProjectStore((s) => s.project?.chat);
  const open = useShellStore((s) => s.left.open && s.left.tab === "chat");

  // Opening the chat shows everything: no toasts then.
  useEffect(() => {
    if (open || !online) clearToasts();
  }, [open, online]);

  useEffect(() => {
    if (!online) return;
    const onKey = (e: KeyboardEvent) => {
      if (!isChatShortcut(e)) return;
      e.preventDefault();
      e.stopPropagation();
      focusChat();
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  }, [online]);

  const shown = toasts.flatMap((id) => {
    const m = chat?.[id];
    return m ? [m] : [];
  });
  if (!online || open || shown.length === 0) return null;
  return (
    <ToastStack label="Chat messages">
      {shown.map((m) => (
        <Toast
          key={m.id}
          title={m.author.name || "Someone"}
          accent={m.author.color !== null ? peerColor(m.author.color) : undefined}
          timeoutMs={TOAST_MS}
          onDismiss={() => dismissToast(m.id)}
          onClick={() => {
            if (!chatOpen()) focusChat();
          }}
        >
          {firstLine(m.text)}
        </Toast>
      ))}
    </ToastStack>
  );
}
