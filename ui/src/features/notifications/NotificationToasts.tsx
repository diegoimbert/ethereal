import { useEffect } from "react";
import { Toast } from "@/kit";
import type { EngineTransport } from "@/transport";
import { onNotificationEvent, toastLook, useNotices } from "./store";

/** Turn the engine's `Event::Notification`s into notices while mounted. */
export function useEngineNotifications(transport: EngineTransport | null): void {
  useEffect(() => transport?.onEvent(onNotificationEvent), [transport]);
}

/**
 * The notices as kit toasts (Info auto-dismissed, Warning/Error kept until dismissed). Render
 * inside the app's one `ToastStack` (the collab chat toasts share it, so they never overlap).
 */
export function NotificationToastItems() {
  const notices = useNotices((s) => s.notices);
  const dismiss = useNotices((s) => s.dismiss);
  return (
    <>
      {notices.map((n) => {
        const look = toastLook(n.level);
        return (
          <Toast
            key={n.id}
            className="eth-notice"
            title={n.title ?? look.title}
            accent={look.accent}
            timeoutMs={look.timeoutMs}
            onDismiss={() => dismiss(n.id)}
            onClick={
              n.onClick
                ? () => {
                    dismiss(n.id);
                    n.onClick?.();
                  }
                : undefined
            }
          >
            {n.message}
          </Toast>
        );
      })}
    </>
  );
}

/** Whether there is anything to show (the shared stack renders only then). */
export function useHasNotices(): boolean {
  return useNotices((s) => s.notices.length > 0);
}
