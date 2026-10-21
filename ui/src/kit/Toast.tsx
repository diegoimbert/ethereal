import "./toast.css";
import clsx from "clsx";
import { X } from "lucide-react";
import { useEffect, useRef, type CSSProperties, type ReactNode } from "react";
import { useT } from "@/i18n";

export interface ToastProps {
  /** First line (e.g. who). */
  title: ReactNode;
  /** Body text, clamped to a few lines. */
  children?: ReactNode;
  /** Accent (a CSS color, e.g. a peer colour: data, like track colours). Default: the accent token. */
  accent?: string;
  /** Click on the toast body (e.g. open what it is about). */
  onClick?: () => void;
  /** A button beside the text (e.g. "resolve"); choosing it dismisses the toast. */
  action?: { label: string; onSelect: () => void };
  onDismiss?: () => void;
  /** Auto-dismiss after this many ms (paused while hovered; a bar shows the time left); omit to keep it. */
  timeoutMs?: number;
  className?: string;
}

/** A short notification card. Stack them in a `ToastStack`. */
export function Toast({ title, children, accent, onClick, action, onDismiss, timeoutMs, className }: ToastProps) {
  const t = useT();
  const dismiss = useRef(onDismiss);
  useEffect(() => {
    dismiss.current = onDismiss;
  });
  const hovered = useRef(false);
  useEffect(() => {
    if (timeoutMs === undefined) return;
    let left = timeoutMs;
    let last = Date.now();
    const t = setInterval(() => {
      const now = Date.now();
      if (!hovered.current) left -= now - last;
      last = now;
      if (left <= 0) {
        clearInterval(t);
        dismiss.current?.();
      }
    }, 100);
    return () => clearInterval(t);
  }, [timeoutMs]);
  return (
    <div
      className={clsx("eth-toast", onClick && "eth-toast--clickable", className)}
      role="status"
      style={accent ? ({ "--eth-toast-accent": accent } as CSSProperties) : undefined}
      onPointerEnter={() => (hovered.current = true)}
      onPointerLeave={() => (hovered.current = false)}
    >
      <button type="button" className="eth-toast__body" onClick={onClick} disabled={!onClick} tabIndex={onClick ? 0 : -1}>
        <span className="eth-toast__title">{title}</span>
        {children !== undefined && <span className="eth-toast__text">{children}</span>}
      </button>
      {action && (
        <button
          type="button"
          className="eth-toast__action"
          onClick={() => {
            action.onSelect();
            onDismiss?.();
          }}
        >
          {action.label}
        </button>
      )}
      {onDismiss && (
        <button type="button" className="eth-toast__close" aria-label={t("common.dismiss")} title={t("common.dismiss")} onClick={onDismiss}>
          <X aria-hidden />
        </button>
      )}
      {timeoutMs !== undefined && (
        <span className="eth-toast__life" style={{ "--eth-toast-life": `${timeoutMs}ms` } as CSSProperties} aria-hidden data-testid="toast-life" />
      )}
    </div>
  );
}

/** Fixed stack of toasts at the top right of the window (newest last). */
export function ToastStack({ children, label }: { children: ReactNode; label?: string }) {
  const t = useT();
  return (
    <div className="eth-toast-stack" aria-label={label ?? t("kit.toast.stack")} aria-live="polite">
      {children}
    </div>
  );
}
