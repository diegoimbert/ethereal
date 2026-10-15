import clsx from "clsx";
import { PanelBottomOpen, PanelRightOpen, Pin, PinOff, X } from "lucide-react";
import { useEffect, useRef, useState, type PointerEvent, type ReactNode } from "react";
import { useT } from "@/i18n";
import { IconButton, setDragCursor } from "@/kit";
import type { PaneSide } from "@/state/shell";
import { getMotionLevel } from "@/timeline/motion";

export interface FloatingPaneProps {
  side: PaneSide;
  /** Its `data-tour` name (guided tours highlight the whole pane, docs/TUTORIALS.md). */
  tour?: string;
  open: boolean;
  pinned: boolean;
  /** Width (left/right) or height (bottom) in px. */
  size: number;
  minSize: number;
  /** Upper bound at drag start. */
  maxSize: () => number;
  onResize(px: number): void;
  onPinnedChange(pinned: boolean): void;
  /**
   * Right / bottom: the × button collapses the pane into a thin floating bar (`collapsed`),
   * which expands it again; its content is unmounted meanwhile.
   */
  collapsed?: boolean;
  onCollapsedChange?: (collapsed: boolean) => void;
  /** Omit to hide the close button (e.g. the inspector closes with the selection). */
  onClose?: () => void;
  /** Accessible name of the pane (shown text, in the current language). */
  label: string;
  /** Header content (title, tabs); the pin and close buttons are added after it. */
  header?: ReactNode;
  children: ReactNode;
  className?: string;
}

/** Keep this in sync with `--float-exit-duration` (unmount fallback when no animationend). */
const EXIT_MS = 180;

/**
 * Keeps something mounted through its exit animation: `mounted` follows `wanted` on the way in
 * and trails it until the animation ends (`end`, from animationend; a timeout is the fallback).
 * With the animation level at "reduced" it follows `wanted` at once.
 */
function usePresence(wanted: boolean): { mounted: boolean; closing: boolean; end(): void } {
  const [mounted, setMounted] = useState(wanted);
  if (wanted && !mounted) setMounted(true);
  const instant = getMotionLevel() === "reduced";
  if (!wanted && mounted && instant) setMounted(false);
  const closing = mounted && !wanted && !instant;
  useEffect(() => {
    if (!closing) return;
    const t = setTimeout(() => setMounted(false), EXIT_MS);
    return () => clearTimeout(t);
  }, [closing]);
  return {
    mounted: wanted || closing,
    closing,
    end: () => {
      if (closing) setMounted(false);
    },
  };
}

/**
 * A floating card docked to one side of the workspace. It flies in when opened and out
 * when closed, can be resized from its inner edge, and pinned: pinned panes also reserve
 * their space (the workspace pads the main view), but look the same. Position and pinned
 * space are laid out by the parent (`Workspace`) through CSS variables.
 */
export function FloatingPane(p: FloatingPaneProps) {
  const t = useT();
  const collapsible = !!p.onCollapsedChange;
  // The card shows while open and not collapsed; the bar while open and collapsed. Each stays
  // mounted through its exit animation; the bar waits for the card to be gone (collapse: the
  // card flies out, then the bar fades in), while an expanding bar fades out as the card flies in.
  const card = usePresence(p.open && !(collapsible && p.collapsed));
  const bar = usePresence(collapsible && p.open && !!p.collapsed && !card.mounted);
  const barEl = bar.mounted && <CollapsedBar {...p} closing={bar.closing} onAnimationEnd={bar.end} onExpand={() => p.onCollapsedChange!(false)} />;
  if (!card.mounted) return barEl || null;
  const closing = card.closing;

  return (
    <>
      {barEl}
      <section
        className={clsx("eth-float", `eth-float--${p.side}`, p.pinned && "eth-float--pinned", closing && "eth-float--closing", p.className)}
        aria-label={p.label}
        data-pane={p.side}
        data-tour={p.tour}
        onAnimationEnd={(e) => {
          if (e.target === e.currentTarget) card.end();
        }}
      >
        <header className="eth-float__header">
          <div className="eth-float__title">{p.header}</div>
          <IconButton
            size="sm"
            tone="ghost"
            active={p.pinned}
            label={p.pinned ? t("shell.pane.unpin", { pane: p.label }) : t("shell.pane.pin", { pane: p.label })}
            title={p.pinned ? t("shell.pane.unpin_title") : t("shell.pane.pin_title")}
            icon={p.pinned ? <PinOff /> : <Pin />}
            onClick={() => p.onPinnedChange(!p.pinned)}
          />
          {p.onCollapsedChange && (
            <IconButton size="sm" tone="ghost" label={t("shell.pane.collapse", { pane: p.label })} icon={<X />} onClick={() => p.onCollapsedChange!(true)} />
          )}
          {!p.onCollapsedChange && p.onClose && (
            <IconButton size="sm" tone="ghost" label={t("shell.pane.close", { pane: p.label })} icon={<X />} onClick={p.onClose} />
          )}
        </header>
        <div className="eth-float__body">{p.children}</div>
        <ResizeEdge {...p} />
      </section>
    </>
  );
}

/**
 * The collapsed pane: a thin bar floating at its edge (vertical title on the right, horizontal
 * at the bottom) with an expand icon. One button: a click or Enter expands (Space still plays,
 * see transport-bar/keys). `data-pane` keeps presses on it from counting as click-away.
 */
function CollapsedBar({ side, label, onExpand, closing, onAnimationEnd }: FloatingPaneProps & { onExpand(): void; closing: boolean; onAnimationEnd(): void }) {
  const t = useT();
  return (
    <button
      type="button"
      className={clsx("eth-float-bar", `eth-float-bar--${side}`, closing && "eth-float-bar--closing")}
      onAnimationEnd={(e) => e.target === e.currentTarget && onAnimationEnd()}
      disabled={closing}
      data-pane={side}
      data-collapsed="true"
      aria-label={t("shell.pane.expand", { pane: label })}
      title={t("shell.pane.expand", { pane: label })}
      onClick={onExpand}
    >
      {side === "bottom" ? <PanelBottomOpen aria-hidden /> : <PanelRightOpen aria-hidden />}
      <span className="eth-float-bar__title">{label}</span>
    </button>
  );
}

/** Drag the pane's inner edge to resize it. Double-click restores the default size. */
function ResizeEdge({ side, size, minSize, maxSize, onResize, label }: FloatingPaneProps) {
  const t = useT();
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
      aria-label={t("shell.pane.resize", { pane: label })}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={end}
      onPointerCancel={end}
    />
  );
}
