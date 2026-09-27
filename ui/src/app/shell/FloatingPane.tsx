import clsx from "clsx";
import { Pin, PinOff, X } from "lucide-react";
import { useEffect, useRef, useState, type PointerEvent, type ReactNode } from "react";
import { IconButton, setDragCursor } from "@/kit";
import type { PaneSide } from "./shellStore";

export interface FloatingPaneProps {
  side: PaneSide;
  open: boolean;
  pinned: boolean;
  /** Width (left/right) or height (bottom) in px. */
  size: number;
  minSize: number;
  /** Upper bound at drag start. */
  maxSize: () => number;
  onResize(px: number): void;
  onPinnedChange(pinned: boolean): void;
  /** Omit to hide the close button (e.g. the inspector closes with the selection). */
  onClose?: () => void;
  /** Accessible name of the pane. */
  label: string;
  /** Header content (title, tabs); the pin and close buttons are added after it. */
  header?: ReactNode;
  children: ReactNode;
  className?: string;
}

/** Keep this in sync with `--float-exit-duration` (unmount fallback when no animationend). */
const EXIT_MS = 180;

/**
 * A floating card docked to one side of the workspace. It flies in when opened and out
 * when closed, can be resized from its inner edge, and pinned: pinned panes also reserve
 * their space (the workspace pads the main view), but look the same. Position and pinned
 * space are laid out by the parent (`Workspace`) through CSS variables.
 */
export function FloatingPane(p: FloatingPaneProps) {
  const [mounted, setMounted] = useState(p.open);
  const closing = mounted && !p.open;
  // Mount on open; unmount after the exit animation.
  if (p.open && !mounted) setMounted(true);
  useEffect(() => {
    if (!closing) return;
    const t = setTimeout(() => setMounted(false), EXIT_MS);
    return () => clearTimeout(t);
  }, [closing]);
  if (!mounted) return null;

  return (
    <section
      className={clsx(
        "eth-float",
        `eth-float--${p.side}`,
        p.pinned && "eth-float--pinned",
        closing && "eth-float--closing",
        p.className,
      )}
      aria-label={p.label}
      data-pane={p.side}
      onAnimationEnd={(e) => {
        if (closing && e.target === e.currentTarget) setMounted(false);
      }}
    >
      <header className="eth-float__header">
        <div className="eth-float__title">{p.header}</div>
        <IconButton
          size="sm"
          tone="ghost"
          active={p.pinned}
          label={p.pinned ? `Unpin ${p.label}` : `Pin ${p.label}`}
          title={p.pinned ? "Unpin (float over the arrangement)" : "Pin (keep its space next to the arrangement)"}
          icon={p.pinned ? <PinOff /> : <Pin />}
          onClick={() => p.onPinnedChange(!p.pinned)}
        />
        {p.onClose && <IconButton size="sm" tone="ghost" label={`Close ${p.label}`} icon={<X />} onClick={p.onClose} />}
      </header>
      <div className="eth-float__body">{p.children}</div>
      <ResizeEdge {...p} />
    </section>
  );
}

/** Drag the pane's inner edge to resize it. Double-click restores the default size. */
function ResizeEdge({ side, size, minSize, maxSize, onResize, label }: FloatingPaneProps) {
  const start = useRef<{ pos: number; size: number; max: number } | null>(null);
  const vertical = side === "bottom";
  const dir = side === "left" ? 1 : -1;

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    start.current = { pos: vertical ? e.clientY : e.clientX, size, max: Math.max(minSize, maxSize()) };
    setDragCursor(vertical ? "ns-resize" : "ew-resize");
  };
  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const s = start.current;
    if (!s) return;
    const d = (vertical ? e.clientY : e.clientX) - s.pos;
    onResize(Math.min(s.max, Math.max(minSize, s.size + dir * d)));
  };
  const end = () => {
    start.current = null;
    setDragCursor(null);
  };
  return (
    <div
      className="eth-float__resize"
      role="separator"
      aria-orientation={vertical ? "horizontal" : "vertical"}
      aria-label={`Resize ${label}`}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={end}
      onPointerCancel={end}
    />
  );
}
