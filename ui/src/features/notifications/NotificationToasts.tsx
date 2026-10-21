import { Button, Dialog, Toast } from "@/kit";
import { formatDateTime, useT } from "@/i18n";
import "./notifications.css";
import { toastLook, useNotices, type Notice } from "./store";

/**
 * The notices as kit toasts (auto-dismissed: Info after a few seconds, Warning/Error a little
 * later), the newest few; when older ones left the stack unseen, "see all messages" opens the
 * messages dialog. Render inside the app's one `ToastStack` (the collab chat toasts share it,
 * so they never overlap).
 */
export function NotificationToastItems() {
  const t = useT();
  const notices = useNotices((s) => s.notices);
  const hidden = useNotices((s) => s.hidden);
  const dismiss = useNotices((s) => s.dismiss);
  const openLog = useNotices((s) => s.openLog);
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
            action={n.action}
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
      {hidden > 0 && notices.length > 0 && (
        <button type="button" className="eth-toast-more" data-testid="notices-see-all" onClick={() => openLog(true)}>
          {t("shell.notice.see_all", { count: hidden + notices.length })}
        </button>
      )}
    </>
  );
}

function LogRow({ notice: n }: { notice: Notice }) {
  const look = toastLook(n.level);
  return (
    <li className="eth-notice-log__row" data-level={n.level} data-testid="notice-log-row">
      <span className="eth-notice-log__dot" style={{ background: look.accent }} aria-hidden />
      <div className="eth-notice-log__main">
        <span className="eth-notice-log__title">
          {n.title ?? look.title}
          <time className="eth-notice-log__time" dateTime={new Date(n.at).toISOString()}>
            {formatDateTime(n.at, { timeStyle: "short" })}
          </time>
        </span>
        <span className="eth-notice-log__text">{n.message}</span>
      </div>
      {n.action && (
        <Button size="sm" onClick={n.action.onSelect}>
          {n.action.label}
        </Button>
      )}
    </li>
  );
}

/** "messages": this session's notices, newest first (from "see all messages"). Mounted once by the app shell. */
export function NotificationLog() {
  const t = useT();
  const open = useNotices((s) => s.logOpen);
  const history = useNotices((s) => s.history);
  const openLog = useNotices((s) => s.openLog);
  const clearHistory = useNotices((s) => s.clearHistory);
  return (
    <Dialog
      open={open}
      onClose={() => openLog(false)}
      title={t("shell.notice.log_title")}
      className="eth-notice-log-dialog"
      testId="notice-log"
      footer={
        <>
          <Button disabled={history.length === 0} onClick={clearHistory}>
            {t("shell.notice.log_clear")}
          </Button>
          <Button tone="accent" onClick={() => openLog(false)}>
            {t("common.close")}
          </Button>
        </>
      }
    >
      {history.length === 0 ? (
        <p className="eth-notice-log__empty">{t("shell.notice.log_empty")}</p>
      ) : (
        <ul className="eth-notice-log">
          {[...history].reverse().map((n) => (
            <LogRow key={n.id} notice={n} />
          ))}
        </ul>
      )}
    </Dialog>
  );
}
