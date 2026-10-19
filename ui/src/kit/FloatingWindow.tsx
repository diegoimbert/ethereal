/**
 * base-154: a floating, non-modal window above the app (device windows, debug panels).
 *
 * - Moves by its title bar, resizes from every edge and corner (pointer capture, within its
 *   min / max size); with the keyboard, once the window has the focus: Alt+arrows move it,
 *   Alt+Shift+arrows resize it, Escape closes it.
 * - May be dragged past the viewport's left, right and bottom edges, part of its title bar
 *   always on screen (`GRIP`: never above the top edge); clamped again when the viewport
 *   changes (the remembered geometry is kept, so a viewport growing back brings the window
 *   back where it was).
 * - Rises above the other windows when clicked. The topmost one closes on desktop ⌘W
 *   (`closeTopFloatingWindow`).
 * - Its geometry is remembered per `id` (kit `floatingWindowStore`); double-clicking the title
 *   resets its size.
 *
 * Callers mount it where it should show (usually in a portal to `document.body`) and unmount
 * it to close it.
 */
import clsx from "clsx";
import { useEffect, useRef, useState, type CSSProperties, type KeyboardEvent, type MouseEvent, type PointerEvent, type ReactNode } from "react";
import { X } from "lucide-react";
import { size } from "../theme/tokens";
import { setDragCursor } from "./dragCursor";
import { clampRect, resizeRect, RESIZE_EDGES, type Grip, type ResizeEdge, type SizeLimits, type Viewport, type WindowRect } from "./floatingWindowGeometry";

/** What stays on screen of a window dragged past an edge: part of its title bar. */
const GRIP: Grip = { x: parseFloat(size.floatingWindowGrip), y: parseFloat(size.panelHeaderHeight) };
import { setWindowCloser, useFloatingWindows } from "./floatingWindowStore";
import { IconButton } from "./IconButton";
import { useT } from "@/i18n";
import "./FloatingWindow.css";

const px = (v: string) => Number.parseFloat(v);
const DEFAULT_WIDTH = px(size.deviceWindowWidth);
const INSET = px(size.floatGap);
/** Height kept on screen for a window sized by its content (enough to grab it back). */
const KEEP = 4 * px(size.controlLg);
/** Alt+arrow step (move or resize), in px. */
const KEY_STEP = px(size.controlMd);

const CURSORS: Record<ResizeEdge, string> = {
  n: "ns-resize",
  s: "ns-resize",
  e: "ew-resize",
  w: "ew-resize",
  ne: "nesw-resize",
  sw: "nesw-resize",
  nw: "nwse-resize",
  se: "nwse-resize",
};

export interface FloatingWindowProps {
  /** Remembers the geometry (keep it stable: e.g. `debug:<id>`, `device:<id>`). */
  id: string;
  /** The title bar's content. */
  title: ReactNode;
  /** Accessible name of the window. */
  label: string;
  onClose(): void;
  /** Accessible name of the close button (default "Close"). */
  closeLabel?: string;
  /** Extra title bar buttons, before the close button. */
  actions?: ReactNode;
  /** Size before any resize; no `defaultHeight`: the content's height (up to the viewport). */
  defaultWidth?: number;
  defaultHeight?: number;
  minWidth?: number;
  minHeight?: number;
  maxWidth?: number;
  maxHeight?: number;
  /** Where the window first opens (default: centered, near the top). */
  defaultPosition?: (width: number) => { x: number; y: number };
  /** Takes the focus when it opens (default true). */
  autoFocus?: boolean;
  /** Called when it's clicked (after it rose). */
  onRaise?: () => void;
  className?: string;
  bodyClassName?: string;
  children?: ReactNode;
  [data: `data-${string}`]: string | undefined;
}

function viewportNow(): Viewport {
  return { width: window.innerWidth, height: window.innerHeight };
}

function useViewport(): Viewport {
  const [vp, setVp] = useState(viewportNow);
  useEffect(() => {
    const on = () => setVp(viewportNow());
    window.addEventListener("resize", on);
    return () => window.removeEventListener("resize", on);
  }, []);
  return vp;
}

type Gesture =
  { kind: "move"; px: number; py: number; start: WindowRect } | { kind: "resize"; px: number; py: number; start: Required<WindowRect>; edge: ResizeEdge };

export function FloatingWindow({
  id,
  title,
  label,
  onClose,
  closeLabel,
  actions,
  defaultWidth = DEFAULT_WIDTH,
  defaultHeight,
  minWidth = px(size.windowMinWidth),
  minHeight = px(size.windowMinHeight),
  maxWidth = Infinity,
  maxHeight = Infinity,
  defaultPosition,
  autoFocus = true,
  onRaise,
  className,
  bodyClassName,
  children,
  ...data
}: FloatingWindowProps) {
  const t = useT();
  const stored = useFloatingWindows((s) => s.rects[id]);
  const stack = useFloatingWindows((s) => s.order.indexOf(id));
  const top = useFloatingWindows((s) => s.order.at(-1) === id);
  const viewport = useViewport();
  const [live, setLive] = useState<WindowRect | null>(null);
  const gesture = useRef<Gesture | null>(null);
  const ref = useRef<HTMLElement>(null);
  const store = useFloatingWindows.getState;

  const limits: SizeLimits = { minWidth, minHeight, maxWidth, maxHeight };
  const initial = (): WindowRect => {
    const p = defaultPosition?.(defaultWidth) ?? { x: Math.round((viewport.width - defaultWidth) / 2), y: 4 * INSET };
    return { x: p.x, y: p.y };
  };
  const clampToView = (r: WindowRect) => clampRect(r, viewport, limits, defaultWidth, defaultHeight ?? KEEP, GRIP);
  const rect = clampToView(live ?? stored ?? initial());
  const width = Math.min(rect.w ?? defaultWidth, viewport.width);
  const height = rect.h ?? defaultHeight;

  // Opening: on top, with the focus; a window without a remembered place keeps the one it
  // opened at (others opening or a viewport change don't move it).
  useEffect(() => {
    store().raise(id);
    if (!store().rects[id]) store().setRect(id, clampToView(initial()));
    if (autoFocus) ref.current?.focus({ preventScroll: true });
    return () => store().remove(id);
    // On open only.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  // How ⌘W closes it while it's the topmost window (the latest `onClose`).
  const close = useRef(onClose);
  useEffect(() => {
    close.current = onClose;
  });
  useEffect(() => {
    setWindowCloser(id, () => close.current());
    return () => setWindowCloser(id, null);
  }, [id]);

  const commit = (r: WindowRect) => store().setRect(id, r);

  /** The window's rect with its size resolved (what a resize starts from). */
  const sized = (): Required<WindowRect> => {
    const box = ref.current?.getBoundingClientRect();
    return {
      x: rect.x,
      y: rect.y,
      w: rect.w ?? (box?.width || width),
      h: rect.h ?? (box?.height || defaultHeight || minHeight),
    };
  };

  const begin = (e: PointerEvent<HTMLElement>, g: Gesture, cursor: string) => {
    e.preventDefault();
    e.stopPropagation();
    e.currentTarget.setPointerCapture?.(e.pointerId);
    gesture.current = g;
    setDragCursor(cursor);
  };
  const onMove = (e: PointerEvent<HTMLElement>) => {
    const g = gesture.current;
    if (!g) return;
    const dx = e.clientX - g.px;
    const dy = e.clientY - g.py;
    if (g.kind === "move") setLive(clampToView({ ...g.start, x: g.start.x + dx, y: g.start.y + dy }));
    else setLive(resizeRect(g.start, g.edge, dx, dy, viewport, limits, true));
  };
  const end = () => {
    if (!gesture.current) return;
    gesture.current = null;
    setDragCursor(null);
    if (live) commit(live);
    setLive(null);
  };

  const onHeaderDown = (e: PointerEvent<HTMLElement>) => {
    if (e.button !== 0 || (e.target as HTMLElement).closest("button, input, select, textarea")) return;
    begin(e, { kind: "move", px: e.clientX, py: e.clientY, start: rect }, "grabbing");
  };
  const onHandleDown = (edge: ResizeEdge) => (e: PointerEvent<HTMLElement>) => {
    if (e.button !== 0) return;
    begin(e, { kind: "resize", px: e.clientX, py: e.clientY, start: sized(), edge }, CURSORS[edge]);
  };
  const onTitleDoubleClick = (e: MouseEvent<HTMLElement>) => {
    if ((e.target as HTMLElement).closest("button")) return;
    commit(clampToView({ x: rect.x, y: rect.y }));
  };

  const onKeyDown = (e: KeyboardEvent<HTMLElement>) => {
    // Only when the window itself has the focus: controls inside use these keys themselves.
    if (e.target !== e.currentTarget) return;
    if (e.key === "Escape") {
      e.stopPropagation();
      onClose();
      return;
    }
    const dir: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
    const d = dir[e.key];
    if (!d || !e.altKey) return;
    e.preventDefault();
    e.stopPropagation();
    const [dx, dy] = [d[0] * KEY_STEP, d[1] * KEY_STEP];
    if (e.shiftKey) commit(resizeRect(sized(), "se", dx, dy, viewport, limits, true));
    else commit(clampToView({ ...rect, x: rect.x + dx, y: rect.y + dy }));
  };

  return (
    <section
      ref={ref}
      {...data}
      className={clsx("eth-fwin", top && "eth-fwin--top", height !== undefined && "eth-fwin--sized", live && "eth-fwin--gesture", className)}
      role="dialog"
      aria-modal="false"
      aria-label={label}
      data-window={id}
      tabIndex={-1}
      style={
        {
          left: rect.x,
          top: rect.y,
          width,
          height,
          "--fwin-stack": Math.max(0, stack),
        } as CSSProperties
      }
      onPointerDownCapture={() => {
        store().raise(id);
        onRaise?.();
      }}
      onKeyDown={onKeyDown}
    >
      <header
        className="eth-fwin__header"
        onPointerDown={onHeaderDown}
        onPointerMove={onMove}
        onPointerUp={end}
        onPointerCancel={end}
        onDoubleClick={onTitleDoubleClick}
      >
        <span className="eth-fwin__title">{title}</span>
        {actions && <span className="eth-fwin__actions">{actions}</span>}
        <IconButton size="sm" tone="ghost" label={closeLabel ?? t("common.close")} icon={<X />} onClick={onClose} />
      </header>
      <div className={clsx("eth-fwin__body", bodyClassName)}>{children}</div>
      {RESIZE_EDGES.map((edge) => (
        <div
          key={edge}
          className={`eth-fwin__handle eth-fwin__handle--${edge}`}
          data-resize={edge}
          aria-hidden="true"
          onPointerDown={onHandleDown(edge)}
          onPointerMove={onMove}
          onPointerUp={end}
          onPointerCancel={end}
        />
      ))}
    </section>
  );
}
