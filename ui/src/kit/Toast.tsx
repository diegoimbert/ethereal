import "./toast.css";
import clsx from "clsx";
import { X } from "lucide-react";
import { useEffect, useRef, type CSSProperties, type ReactNode } from "react";

export interface ToastProps {
  /** First line (e.g. who). */
  title: ReactNode;
  /** Body text, clamped to a few lines. */
  children?: ReactNode;
  /** Accent (a CSS color, e.g. a peer colour: data, like track colours). Default: the accent token. */
  accent?: string;
  /** Click on the toast body (e.g. open what it is about). */
  onClick?: () => void;
  onDismiss?: () => void;
  /** Auto-dismiss after this many ms (paused while hovered); omit to keep it. */
  timeoutMs?: number;
  className?: string;
}

/** A short notification card. Stack them in a `ToastStack`. */
export function Toast({ title, children, accent, onClick, onDismiss, timeoutMs, className }: ToastProps) {
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
      {onDismiss && (
        <button type="button" className="eth-toast__close" aria-label="Dismiss" title="Dismiss" onClick={onDismiss}>
          <X aria-hidden />
        </button>
      )}
    </div>
  );
}

/** Fixed stack of toasts at the top right of the window (newest last). */
export function ToastStack({ children, label = "Notifications" }: { children: ReactNode; label?: string }) {
  return (
    <div className="eth-toast-stack" aria-label={label} aria-live="polite">
      {children}
    </div>
  );
}
