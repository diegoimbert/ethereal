import "./toast.css";
import clsx from "clsx";
import { Check, ChevronDown, Copy, X } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { useT } from "@/i18n";

export interface ToastProps {
  /** First line (e.g. who). */
  title: ReactNode;
  /** Body text, clamped to a few lines; a clamped text expands (chevron, or a click on a toast
   *  without `onClick`) to all of it, with a copy button, and then stays until dismissed. */
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
  // Paused while hovered or focused (reading or about to click), and for good once expanded.
  const hovered = useRef(false);
  const focused = useRef(false);
  const [expanded, setExpanded] = useState(false);
  const held = useRef(false);
  useEffect(() => {
    held.current = expanded;
  }, [expanded]);
  useEffect(() => {
    if (timeoutMs === undefined) return;
    let left = timeoutMs;
    let last = Date.now();
    const t = setInterval(() => {
      const now = Date.now();
      if (!hovered.current && !focused.current && !held.current) left -= now - last;
      last = now;
      if (left <= 0) {
        clearInterval(t);
        dismiss.current?.();
      }
    }, 100);
    return () => clearInterval(t);
  }, [timeoutMs]);
  // Is the text clamped (more lines than shown)? Measured after each render and on resize.
  const textRef = useRef<HTMLSpanElement>(null);
  const [clamped, setClamped] = useState(false);
  useLayoutEffect(() => {
    const el = textRef.current;
    if (!el || expanded) return;
    const measure = () => setClamped(el.scrollHeight > el.clientHeight + 1);
    measure();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  });
  const expandable = clamped || expanded;
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const id = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(id);
  }, [copied]);
  const copy = () => {
    const text = textRef.current?.innerText ?? textRef.current?.textContent ?? "";
    void navigator.clipboard?.writeText(text).then(
      () => setCopied(true),
      () => undefined,
    );
  };
  const bodyClick = onClick ?? (expandable ? () => setExpanded((e) => !e) : undefined);
  return (
    <div
      className={clsx("eth-toast", onClick && "eth-toast--clickable", expandable && "eth-toast--expandable", expanded && "eth-toast--expanded", className)}
      role="status"
      style={accent ? ({ "--eth-toast-accent": accent } as CSSProperties) : undefined}
      data-expanded={expanded || undefined}
      onPointerEnter={() => (hovered.current = true)}
      onPointerLeave={() => (hovered.current = false)}
      onFocus={() => (focused.current = true)}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) focused.current = false;
      }}
    >
      <button
        type="button"
        className="eth-toast__body"
        onClick={bodyClick}
        disabled={!bodyClick}
        tabIndex={bodyClick ? 0 : -1}
        aria-expanded={!onClick && expandable ? expanded : undefined}
      >
        <span className="eth-toast__title">{title}</span>
        {children !== undefined && (
          <span ref={textRef} className="eth-toast__text" data-testid="toast-text">
            {children}
          </span>
        )}
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
      <div className="eth-toast__tools">
        {onDismiss && (
          <button type="button" className="eth-toast__close" aria-label={t("common.dismiss")} title={t("common.dismiss")} onClick={onDismiss}>
            <X aria-hidden />
          </button>
        )}
        {expandable && (
          <button
            type="button"
            className="eth-toast__close eth-toast__expand"
            aria-label={expanded ? t("kit.toast.collapse") : t("kit.toast.expand")}
            title={expanded ? t("kit.toast.collapse") : t("kit.toast.expand")}
            aria-expanded={expanded}
            data-testid="toast-expand"
            onClick={() => setExpanded((e) => !e)}
          >
            <ChevronDown aria-hidden />
          </button>
        )}
        {expanded && (
          <button
            type="button"
            className="eth-toast__close"
            aria-label={copied ? t("kit.toast.copied") : t("kit.toast.copy")}
            title={copied ? t("kit.toast.copied") : t("kit.toast.copy")}
            data-testid="toast-copy"
            onClick={copy}
          >
            {copied ? <Check aria-hidden /> : <Copy aria-hidden />}
          </button>
        )}
      </div>
      {timeoutMs !== undefined && !expanded && (
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
