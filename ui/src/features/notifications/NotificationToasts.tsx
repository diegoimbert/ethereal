import { Toast } from "@/kit";
import { toastLook, useNotices } from "./store";

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
